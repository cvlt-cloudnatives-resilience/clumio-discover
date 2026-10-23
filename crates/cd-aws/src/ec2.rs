use std::collections::{BTreeSet, HashMap, HashSet};

use aws_sdk_ec2::types::{
    Image, ImageAttributeName, ImageState, Instance, InstanceAttributeName, InstanceStatusSummary,
    InstanceType, IpPermission, LaunchPermission, PermissionGroup, SnapshotState, Tag,
};
use aws_sdk_ec2::Client;
use aws_smithy_types::date_time::Format;
use aws_smithy_types::DateTime;
use futures::future::join_all;

use cd_model::{
    CapacityReservationRef, DiscoveryError, Ec2Instance, ElasticIp, InstanceProfile,
    MetadataOptions, NativeBackup, NativeKind, OwnedAmi, Protection, SecurityGroupRef, SsmAgent,
    Tags, MSSQL_LICENSE_INCLUDED, MSSQL_POSSIBLE,
};

use crate::{failure, timestamp, to_tags, Scope, Section};

const SERVICE: &str = "ec2";
const BATCH: usize = 100;

pub(crate) const ACTIONS: &[&str] = &[
    "ec2:DescribeAddresses",
    "ec2:DescribeCapacityReservations",
    "ec2:DescribeImageAttribute",
    "ec2:DescribeImages",
    "ec2:DescribeInstanceAttribute",
    "ec2:DescribeInstanceCreditSpecifications",
    "ec2:DescribeInstanceStatus",
    "ec2:DescribeInstanceTypes",
    "ec2:DescribeInstances",
    "ec2:DescribeKeyPairs",
    "ec2:DescribeSecurityGroups",
    "ec2:DescribeSnapshots",
    "ec2:DescribeSubnets",
    "ec2:DescribeVpcs",
    "iam:GetInstanceProfile",
    "ssm:DescribeInstanceProperties",
];

pub async fn discover(scope: &Scope<'_>) -> Section<Ec2Instance> {
    let client = Client::new(scope.sdk());
    let raw = match list_instances(scope, &client).await {
        Ok(raw) => raw,
        Err(err) => return Section::failed(err),
    };
    if raw.is_empty() {
        return Section::ok(Vec::new(), Vec::new());
    }

    let mut errors = Vec::new();
    let context = RegionContext::load(scope, &client, &raw, &mut errors).await;
    let mut instances: Vec<Ec2Instance> = raw
        .iter()
        .map(|instance| {
            let id = instance.instance_id().unwrap_or_default();
            let arn = scope.arn(SERVICE, &format!("instance/{id}"));
            map_instance(instance, arn, &scope.region, &context)
        })
        .collect();
    if scope.ctx.deep() {
        errors.extend(deepen(scope, &client, &mut instances).await);
    }
    Section::ok(instances, errors)
}

pub(crate) fn ec2_tags(tags: &[Tag]) -> Tags {
    to_tags(
        tags.iter()
            .map(|t| (t.key().unwrap_or_default(), t.value().unwrap_or_default())),
    )
}

async fn list_instances(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<Instance>, DiscoveryError> {
    let mut pages = client.describe_instances().into_paginator().send();
    let mut instances = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeInstances", None, e))?;
        for instance in page
            .reservations
            .unwrap_or_default()
            .into_iter()
            .flat_map(|r| r.instances.unwrap_or_default())
        {
            let terminated = instance
                .state()
                .and_then(|s| s.name())
                .is_some_and(|n| n.as_str() == "terminated");
            if !terminated && scope.filters().admits(&ec2_tags(instance.tags())) {
                instances.push(instance);
            }
        }
    }
    Ok(instances)
}

struct SnapshotRef {
    id: String,
    volume_id: Option<String>,
    started: Option<String>,
    completed: bool,
}

#[derive(Debug, Default, PartialEq)]
struct SnapshotStats {
    count: u64,
    latest: Option<String>,
}

#[derive(Default)]
struct RegionContext {
    statuses: HashMap<String, (Option<String>, Option<String>)>,
    types: HashMap<String, (Option<i32>, Option<i64>)>,
    vpc_names: HashMap<String, String>,
    subnet_names: HashMap<String, String>,
    open_to_world: HashMap<String, Vec<String>>,
    addresses: HashMap<String, ElasticIp>,
    key_names: Option<HashSet<String>>,
    reservations: HashMap<String, CapacityReservationRef>,
    credits: HashMap<String, String>,
    ssm: Option<HashMap<String, SsmAgent>>,
    profiles: HashMap<String, Vec<String>>,
    amis: HashMap<String, Vec<OwnedAmi>>,
    snapshots: HashMap<String, SnapshotStats>,
}

fn keep<T: Default>(result: Result<T, DiscoveryError>, errors: &mut Vec<DiscoveryError>) -> T {
    result.unwrap_or_else(|err| {
        errors.push(err);
        T::default()
    })
}

fn keep_known<T>(result: Result<T, DiscoveryError>, errors: &mut Vec<DiscoveryError>) -> Option<T> {
    result.map_err(|err| errors.push(err)).ok()
}

fn split<T>(results: Vec<Result<T, DiscoveryError>>) -> (Vec<T>, Vec<DiscoveryError>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(value) => found.push(value),
            Err(err) => errors.push(err),
        }
    }
    (found, errors)
}

fn burstable(instance_type: &str) -> bool {
    instance_type
        .strip_prefix('t')
        .and_then(|rest| rest.chars().next())
        .is_some_and(|generation| generation.is_ascii_digit() && generation != '1')
}

impl RegionContext {
    async fn load(
        scope: &Scope<'_>,
        client: &Client,
        instances: &[Instance],
        errors: &mut Vec<DiscoveryError>,
    ) -> Self {
        let types: BTreeSet<String> = instances
            .iter()
            .filter_map(|i| i.instance_type())
            .map(|t| t.as_str().to_string())
            .collect();
        let burstable: Vec<String> = instances
            .iter()
            .filter(|i| i.instance_type().is_some_and(|t| burstable(t.as_str())))
            .filter_map(|i| i.instance_id())
            .map(str::to_string)
            .collect();
        let profiles: BTreeSet<String> = instances
            .iter()
            .filter_map(|i| i.iam_instance_profile().and_then(|p| p.arn()))
            .map(str::to_string)
            .collect();
        let has_keys = instances.iter().any(|i| i.key_name().is_some());
        let has_reservations = instances.iter().any(|i| reservation_id(i).is_some());

        let (
            statuses,
            type_info,
            images,
            snapshots,
            vpcs,
            subnets,
            groups,
            addresses,
            keys,
            reservations,
            credits,
            ssm,
            profiles,
        ) = tokio::join!(
            instance_statuses(scope, client),
            instance_types(scope, client, &types),
            owned_images(scope, client),
            owned_snapshots(scope, client),
            vpc_names(scope, client),
            subnet_names(scope, client),
            security_groups(scope, client),
            elastic_ips(scope, client),
            async {
                if has_keys {
                    key_names(scope, client).await.map(Some)
                } else {
                    Ok(None)
                }
            },
            async {
                if has_reservations {
                    capacity_reservations(scope, client).await
                } else {
                    Ok(HashMap::new())
                }
            },
            credit_specifications(scope, client, &burstable),
            ssm_agents(scope),
            instance_profiles(scope, &profiles),
        );

        let (types, type_errors) = type_info;
        let (credits, credit_errors) = credits;
        let (profiles, profile_errors) = profiles;
        errors.extend(
            type_errors
                .into_iter()
                .chain(credit_errors)
                .chain(profile_errors),
        );
        let images = keep(images, errors);
        let snapshots = keep(snapshots, errors);
        let volume_owner: HashMap<&str, &str> = instances
            .iter()
            .flat_map(|i| {
                let id = i.instance_id().unwrap_or_default();
                instance_volumes(i).into_iter().map(move |v| (v, id))
            })
            .collect();

        Self {
            statuses: keep(statuses, errors),
            types,
            vpc_names: keep(vpcs, errors),
            subnet_names: keep(subnets, errors),
            open_to_world: keep(groups, errors),
            addresses: keep(addresses, errors),
            key_names: keep_known(keys, errors).flatten(),
            reservations: keep(reservations, errors),
            credits,
            ssm: keep_known(ssm, errors),
            profiles,
            amis: attribute_amis(&images, &snapshots, &volume_owner),
            snapshots: snapshot_stats(&snapshots, &ami_snapshot_ids(&images)),
        }
    }
}

fn reservation_id(instance: &Instance) -> Option<&str> {
    instance.capacity_reservation_id().or_else(|| {
        instance
            .capacity_reservation_specification()
            .and_then(|s| s.capacity_reservation_target())
            .and_then(|t| t.capacity_reservation_id())
    })
}

fn instance_volumes(instance: &Instance) -> Vec<&str> {
    instance
        .block_device_mappings()
        .iter()
        .filter_map(|m| m.ebs().and_then(|e| e.volume_id()))
        .collect()
}

async fn instance_statuses(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, (Option<String>, Option<String>)>, DiscoveryError> {
    let mut pages = client
        .describe_instance_status()
        .include_all_instances(true)
        .into_paginator()
        .send();
    let summary = |s: Option<&InstanceStatusSummary>| {
        s.and_then(|s| s.status()).map(|s| s.as_str().to_string())
    };
    let mut statuses = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeInstanceStatus", None, e))?;
        for status in page.instance_statuses() {
            statuses.insert(
                status.instance_id().unwrap_or_default().to_string(),
                (
                    summary(status.system_status()),
                    summary(status.instance_status()),
                ),
            );
        }
    }
    Ok(statuses)
}

async fn instance_types(
    scope: &Scope<'_>,
    client: &Client,
    types: &BTreeSet<String>,
) -> (
    HashMap<String, (Option<i32>, Option<i64>)>,
    Vec<DiscoveryError>,
) {
    let types: Vec<&String> = types.iter().collect();
    let batches = join_all(types.chunks(BATCH).map(|batch| async move {
        let mut pages = client
            .describe_instance_types()
            .set_instance_types(Some(
                batch
                    .iter()
                    .map(|t| InstanceType::from(t.as_str()))
                    .collect(),
            ))
            .into_paginator()
            .send();
        let mut info = HashMap::new();
        while let Some(page) = scope.call(pages.next()).await {
            let page = page.map_err(|e| failure(SERVICE, "DescribeInstanceTypes", None, e))?;
            for t in page.instance_types() {
                if let Some(name) = t.instance_type() {
                    info.insert(
                        name.as_str().to_string(),
                        (
                            t.v_cpu_info().and_then(|v| v.default_v_cpus()),
                            t.memory_info().and_then(|m| m.size_in_mib()),
                        ),
                    );
                }
            }
        }
        Ok::<_, DiscoveryError>(info)
    }))
    .await;
    let (batches, errors) = split(batches);
    (batches.into_iter().flatten().collect(), errors)
}

async fn owned_images(scope: &Scope<'_>, client: &Client) -> Result<Vec<Image>, DiscoveryError> {
    let mut pages = client
        .describe_images()
        .owners("self")
        .include_disabled(true)
        .into_paginator()
        .page_size(1000)
        .send();
    let mut images = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeImages", None, e))?;
        images.extend(page.images.unwrap_or_default());
    }
    Ok(images)
}

async fn owned_snapshots(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<SnapshotRef>, DiscoveryError> {
    let mut pages = client
        .describe_snapshots()
        .owner_ids("self")
        .into_paginator()
        .page_size(1000)
        .send();
    let mut snapshots = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeSnapshots", None, e))?;
        snapshots.extend(page.snapshots().iter().map(|s| SnapshotRef {
            id: s.snapshot_id().unwrap_or_default().to_string(),
            volume_id: s.volume_id().map(str::to_string),
            started: s.start_time().and_then(timestamp),
            completed: s.state() == Some(&SnapshotState::Completed),
        }));
    }
    Ok(snapshots)
}

fn name_tag(tags: &[Tag]) -> Option<String> {
    tags.iter()
        .find(|t| t.key() == Some("Name"))
        .and_then(|t| t.value())
        .map(str::to_string)
}

async fn vpc_names(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, String>, DiscoveryError> {
    let mut pages = client.describe_vpcs().into_paginator().send();
    let mut names = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeVpcs", None, e))?;
        for vpc in page.vpcs() {
            if let (Some(id), Some(name)) = (vpc.vpc_id(), name_tag(vpc.tags())) {
                names.insert(id.to_string(), name);
            }
        }
    }
    Ok(names)
}

async fn subnet_names(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, String>, DiscoveryError> {
    let mut pages = client.describe_subnets().into_paginator().send();
    let mut names = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeSubnets", None, e))?;
        for subnet in page.subnets() {
            if let (Some(id), Some(name)) = (subnet.subnet_id(), name_tag(subnet.tags())) {
                names.insert(id.to_string(), name);
            }
        }
    }
    Ok(names)
}

async fn security_groups(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, Vec<String>>, DiscoveryError> {
    let mut pages = client.describe_security_groups().into_paginator().send();
    let mut groups = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeSecurityGroups", None, e))?;
        for group in page.security_groups() {
            let open = open_to_world(group.ip_permissions());
            if let Some(id) = group.group_id().filter(|_| !open.is_empty()) {
                groups.insert(id.to_string(), open);
            }
        }
    }
    Ok(groups)
}

fn open_to_world(permissions: &[IpPermission]) -> Vec<String> {
    let mut open: Vec<String> = permissions
        .iter()
        .filter(|p| {
            p.ip_ranges()
                .iter()
                .any(|r| r.cidr_ip() == Some("0.0.0.0/0"))
                || p.ipv6_ranges()
                    .iter()
                    .any(|r| r.cidr_ipv6() == Some("::/0"))
        })
        .map(port_spec)
        .collect();
    open.sort();
    open.dedup();
    open
}

fn port_spec(permission: &IpPermission) -> String {
    let protocol = permission.ip_protocol().unwrap_or("-1");
    if protocol == "-1" {
        return "all".to_string();
    }
    match (permission.from_port(), permission.to_port()) {
        (Some(from), Some(to)) if matches!(protocol, "tcp" | "udp") && from == to => {
            format!("{protocol}/{from}")
        }
        (Some(from), Some(to)) if matches!(protocol, "tcp" | "udp") => {
            format!("{protocol}/{from}-{to}")
        }
        _ => protocol.to_string(),
    }
}

async fn elastic_ips(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, ElasticIp>, DiscoveryError> {
    let output = scope
        .call(client.describe_addresses().send())
        .await
        .map_err(|e| failure(SERVICE, "DescribeAddresses", None, e))?;
    Ok(output
        .addresses()
        .iter()
        .filter_map(|a| {
            Some((
                a.instance_id()?.to_string(),
                ElasticIp {
                    public_ip: a.public_ip()?.to_string(),
                    allocation_id: a.allocation_id().map(str::to_string),
                },
            ))
        })
        .collect())
}

async fn key_names(scope: &Scope<'_>, client: &Client) -> Result<HashSet<String>, DiscoveryError> {
    let output = scope
        .call(client.describe_key_pairs().send())
        .await
        .map_err(|e| failure(SERVICE, "DescribeKeyPairs", None, e))?;
    Ok(output
        .key_pairs()
        .iter()
        .filter_map(|k| k.key_name())
        .map(str::to_string)
        .collect())
}

async fn capacity_reservations(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, CapacityReservationRef>, DiscoveryError> {
    let mut pages = client
        .describe_capacity_reservations()
        .into_paginator()
        .send();
    let mut reservations = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeCapacityReservations", None, e))?;
        for r in page.capacity_reservations() {
            if let Some(id) = r.capacity_reservation_id() {
                reservations.insert(
                    id.to_string(),
                    CapacityReservationRef {
                        id: id.to_string(),
                        state: r.state().map(|s| s.as_str().to_string()),
                        instance_type: r.instance_type().map(str::to_string),
                    },
                );
            }
        }
    }
    Ok(reservations)
}

async fn credit_specifications(
    scope: &Scope<'_>,
    client: &Client,
    instance_ids: &[String],
) -> (HashMap<String, String>, Vec<DiscoveryError>) {
    let batches = join_all(instance_ids.chunks(BATCH).map(|batch| async move {
        let mut pages = client
            .describe_instance_credit_specifications()
            .set_instance_ids(Some(batch.to_vec()))
            .into_paginator()
            .send();
        let mut credits = HashMap::new();
        while let Some(page) = scope.call(pages.next()).await {
            let page = page
                .map_err(|e| failure(SERVICE, "DescribeInstanceCreditSpecifications", None, e))?;
            for spec in page.instance_credit_specifications() {
                if let (Some(id), Some(credits_mode)) = (spec.instance_id(), spec.cpu_credits()) {
                    credits.insert(id.to_string(), credits_mode.to_string());
                }
            }
        }
        Ok::<_, DiscoveryError>(credits)
    }))
    .await;
    let (batches, errors) = split(batches);
    (batches.into_iter().flatten().collect(), errors)
}

async fn ssm_agents(scope: &Scope<'_>) -> Result<HashMap<String, SsmAgent>, DiscoveryError> {
    let client = aws_sdk_ssm::Client::new(scope.sdk());
    let mut pages = client
        .describe_instance_properties()
        .into_paginator()
        .send();
    let mut agents = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure("ssm", "DescribeInstanceProperties", None, e))?;
        for p in page.instance_properties() {
            if let Some(id) = p.instance_id() {
                agents.insert(
                    id.to_string(),
                    SsmAgent {
                        ping_status: p.ping_status().map(|s| s.as_str().to_string()),
                        last_ping: p.last_ping_date_time().and_then(timestamp),
                        platform_name: p.platform_name().map(str::to_string),
                        platform_version: p.platform_version().map(str::to_string),
                        agent_version: p.agent_version().map(str::to_string),
                    },
                );
            }
        }
    }
    Ok(agents)
}

async fn instance_profiles(
    scope: &Scope<'_>,
    arns: &BTreeSet<String>,
) -> (HashMap<String, Vec<String>>, Vec<DiscoveryError>) {
    let client = &aws_sdk_iam::Client::new(scope.ctx.home());
    let results = join_all(arns.iter().map(|arn| async move {
        let name = arn.rsplit('/').next().unwrap_or(arn);
        let output = scope
            .call(
                client
                    .get_instance_profile()
                    .instance_profile_name(name)
                    .send(),
            )
            .await;
        let roles = match output {
            Ok(output) => output
                .instance_profile()
                .map(|p| {
                    p.roles()
                        .iter()
                        .map(|r| r.role_name().to_string())
                        .collect()
                })
                .unwrap_or_default(),
            Err(err)
                if err.as_service_error().and_then(|e| e.meta().code()) == Some("NoSuchEntity") =>
            {
                Vec::new()
            }
            Err(err) => {
                return Err(failure(
                    "iam",
                    "GetInstanceProfile",
                    Some(arn.as_str()),
                    err,
                ))
            }
        };
        Ok((arn.clone(), roles))
    }))
    .await;
    let (found, errors) = split(results);
    (found.into_iter().collect(), errors)
}

fn rfc3339(raw: &str) -> String {
    DateTime::from_str(raw, Format::DateTime)
        .ok()
        .and_then(|t| timestamp(&t))
        .unwrap_or_else(|| raw.to_string())
}

fn ami_snapshot_ids(images: &[Image]) -> HashSet<&str> {
    images
        .iter()
        .flat_map(|i| i.block_device_mappings())
        .filter_map(|m| m.ebs().and_then(|e| e.snapshot_id()))
        .collect()
}

fn attribute_amis(
    images: &[Image],
    snapshots: &[SnapshotRef],
    volume_owner: &HashMap<&str, &str>,
) -> HashMap<String, Vec<OwnedAmi>> {
    let snapshot_volume: HashMap<&str, &str> = snapshots
        .iter()
        .filter_map(|s| Some((s.id.as_str(), s.volume_id.as_deref()?)))
        .collect();
    let mut amis: HashMap<String, Vec<OwnedAmi>> = HashMap::new();
    for image in images {
        let owner = image.source_instance_id().or_else(|| {
            image
                .block_device_mappings()
                .iter()
                .filter_map(|m| m.ebs().and_then(|e| e.snapshot_id()))
                .filter_map(|snapshot| snapshot_volume.get(snapshot))
                .find_map(|volume| volume_owner.get(volume).copied())
        });
        let Some(owner) = owner else { continue };
        amis.entry(owner.to_string()).or_default().push(OwnedAmi {
            id: image.image_id().unwrap_or_default().to_string(),
            name: image.name().map(str::to_string),
            state: image.state().map(|s| s.as_str().to_string()),
            created: image.creation_date().map(rfc3339),
            public: image.public(),
            shared_with: Vec::new(),
            tags: ec2_tags(image.tags()),
        });
    }
    amis
}

fn snapshot_stats(
    snapshots: &[SnapshotRef],
    ami_snapshots: &HashSet<&str>,
) -> HashMap<String, SnapshotStats> {
    let mut stats: HashMap<String, SnapshotStats> = HashMap::new();
    for snapshot in snapshots {
        let Some(volume) = &snapshot.volume_id else {
            continue;
        };
        if !snapshot.completed || ami_snapshots.contains(snapshot.id.as_str()) {
            continue;
        }
        let entry = stats.entry(volume.clone()).or_default();
        entry.count += 1;
        entry.latest = entry.latest.take().max(snapshot.started.clone());
    }
    stats
}

fn native_backups(
    volumes: &[String],
    amis: &[OwnedAmi],
    snapshots: &HashMap<String, SnapshotStats>,
) -> Vec<NativeBackup> {
    let mut native = Vec::new();
    let available: Vec<&OwnedAmi> = amis
        .iter()
        .filter(|a| a.state.as_deref() == Some(ImageState::Available.as_str()))
        .collect();
    if let Some(latest) = available.iter().max_by(|a, b| a.created.cmp(&b.created)) {
        native.push(NativeBackup {
            kind: NativeKind::Ami,
            count: Some(available.len() as u64),
            latest: latest.created.clone(),
            detail: Some(latest.id.clone()),
        });
    }
    let covered: Vec<&SnapshotStats> = volumes.iter().filter_map(|v| snapshots.get(v)).collect();
    if !covered.is_empty() {
        native.push(NativeBackup {
            kind: NativeKind::EbsSnapshot,
            count: Some(covered.iter().map(|s| s.count).sum()),
            latest: covered.iter().filter_map(|s| s.latest.clone()).max(),
            detail: Some(format!("{} of {} volumes", covered.len(), volumes.len())),
        });
    }
    native
}

fn mssql_candidate(
    platform: Option<&str>,
    platform_details: Option<&str>,
    ssm_managed: Option<bool>,
) -> Option<String> {
    let details = platform_details.unwrap_or_default();
    if details.contains("SQL Server") {
        return Some(MSSQL_LICENSE_INCLUDED.to_string());
    }
    let windows = platform.is_some_and(|p| p.eq_ignore_ascii_case("windows"))
        || details.starts_with("Windows");
    (windows && ssm_managed == Some(true)).then(|| MSSQL_POSSIBLE.to_string())
}

fn map_instance(raw: &Instance, arn: String, region: &str, cx: &RegionContext) -> Ec2Instance {
    let id = raw.instance_id().unwrap_or_default().to_string();
    let instance_type = raw.instance_type().map(|t| t.as_str().to_string());
    let (vcpus, memory_mib) = instance_type
        .as_ref()
        .and_then(|t| cx.types.get(t))
        .copied()
        .unwrap_or_default();
    let (system_status, instance_status) = cx.statuses.get(&id).cloned().unwrap_or_default();
    let volumes: Vec<String> = instance_volumes(raw)
        .into_iter()
        .map(str::to_string)
        .collect();
    let platform = raw.platform().map(|p| p.as_str().to_string());
    let platform_details = raw.platform_details().map(str::to_string);
    let ssm_managed = cx.ssm.as_ref().map(|agents| agents.contains_key(&id));
    let key_name = raw.key_name().map(str::to_string);
    let vpc_id = raw.vpc_id().map(str::to_string);
    let subnet_id = raw.subnet_id().map(str::to_string);
    let amis = cx.amis.get(&id).cloned().unwrap_or_default();

    Ec2Instance {
        arn,
        region: region.to_string(),
        state: raw
            .state()
            .and_then(|s| s.name())
            .map(|n| n.as_str().to_string()),
        launch_time: raw.launch_time().and_then(timestamp),
        image_id: raw.image_id().map(str::to_string),
        mssql: mssql_candidate(
            platform.as_deref(),
            platform_details.as_deref(),
            ssm_managed,
        ),
        platform,
        platform_details,
        architecture: raw.architecture().map(|a| a.as_str().to_string()),
        vcpus,
        memory_mib,
        availability_zone: raw
            .placement()
            .and_then(|p| p.availability_zone())
            .map(str::to_string),
        vpc_name: vpc_id.as_ref().and_then(|v| cx.vpc_names.get(v)).cloned(),
        vpc_id,
        subnet_name: subnet_id
            .as_ref()
            .and_then(|s| cx.subnet_names.get(s))
            .cloned(),
        subnet_id,
        private_ip: raw.private_ip_address().map(str::to_string),
        public_ip: raw.public_ip_address().map(str::to_string),
        elastic_ip: cx.addresses.get(&id).cloned(),
        eni_count: raw.network_interfaces().len() as u32,
        security_groups: raw
            .security_groups()
            .iter()
            .filter_map(|g| {
                let id = g.group_id()?;
                Some(SecurityGroupRef {
                    id: id.to_string(),
                    name: g.group_name().map(str::to_string),
                    open_to_world: cx.open_to_world.get(id).cloned().unwrap_or_default(),
                })
            })
            .collect(),
        key_pair_exists: key_name
            .as_ref()
            .and_then(|k| cx.key_names.as_ref().map(|names| names.contains(k))),
        key_name,
        iam_instance_profile: raw.iam_instance_profile().and_then(|p| p.arn()).map(|arn| {
            InstanceProfile {
                arn: arn.to_string(),
                roles: cx.profiles.get(arn).cloned().unwrap_or_default(),
            }
        }),
        root_device_type: raw.root_device_type().map(|t| t.as_str().to_string()),
        ebs_optimized: raw.ebs_optimized(),
        monitoring: raw
            .monitoring()
            .and_then(|m| m.state())
            .map(|s| s.as_str().to_string()),
        tenancy: raw
            .placement()
            .and_then(|p| p.tenancy())
            .map(|t| t.as_str().to_string()),
        hibernation: raw.hibernation_options().and_then(|h| h.configured()),
        metadata_options: raw.metadata_options().map(|m| MetadataOptions {
            http_tokens: m.http_tokens().map(|t| t.as_str().to_string()),
            http_endpoint: m.http_endpoint().map(|e| e.as_str().to_string()),
            hop_limit: m.http_put_response_hop_limit(),
        }),
        lifecycle: Some(
            raw.instance_lifecycle()
                .map(|l| l.as_str())
                .unwrap_or("on-demand")
                .to_string(),
        ),
        capacity_reservation: reservation_id(raw).map(|rid| {
            cx.reservations
                .get(rid)
                .cloned()
                .unwrap_or_else(|| CapacityReservationRef {
                    id: rid.to_string(),
                    state: None,
                    instance_type: None,
                })
        }),
        cpu_credits: cx.credits.get(&id).cloned(),
        system_status,
        instance_status,
        ssm: cx.ssm.as_ref().and_then(|agents| agents.get(&id)).cloned(),
        ssm_managed,
        protection: Protection {
            native: native_backups(&volumes, &amis, &cx.snapshots),
            ..Protection::default()
        },
        amis,
        volumes,
        instance_type,
        tags: ec2_tags(raw.tags()),
        id,
        ..Ec2Instance::default()
    }
}

async fn deepen(
    scope: &Scope<'_>,
    client: &Client,
    instances: &mut [Ec2Instance],
) -> Vec<DiscoveryError> {
    let instance_ids: Vec<String> = instances.iter().map(|i| i.id.clone()).collect();
    let ami_ids: Vec<String> = instances
        .iter()
        .flat_map(|i| i.amis.iter().map(|a| a.id.clone()))
        .collect();
    let (attributes, permissions) = tokio::join!(
        join_all(
            instance_ids
                .iter()
                .map(|id| instance_attributes(scope, client, id))
        ),
        join_all(
            ami_ids
                .iter()
                .map(|id| launch_permissions(scope, client, id))
        ),
    );

    let mut errors = Vec::new();
    for (instance, (termination, shutdown)) in instances.iter_mut().zip(attributes) {
        instance.termination_protection = keep_known(termination, &mut errors).flatten();
        instance.shutdown_behavior = keep_known(shutdown, &mut errors).flatten();
    }
    let mut sharing: HashMap<String, (bool, Vec<String>)> = HashMap::new();
    for (id, result) in ami_ids.into_iter().zip(permissions) {
        if let Some(found) = keep_known(result, &mut errors) {
            sharing.insert(id, found);
        }
    }
    for ami in instances.iter_mut().flat_map(|i| i.amis.iter_mut()) {
        if let Some((public, shared_with)) = sharing.get(&ami.id) {
            ami.public = Some(ami.public == Some(true) || *public);
            ami.shared_with.clone_from(shared_with);
        }
    }
    errors
}

type Attribute<T> = Result<Option<T>, DiscoveryError>;

async fn instance_attributes(
    scope: &Scope<'_>,
    client: &Client,
    id: &str,
) -> (Attribute<bool>, Attribute<String>) {
    let request = |attribute: InstanceAttributeName| {
        client
            .describe_instance_attribute()
            .instance_id(id)
            .attribute(attribute)
            .send()
    };
    let (termination, shutdown) = tokio::join!(
        scope.call(request(InstanceAttributeName::DisableApiTermination)),
        scope.call(request(
            InstanceAttributeName::InstanceInitiatedShutdownBehavior
        )),
    );
    (
        termination
            .map(|o| o.disable_api_termination().and_then(|v| v.value()))
            .map_err(|e| failure(SERVICE, "DescribeInstanceAttribute", Some(id), e)),
        shutdown
            .map(|o| {
                o.instance_initiated_shutdown_behavior()
                    .and_then(|v| v.value())
                    .map(str::to_string)
            })
            .map_err(|e| failure(SERVICE, "DescribeInstanceAttribute", Some(id), e)),
    )
}

async fn launch_permissions(
    scope: &Scope<'_>,
    client: &Client,
    id: &str,
) -> Result<(bool, Vec<String>), DiscoveryError> {
    let output = scope
        .call(
            client
                .describe_image_attribute()
                .image_id(id)
                .attribute(ImageAttributeName::LaunchPermission)
                .send(),
        )
        .await
        .map_err(|e| failure(SERVICE, "DescribeImageAttribute", Some(id), e))?;
    Ok(sharing(output.launch_permissions()))
}

fn sharing(permissions: &[LaunchPermission]) -> (bool, Vec<String>) {
    let public = permissions
        .iter()
        .any(|p| p.group() == Some(&PermissionGroup::All));
    let shared_with = permissions
        .iter()
        .filter_map(|p| {
            p.user_id()
                .or(p.organization_arn())
                .or(p.organizational_unit_arn())
        })
        .map(str::to_string)
        .collect();
    (public, shared_with)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_ec2::types::{
        BlockDeviceMapping, EbsBlockDevice, EbsInstanceBlockDevice, GroupIdentifier,
        HttpTokensState, IamInstanceProfile, InstanceBlockDeviceMapping, InstanceLifecycleType,
        InstanceMetadataOptionsResponse, InstanceNetworkInterface, InstanceState,
        InstanceStateName, IpRange, Ipv6Range, Placement, PlatformValues, Tenancy,
    };

    fn snapshot(id: &str, volume: &str, started: &str, completed: bool) -> SnapshotRef {
        SnapshotRef {
            id: id.into(),
            volume_id: Some(volume.into()),
            started: Some(started.into()),
            completed,
        }
    }

    fn image(id: &str, source: Option<&str>, snapshots: &[&str], created: &str) -> Image {
        let mut builder = Image::builder()
            .image_id(id)
            .tags(Tag::builder().key("keep").value("yes").build())
            .state(ImageState::Available)
            .creation_date(created)
            .public(false)
            .set_source_instance_id(source.map(str::to_string));
        for snapshot in snapshots {
            builder = builder.block_device_mappings(
                BlockDeviceMapping::builder()
                    .ebs(EbsBlockDevice::builder().snapshot_id(*snapshot).build())
                    .build(),
            );
        }
        builder.build()
    }

    fn raw_instance() -> Instance {
        let volume = |id: &str| {
            InstanceBlockDeviceMapping::builder()
                .ebs(EbsInstanceBlockDevice::builder().volume_id(id).build())
                .build()
        };
        Instance::builder()
            .instance_id("i-1")
            .instance_type(InstanceType::T3Micro)
            .state(
                InstanceState::builder()
                    .name(InstanceStateName::Running)
                    .build(),
            )
            .launch_time(DateTime::from_secs(1_792_713_600))
            .image_id("ami-base")
            .platform(PlatformValues::Windows)
            .platform_details("Windows")
            .placement(
                Placement::builder()
                    .availability_zone("ap-southeast-2a")
                    .tenancy(Tenancy::Default)
                    .build(),
            )
            .vpc_id("vpc-1")
            .subnet_id("subnet-1")
            .private_ip_address("10.0.0.5")
            .public_ip_address("203.0.113.9")
            .security_groups(
                GroupIdentifier::builder()
                    .group_id("sg-1")
                    .group_name("web")
                    .build(),
            )
            .key_name("old-key")
            .iam_instance_profile(
                IamInstanceProfile::builder()
                    .arn("arn:aws:iam::1:instance-profile/app/web-profile")
                    .build(),
            )
            .metadata_options(
                InstanceMetadataOptionsResponse::builder()
                    .http_tokens(HttpTokensState::Optional)
                    .build(),
            )
            .instance_lifecycle(InstanceLifecycleType::Spot)
            .network_interfaces(InstanceNetworkInterface::builder().build())
            .network_interfaces(InstanceNetworkInterface::builder().build())
            .block_device_mappings(volume("vol-root"))
            .block_device_mappings(volume("vol-data"))
            .tags(Tag::builder().key("Name").value("web-1").build())
            .build()
    }

    #[test]
    fn maps_instance_and_region_context() {
        let mut cx = RegionContext::default();
        cx.types.insert("t3.micro".into(), (Some(2), Some(1024)));
        cx.statuses
            .insert("i-1".into(), (Some("ok".into()), Some("impaired".into())));
        cx.vpc_names.insert("vpc-1".into(), "main".into());
        cx.subnet_names.insert("subnet-1".into(), "public-a".into());
        cx.open_to_world
            .insert("sg-1".into(), vec!["tcp/3389".into()]);
        cx.key_names = Some(HashSet::from(["other-key".to_string()]));
        cx.credits.insert("i-1".into(), "unlimited".into());
        cx.ssm = Some(HashMap::from([(
            "i-1".to_string(),
            SsmAgent {
                ping_status: Some("Online".into()),
                ..SsmAgent::default()
            },
        )]));
        cx.profiles.insert(
            "arn:aws:iam::1:instance-profile/app/web-profile".into(),
            vec!["web-role".into()],
        );

        let i = map_instance(&raw_instance(), "arn:i-1".into(), "ap-southeast-2", &cx);

        assert_eq!(i.id, "i-1");
        assert_eq!(i.state.as_deref(), Some("running"));
        assert_eq!(i.instance_type.as_deref(), Some("t3.micro"));
        assert_eq!(i.launch_time.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!((i.vcpus, i.memory_mib), (Some(2), Some(1024)));
        assert_eq!(i.system_status.as_deref(), Some("ok"));
        assert_eq!(i.instance_status.as_deref(), Some("impaired"));
        assert_eq!(i.vpc_name.as_deref(), Some("main"));
        assert_eq!(i.subnet_name.as_deref(), Some("public-a"));
        assert_eq!(i.availability_zone.as_deref(), Some("ap-southeast-2a"));
        assert_eq!(i.tenancy.as_deref(), Some("default"));
        assert_eq!(i.eni_count, 2);
        assert_eq!(i.volumes, vec!["vol-root", "vol-data"]);
        assert_eq!(i.security_groups[0].name.as_deref(), Some("web"));
        assert_eq!(i.security_groups[0].open_to_world, vec!["tcp/3389"]);
        assert_eq!(i.key_pair_exists, Some(false));
        assert_eq!(i.iam_instance_profile.unwrap().roles, vec!["web-role"]);
        assert_eq!(i.lifecycle.as_deref(), Some("spot"));
        assert_eq!(i.cpu_credits.as_deref(), Some("unlimited"));
        assert_eq!(i.ssm_managed, Some(true));
        assert_eq!(i.mssql.as_deref(), Some(MSSQL_POSSIBLE));
        assert!(i.metadata_options.unwrap().imdsv1_allowed());
        assert_eq!(i.tags.get("Name").map(String::as_str), Some("web-1"));
        assert!(i.protection.native.is_empty());
        assert!(i.capacity_reservation.is_none());
    }

    #[test]
    fn failed_lookups_leave_flags_unknown() {
        let i = map_instance(
            &raw_instance(),
            "arn".into(),
            "ap-southeast-2",
            &RegionContext::default(),
        );
        assert_eq!(i.key_pair_exists, None);
        assert_eq!(i.ssm_managed, None);
        assert_eq!(i.mssql, None);
        assert_eq!(i.vcpus, None);
        assert!(i.iam_instance_profile.unwrap().roles.is_empty());
    }

    #[test]
    fn amis_attribute_by_source_instance_then_by_snapshot_volume() {
        let images = vec![
            image(
                "ami-a",
                Some("i-1"),
                &["snap-a"],
                "2026-10-01T00:00:00.000Z",
            ),
            image("ami-b", None, &["snap-b"], "2026-10-05T00:00:00.000Z"),
            image("ami-c", None, &["snap-unknown"], "2026-10-06T00:00:00.000Z"),
            image(
                "ami-d",
                Some("i-gone"),
                &["snap-d"],
                "2026-10-07T00:00:00.000Z",
            ),
        ];
        let snapshots = vec![
            snapshot("snap-b", "vol-data", "2026-10-05T00:00:00Z", true),
            snapshot("snap-d", "vol-root", "2026-10-07T00:00:00Z", true),
        ];
        let owner = HashMap::from([("vol-root", "i-1"), ("vol-data", "i-1")]);

        let amis = attribute_amis(&images, &snapshots, &owner);
        let ids: Vec<&str> = amis["i-1"].iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["ami-a", "ami-b"]);
        assert_eq!(
            amis["i-1"][1].created.as_deref(),
            Some("2026-10-05T00:00:00Z")
        );
        assert_eq!(
            amis["i-1"][0].tags.get("keep").map(String::as_str),
            Some("yes")
        );
        assert_eq!(amis["i-gone"].len(), 1);
        assert_eq!(amis.len(), 2);
    }

    #[test]
    fn volume_snapshots_exclude_ami_backing_and_incomplete() {
        let images = vec![image(
            "ami-a",
            Some("i-1"),
            &["snap-ami"],
            "2026-10-09T00:00:00.000Z",
        )];
        let snapshots = vec![
            snapshot("snap-1", "vol-root", "2026-10-01T00:00:00Z", true),
            snapshot("snap-2", "vol-root", "2026-10-03T00:00:00Z", true),
            snapshot("snap-ami", "vol-root", "2026-10-09T00:00:00Z", true),
            snapshot("snap-3", "vol-root", "2026-10-10T00:00:00Z", false),
            snapshot("snap-4", "vol-other", "2026-10-02T00:00:00Z", true),
        ];
        let stats = snapshot_stats(&snapshots, &ami_snapshot_ids(&images));
        assert_eq!(
            stats["vol-root"],
            SnapshotStats {
                count: 2,
                latest: Some("2026-10-03T00:00:00Z".into())
            }
        );
        assert_eq!(stats["vol-other"].count, 1);
    }

    #[test]
    fn native_backups_sum_volumes_and_count_available_amis() {
        let volumes = vec!["vol-root".to_string(), "vol-data".to_string()];
        let stats = HashMap::from([
            (
                "vol-root".to_string(),
                SnapshotStats {
                    count: 3,
                    latest: Some("2026-10-03T00:00:00Z".into()),
                },
            ),
            (
                "vol-x".to_string(),
                SnapshotStats {
                    count: 9,
                    latest: Some("2026-10-20T00:00:00Z".into()),
                },
            ),
        ]);
        let ami = |id: &str, state: &str, created: &str| OwnedAmi {
            id: id.into(),
            state: Some(state.into()),
            created: Some(created.into()),
            ..OwnedAmi::default()
        };
        let amis = vec![
            ami("ami-old", "available", "2026-10-01T00:00:00Z"),
            ami("ami-new", "available", "2026-10-04T00:00:00Z"),
            ami("ami-bad", "failed", "2026-10-09T00:00:00Z"),
        ];

        let native = native_backups(&volumes, &amis, &stats);
        assert_eq!(
            native,
            vec![
                NativeBackup {
                    kind: NativeKind::Ami,
                    count: Some(2),
                    latest: Some("2026-10-04T00:00:00Z".into()),
                    detail: Some("ami-new".into()),
                },
                NativeBackup {
                    kind: NativeKind::EbsSnapshot,
                    count: Some(3),
                    latest: Some("2026-10-03T00:00:00Z".into()),
                    detail: Some("1 of 2 volumes".into()),
                },
            ]
        );
        assert!(native_backups(&volumes, &[], &HashMap::new()).is_empty());
    }

    #[test]
    fn mssql_detector() {
        assert_eq!(
            mssql_candidate(
                Some("Windows"),
                Some("Windows with SQL Server Standard"),
                None
            )
            .as_deref(),
            Some(MSSQL_LICENSE_INCLUDED)
        );
        assert_eq!(
            mssql_candidate(None, Some("Linux with SQL Server Enterprise"), Some(false)).as_deref(),
            Some(MSSQL_LICENSE_INCLUDED)
        );
        assert_eq!(
            mssql_candidate(Some("windows"), Some("Windows"), Some(true)).as_deref(),
            Some(MSSQL_POSSIBLE)
        );
        assert_eq!(
            mssql_candidate(Some("Windows"), Some("Windows"), Some(false)),
            None
        );
        assert_eq!(mssql_candidate(Some("Windows"), None, None), None);
        assert_eq!(mssql_candidate(None, Some("Linux/UNIX"), Some(true)), None);
    }

    #[test]
    fn burstable_families() {
        for t in ["t2.micro", "t3.large", "t3a.nano", "t4g.small"] {
            assert!(burstable(t), "{t}");
        }
        for t in ["t1.micro", "trn1.2xlarge", "m5.large", "c7g.xlarge"] {
            assert!(!burstable(t), "{t}");
        }
    }

    #[test]
    fn world_open_ingress() {
        let world = IpRange::builder().cidr_ip("0.0.0.0/0").build();
        let private = IpRange::builder().cidr_ip("10.0.0.0/8").build();
        let permissions = vec![
            IpPermission::builder()
                .ip_protocol("tcp")
                .from_port(22)
                .to_port(22)
                .ip_ranges(world.clone())
                .build(),
            IpPermission::builder()
                .ip_protocol("tcp")
                .from_port(1000)
                .to_port(2000)
                .ipv6_ranges(Ipv6Range::builder().cidr_ipv6("::/0").build())
                .build(),
            IpPermission::builder()
                .ip_protocol("-1")
                .ip_ranges(private)
                .build(),
            IpPermission::builder()
                .ip_protocol("icmp")
                .from_port(-1)
                .to_port(-1)
                .ip_ranges(world)
                .build(),
        ];
        assert_eq!(
            open_to_world(&permissions),
            vec!["icmp", "tcp/1000-2000", "tcp/22"]
        );
    }

    #[test]
    fn launch_permission_sharing() {
        let (public, shared) = sharing(&[
            LaunchPermission::builder().user_id("111122223333").build(),
            LaunchPermission::builder()
                .organization_arn("arn:aws:organizations::1:organization/o-1")
                .build(),
        ]);
        assert!(!public);
        assert_eq!(shared.len(), 2);

        let (public, shared) = sharing(&[LaunchPermission::builder()
            .group(PermissionGroup::All)
            .build()]);
        assert!(public);
        assert!(shared.is_empty());
    }
}
