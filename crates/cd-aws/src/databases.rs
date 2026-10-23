use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::time::{Duration, SystemTime};

use aws_sdk_cloudwatch::types::{Datapoint, Dimension, Statistic};
use aws_sdk_rds::types::{
    DbCluster, DbClusterSnapshot, DbInstance, DbInstanceAutomatedBackup, DbSnapshot, GlobalCluster,
    Tag,
};
use aws_sdk_rds::Client;
use aws_smithy_types::DateTime;
use futures::future::join_all;

use cd_model::{
    ClusterMember, Database, DbKind, DiscoveryError, DocumentDbCluster, GlobalMembership,
    GlobalRole, NativeBackup, NativeKind, NeptuneCluster, RdsDatabase, RetainedBackup,
    ServerlessV2Capacity, Services, SnapshotStats, Tags,
};

use crate::{failure, timestamp, to_tags, DbSections, Scope};

const SERVICE: &str = "rds";
const VOLUME_METRIC_DAYS: u64 = 3;
const DAY_SECS: u64 = 86_400;

pub(crate) const ACTIONS: &[&str] = &[
    "cloudwatch:GetMetricStatistics",
    "rds:DescribeDBClusterSnapshotAttributes",
    "rds:DescribeDBClusterSnapshots",
    "rds:DescribeDBClusters",
    "rds:DescribeDBInstanceAutomatedBackups",
    "rds:DescribeDBInstances",
    "rds:DescribeDBSnapshotAttributes",
    "rds:DescribeDBSnapshots",
    "rds:DescribeDBSubnetGroups",
    "rds:DescribeGlobalClusters",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Rds,
    Neptune,
    DocumentDb,
}

fn family(engine: &str) -> Family {
    match engine {
        "neptune" => Family::Neptune,
        "docdb" => Family::DocumentDb,
        _ => Family::Rds,
    }
}

impl Family {
    fn enabled(self, services: &Services) -> bool {
        match self {
            Family::Rds => services.rds,
            Family::Neptune => services.neptune,
            Family::DocumentDb => services.documentdb,
        }
    }
}

fn volume_namespace(engine: &str) -> Option<&'static str> {
    match family(engine) {
        Family::Neptune => Some("AWS/Neptune"),
        Family::DocumentDb => Some("AWS/DocDB"),
        Family::Rds if engine.starts_with("aurora") => Some("AWS/RDS"),
        Family::Rds => None,
    }
}

pub async fn discover(scope: &Scope<'_>) -> DbSections {
    let client = &Client::new(scope.sdk());
    let services = scope.ctx.config.services;
    let mut out = DbSections::default();

    let (clusters, instances) = tokio::join!(clusters(scope, client), instances(scope, client));
    let clusters = match clusters {
        Ok(clusters) => clusters,
        Err(err) => {
            out.errors.push(err);
            return out;
        }
    };
    let instances = match instances {
        Ok(instances) => Some(instances),
        Err(err) => {
            out.errors.push(err);
            None
        }
    };
    let standalone = services.rds && instances.is_some();

    let by_id: HashMap<&str, &DbInstance> = instances
        .iter()
        .flatten()
        .filter_map(|i| Some((i.db_instance_identifier()?, i)))
        .collect();
    let mut dbs: Vec<Database> = clusters
        .iter()
        .filter(|c| family(c.engine().unwrap_or_default()).enabled(&services))
        .map(|c| cluster_database(&scope.region, c, &by_id))
        .collect();
    if standalone {
        dbs.extend(
            instances
                .iter()
                .flatten()
                .filter(|i| i.db_cluster_identifier().is_none())
                .map(|i| instance_database(&scope.region, i)),
        );
    }
    dbs.retain(|db| scope.filters().admits(&db.tags));

    let any = |kind| dbs.iter().any(|db| db.kind == kind);
    let (any_clusters, any_instances) = (any(DbKind::Cluster), any(DbKind::Instance));
    let need_subnets = dbs
        .iter()
        .any(|db| db.vpc_id.is_none() && db.db_subnet_group.is_some());
    let (globals, automated, snapshots, cluster_snapshots, subnets) = tokio::join!(
        listed_if(any_clusters, global_clusters(scope, client)),
        listed_if(standalone, automated_backups(scope, client)),
        listed_if(any_instances, db_snapshots(scope, client)),
        listed_if(any_clusters, cluster_snapshots(scope, client)),
        listed_if(need_subnets, subnet_groups(scope, client)),
    );
    let globals = global_memberships(&or_record(globals, &mut out.errors));
    let automated = or_record(automated, &mut out.errors);
    let mut snapshots = aggregate(or_record(snapshots, &mut out.errors));
    let mut cluster_snapshots = aggregate(or_record(cluster_snapshots, &mut out.errors));
    let subnets: HashMap<String, String> =
        or_record(subnets, &mut out.errors).into_iter().collect();

    let live = attach_automated_backups(&mut dbs, &automated);
    let retained = retained_backups(&scope.region, &automated, &live, |tags| {
        scope.filters().admits(tags)
    });
    let mut manual_ids = Vec::new();
    for (index, db) in dbs.iter_mut().enumerate() {
        let found = match db.kind {
            DbKind::Cluster => cluster_snapshots.remove(&db.identifier),
            DbKind::Instance => snapshots.remove(&db.identifier),
        };
        if let Some(found) = found {
            db.automated_snapshots = found.automated;
            db.manual_snapshots = found.manual;
            if scope.ctx.deep() {
                manual_ids.extend(found.manual_ids.into_iter().map(|id| (index, db.kind, id)));
            }
        }
        if db.kind == DbKind::Cluster {
            if let Some(global) = globals.get(&db.arn) {
                db.global_cluster = Some(global.clone());
            }
        }
        if db.vpc_id.is_none() {
            db.vpc_id = db
                .db_subnet_group
                .as_ref()
                .and_then(|group| subnets.get(group))
                .cloned();
        }
    }

    let cloudwatch = &aws_sdk_cloudwatch::Client::new(scope.sdk());
    let volume_jobs: Vec<(usize, &'static str, String)> = dbs
        .iter()
        .enumerate()
        .filter(|(_, db)| db.kind == DbKind::Cluster)
        .filter_map(|(i, db)| Some((i, volume_namespace(&db.engine)?, db.identifier.clone())))
        .collect();
    let (volumes, restorers) = tokio::join!(
        join_all(
            volume_jobs
                .iter()
                .map(|(i, namespace, cluster)| async move {
                    (
                        *i,
                        volume_bytes(scope, cloudwatch, namespace, cluster).await,
                    )
                })
        ),
        join_all(manual_ids.iter().map(|(i, kind, id)| async move {
            (*i, id, snapshot_restorers(scope, client, *kind, id).await)
        })),
    );
    for (i, result) in volumes {
        match result {
            Ok(bytes) => dbs[i].used_bytes = bytes,
            Err(err) => out.errors.push(err),
        }
    }
    for (i, id, result) in restorers {
        match result {
            Ok(accounts) => record_restorers(&mut dbs[i], id, accounts),
            Err(err) => out.errors.push(err),
        }
    }

    let (mut rds, mut neptune, mut documentdb) = (Vec::new(), Vec::new(), Vec::new());
    for mut db in dbs {
        db.protection.native = native_backups(&db);
        match family(&db.engine) {
            Family::Rds => rds.push(RdsDatabase(db)),
            Family::Neptune => neptune.push(NeptuneCluster(db)),
            Family::DocumentDb => documentdb.push(DocumentDbCluster(db)),
        }
    }
    if standalone {
        out.rds = Some(rds);
        out.retained_backups = Some(retained);
    }
    if services.neptune {
        out.neptune = Some(neptune);
    }
    if services.documentdb {
        out.documentdb = Some(documentdb);
    }
    out
}

type Listed<T> = Result<Vec<T>, DiscoveryError>;

async fn listed_if<T>(enabled: bool, list: impl Future<Output = Listed<T>>) -> Listed<T> {
    if enabled {
        list.await
    } else {
        Ok(Vec::new())
    }
}

fn or_record<T>(listed: Listed<T>, errors: &mut Vec<DiscoveryError>) -> Vec<T> {
    listed.unwrap_or_else(|err| {
        errors.push(err);
        Vec::new()
    })
}

async fn clusters(scope: &Scope<'_>, client: &Client) -> Listed<DbCluster> {
    let mut pages = client.describe_db_clusters().into_paginator().send();
    let mut clusters = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeDBClusters", None, e))?;
        clusters.extend(page.db_clusters().iter().cloned());
    }
    Ok(clusters)
}

async fn instances(scope: &Scope<'_>, client: &Client) -> Listed<DbInstance> {
    let mut pages = client.describe_db_instances().into_paginator().send();
    let mut instances = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeDBInstances", None, e))?;
        instances.extend(page.db_instances().iter().cloned());
    }
    Ok(instances)
}

async fn global_clusters(scope: &Scope<'_>, client: &Client) -> Listed<GlobalCluster> {
    let mut pages = client.describe_global_clusters().into_paginator().send();
    let mut globals = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeGlobalClusters", None, e))?;
        globals.extend(page.global_clusters().iter().cloned());
    }
    Ok(globals)
}

async fn automated_backups(
    scope: &Scope<'_>,
    client: &Client,
) -> Listed<DbInstanceAutomatedBackup> {
    let mut pages = client
        .describe_db_instance_automated_backups()
        .into_paginator()
        .send();
    let mut backups = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page =
            page.map_err(|e| failure(SERVICE, "DescribeDBInstanceAutomatedBackups", None, e))?;
        backups.extend(page.db_instance_automated_backups().iter().cloned());
    }
    Ok(backups)
}

async fn db_snapshots(scope: &Scope<'_>, client: &Client) -> Listed<SnapshotRow> {
    let mut pages = client.describe_db_snapshots().into_paginator().send();
    let mut rows = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeDBSnapshots", None, e))?;
        rows.extend(page.db_snapshots().iter().filter_map(db_snapshot_row));
    }
    Ok(rows)
}

async fn cluster_snapshots(scope: &Scope<'_>, client: &Client) -> Listed<SnapshotRow> {
    let mut pages = client
        .describe_db_cluster_snapshots()
        .into_paginator()
        .send();
    let mut rows = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeDBClusterSnapshots", None, e))?;
        rows.extend(
            page.db_cluster_snapshots()
                .iter()
                .filter_map(cluster_snapshot_row),
        );
    }
    Ok(rows)
}

async fn subnet_groups(scope: &Scope<'_>, client: &Client) -> Listed<(String, String)> {
    let mut pages = client.describe_db_subnet_groups().into_paginator().send();
    let mut groups = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeDBSubnetGroups", None, e))?;
        groups.extend(page.db_subnet_groups().iter().filter_map(|g| {
            Some((
                g.db_subnet_group_name()?.to_string(),
                g.vpc_id()?.to_string(),
            ))
        }));
    }
    Ok(groups)
}

async fn volume_bytes(
    scope: &Scope<'_>,
    client: &aws_sdk_cloudwatch::Client,
    namespace: &str,
    cluster: &str,
) -> Result<Option<u64>, DiscoveryError> {
    let now = SystemTime::now();
    let since = now - Duration::from_secs(VOLUME_METRIC_DAYS * DAY_SECS);
    let output = scope
        .call(
            client
                .get_metric_statistics()
                .namespace(namespace)
                .metric_name("VolumeBytesUsed")
                .dimensions(
                    Dimension::builder()
                        .name("DBClusterIdentifier")
                        .value(cluster)
                        .build(),
                )
                .start_time(DateTime::from(since))
                .end_time(DateTime::from(now))
                .period(DAY_SECS as i32)
                .statistics(Statistic::Average)
                .send(),
        )
        .await
        .map_err(|e| failure("cloudwatch", "GetMetricStatistics", Some(cluster), e))?;
    Ok(latest_average(output.datapoints()))
}

fn latest_average(points: &[Datapoint]) -> Option<u64> {
    points
        .iter()
        .filter_map(|p| Some((p.timestamp()?, p.average()?)))
        .max_by_key(|(at, _)| *at)
        .map(|(_, bytes)| bytes.max(0.0) as u64)
}

async fn snapshot_restorers(
    scope: &Scope<'_>,
    client: &Client,
    kind: DbKind,
    id: &str,
) -> Result<Vec<String>, DiscoveryError> {
    Ok(match kind {
        DbKind::Instance => {
            let output = scope
                .call(
                    client
                        .describe_db_snapshot_attributes()
                        .db_snapshot_identifier(id)
                        .send(),
                )
                .await
                .map_err(|e| failure(SERVICE, "DescribeDBSnapshotAttributes", Some(id), e))?;
            let attributes = output
                .db_snapshot_attributes_result()
                .map(|r| r.db_snapshot_attributes())
                .unwrap_or_default();
            restore_values(
                attributes
                    .iter()
                    .map(|a| (a.attribute_name(), a.attribute_values())),
            )
        }
        DbKind::Cluster => {
            let output = scope
                .call(
                    client
                        .describe_db_cluster_snapshot_attributes()
                        .db_cluster_snapshot_identifier(id)
                        .send(),
                )
                .await
                .map_err(|e| {
                    failure(SERVICE, "DescribeDBClusterSnapshotAttributes", Some(id), e)
                })?;
            let attributes = output
                .db_cluster_snapshot_attributes_result()
                .map(|r| r.db_cluster_snapshot_attributes())
                .unwrap_or_default();
            restore_values(
                attributes
                    .iter()
                    .map(|a| (a.attribute_name(), a.attribute_values())),
            )
        }
    })
}

fn restore_values<'a>(
    attributes: impl IntoIterator<Item = (Option<&'a str>, &'a [String])>,
) -> Vec<String> {
    attributes
        .into_iter()
        .filter(|(name, _)| *name == Some("restore"))
        .flat_map(|(_, values)| values.iter().cloned())
        .collect()
}

fn record_restorers(db: &mut Database, snapshot: &str, accounts: Vec<String>) {
    let mut shared: BTreeSet<String> = db.snapshot_shared_with.drain(..).collect();
    for account in accounts {
        if account == "all" {
            if !db.public_snapshots.iter().any(|s| s == snapshot) {
                db.public_snapshots.push(snapshot.to_string());
            }
        } else {
            shared.insert(account);
        }
    }
    db.snapshot_shared_with = shared.into_iter().collect();
}

fn cluster_database(
    region: &str,
    cluster: &DbCluster,
    instances: &HashMap<&str, &DbInstance>,
) -> Database {
    let engine = cluster.engine().unwrap_or_default().to_string();
    let member_of = |m: &aws_sdk_rds::types::DbClusterMember| {
        m.db_instance_identifier()
            .and_then(|id| instances.get(id))
            .copied()
    };
    let members = cluster
        .db_cluster_members()
        .iter()
        .map(|m| {
            let instance = member_of(m);
            ClusterMember {
                id: m.db_instance_identifier().unwrap_or_default().to_string(),
                writer: m.is_cluster_writer().unwrap_or(false),
                instance_class: instance
                    .and_then(|i| i.db_instance_class())
                    .map(str::to_string),
                publicly_accessible: instance.and_then(|i| i.publicly_accessible()),
            }
        })
        .collect();
    let vpc_id = cluster
        .db_cluster_members()
        .iter()
        .filter_map(member_of)
        .find_map(|i| i.db_subnet_group()?.vpc_id())
        .map(str::to_string);

    Database {
        identifier: cluster
            .db_cluster_identifier()
            .unwrap_or_default()
            .to_string(),
        arn: cluster.db_cluster_arn().unwrap_or_default().to_string(),
        region: region.to_string(),
        kind: DbKind::Cluster,
        engine_version: cluster.engine_version().map(str::to_string),
        engine_mode: cluster.engine_mode().map(str::to_string),
        status: cluster.status().map(str::to_string),
        resource_id: cluster.db_cluster_resource_id().map(str::to_string),
        instance_class: cluster.db_cluster_instance_class().map(str::to_string),
        members,
        serverless_v2: cluster.serverless_v2_scaling_configuration().map(|s| {
            ServerlessV2Capacity {
                min_acu: s.min_capacity(),
                max_acu: s.max_capacity(),
            }
        }),
        global_cluster: cluster
            .global_cluster_identifier()
            .map(|id| GlobalMembership {
                id: id.to_string(),
                role: None,
            }),
        replica_of: cluster.replication_source_identifier().map(str::to_string),
        multi_az: cluster.multi_az(),
        publicly_accessible: cluster.publicly_accessible(),
        storage_encrypted: cluster.storage_encrypted(),
        kms_key_id: cluster.kms_key_id().map(str::to_string),
        iam_auth: cluster.iam_database_authentication_enabled(),
        deletion_protection: cluster.deletion_protection(),
        copy_tags_to_snapshot: cluster.copy_tags_to_snapshot(),
        storage_type: cluster.storage_type().map(str::to_string),
        allocated_storage_gib: if volume_namespace(&engine).is_none() {
            cluster.allocated_storage()
        } else {
            None
        },
        iops: cluster.iops(),
        backup_retention_days: cluster.backup_retention_period(),
        preferred_backup_window: cluster.preferred_backup_window().map(str::to_string),
        earliest_restorable_time: cluster.earliest_restorable_time().and_then(timestamp),
        latest_restorable_time: cluster.latest_restorable_time().and_then(timestamp),
        endpoint: cluster.endpoint().map(str::to_string),
        db_subnet_group: cluster.db_subnet_group().map(str::to_string),
        vpc_id,
        parameter_group: cluster.db_cluster_parameter_group().map(str::to_string),
        ca_certificate: cluster
            .certificate_details()
            .and_then(|c| c.ca_identifier())
            .map(str::to_string),
        tags: rds_tags(cluster.tag_list()),
        engine,
        ..Database::default()
    }
}

fn instance_database(region: &str, instance: &DbInstance) -> Database {
    let subnet_group = instance.db_subnet_group();
    Database {
        identifier: instance
            .db_instance_identifier()
            .unwrap_or_default()
            .to_string(),
        arn: instance.db_instance_arn().unwrap_or_default().to_string(),
        region: region.to_string(),
        kind: DbKind::Instance,
        engine: instance.engine().unwrap_or_default().to_string(),
        engine_version: instance.engine_version().map(str::to_string),
        status: instance.db_instance_status().map(str::to_string),
        resource_id: instance.dbi_resource_id().map(str::to_string),
        instance_class: instance.db_instance_class().map(str::to_string),
        replica_of: instance
            .read_replica_source_db_instance_identifier()
            .or(instance.read_replica_source_db_cluster_identifier())
            .map(str::to_string),
        multi_az: instance.multi_az(),
        publicly_accessible: instance.publicly_accessible(),
        storage_encrypted: instance.storage_encrypted(),
        kms_key_id: instance.kms_key_id().map(str::to_string),
        iam_auth: instance.iam_database_authentication_enabled(),
        deletion_protection: instance.deletion_protection(),
        copy_tags_to_snapshot: instance.copy_tags_to_snapshot(),
        storage_type: instance.storage_type().map(str::to_string),
        allocated_storage_gib: instance.allocated_storage(),
        iops: instance.iops(),
        backup_retention_days: instance.backup_retention_period(),
        preferred_backup_window: instance.preferred_backup_window().map(str::to_string),
        latest_restorable_time: instance.latest_restorable_time().and_then(timestamp),
        backup_replications: instance
            .db_instance_automated_backups_replications()
            .iter()
            .filter_map(|r| r.db_instance_automated_backups_arn())
            .map(str::to_string)
            .collect(),
        endpoint: instance
            .endpoint()
            .and_then(|e| e.address())
            .map(str::to_string),
        db_subnet_group: subnet_group
            .and_then(|g| g.db_subnet_group_name())
            .map(str::to_string),
        vpc_id: subnet_group.and_then(|g| g.vpc_id()).map(str::to_string),
        option_group: instance
            .option_group_memberships()
            .first()
            .and_then(|o| o.option_group_name())
            .map(str::to_string),
        parameter_group: instance
            .db_parameter_groups()
            .first()
            .and_then(|p| p.db_parameter_group_name())
            .map(str::to_string),
        ca_certificate: instance.ca_certificate_identifier().map(str::to_string),
        tags: rds_tags(instance.tag_list()),
        ..Database::default()
    }
}

fn global_memberships(globals: &[GlobalCluster]) -> HashMap<String, GlobalMembership> {
    let mut by_arn = HashMap::new();
    for global in globals {
        let Some(id) = global.global_cluster_identifier() else {
            continue;
        };
        for member in global.global_cluster_members() {
            let Some(arn) = member.db_cluster_arn() else {
                continue;
            };
            let role = member.is_writer().map(|writer| {
                if writer {
                    GlobalRole::Primary
                } else {
                    GlobalRole::Secondary
                }
            });
            by_arn.insert(
                arn.to_string(),
                GlobalMembership {
                    id: id.to_string(),
                    role,
                },
            );
        }
    }
    by_arn
}

fn attach_automated_backups(
    dbs: &mut [Database],
    backups: &[DbInstanceAutomatedBackup],
) -> BTreeSet<String> {
    let by_resource: HashMap<&str, &DbInstanceAutomatedBackup> = backups
        .iter()
        .filter_map(|b| Some((b.dbi_resource_id()?, b)))
        .collect();
    let mut live = BTreeSet::new();
    for db in dbs.iter_mut().filter(|db| db.kind == DbKind::Instance) {
        let Some(id) = db.resource_id.as_deref() else {
            continue;
        };
        let Some(backup) = by_resource.get(id) else {
            continue;
        };
        live.insert(id.to_string());
        let window = backup.restore_window();
        db.earliest_restorable_time = window.and_then(|w| w.earliest_time()).and_then(timestamp);
        if db.latest_restorable_time.is_none() {
            db.latest_restorable_time = window.and_then(|w| w.latest_time()).and_then(timestamp);
        }
    }
    live
}

fn retained_backups(
    region: &str,
    backups: &[DbInstanceAutomatedBackup],
    live: &BTreeSet<String>,
    admits: impl Fn(&Tags) -> bool,
) -> Vec<RetainedBackup> {
    backups
        .iter()
        .filter(|b| b.status() == Some("retained"))
        .filter(|b| b.dbi_resource_id().is_none_or(|id| !live.contains(id)))
        .map(|b| {
            let window = b.restore_window();
            RetainedBackup {
                identifier: b.db_instance_identifier().unwrap_or_default().to_string(),
                arn: b
                    .db_instance_automated_backups_arn()
                    .unwrap_or_default()
                    .to_string(),
                region: region.to_string(),
                engine: b.engine().map(str::to_string),
                resource_id: b.dbi_resource_id().map(str::to_string),
                earliest_restorable_time: window
                    .and_then(|w| w.earliest_time())
                    .and_then(timestamp),
                latest_restorable_time: window.and_then(|w| w.latest_time()).and_then(timestamp),
                backup_retention_days: b.backup_retention_period(),
                allocated_storage_gib: b.allocated_storage(),
                encrypted: b.encrypted(),
                tags: rds_tags(b.tag_list()),
            }
        })
        .filter(|b| admits(&b.tags))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotRow {
    owner: String,
    id: String,
    manual: bool,
    created: Option<String>,
    allocated_gib: Option<i32>,
}

fn snapshot_row(
    owner: Option<&str>,
    id: Option<&str>,
    kind: Option<&str>,
    status: Option<&str>,
    created: Option<&DateTime>,
    allocated_gib: Option<i32>,
) -> Option<SnapshotRow> {
    let manual = match kind? {
        "manual" => true,
        "automated" => false,
        _ => return None,
    };
    if status != Some("available") {
        return None;
    }
    Some(SnapshotRow {
        owner: owner?.to_string(),
        id: id?.to_string(),
        manual,
        created: created.and_then(timestamp),
        allocated_gib,
    })
}

fn db_snapshot_row(s: &DbSnapshot) -> Option<SnapshotRow> {
    snapshot_row(
        s.db_instance_identifier(),
        s.db_snapshot_identifier(),
        s.snapshot_type(),
        s.status(),
        s.snapshot_create_time(),
        s.allocated_storage(),
    )
}

fn cluster_snapshot_row(s: &DbClusterSnapshot) -> Option<SnapshotRow> {
    snapshot_row(
        s.db_cluster_identifier(),
        s.db_cluster_snapshot_identifier(),
        s.snapshot_type(),
        s.status(),
        s.snapshot_create_time(),
        s.allocated_storage(),
    )
}

#[derive(Debug, Default)]
struct Snapshots {
    automated: SnapshotStats,
    manual: SnapshotStats,
    manual_ids: Vec<String>,
}

fn aggregate(rows: Vec<SnapshotRow>) -> HashMap<String, Snapshots> {
    let mut by_owner: HashMap<String, Snapshots> = HashMap::new();
    for row in rows {
        let entry = by_owner.entry(row.owner).or_default();
        if row.manual {
            entry.manual.add(row.created, row.allocated_gib);
            entry.manual_ids.push(row.id);
        } else {
            entry.automated.add(row.created, row.allocated_gib);
        }
    }
    by_owner
}

fn native_backups(db: &Database) -> Vec<NativeBackup> {
    let mut native = Vec::new();
    if let Some(days) = db.backup_retention_days.filter(|days| *days > 0) {
        native.push(NativeBackup {
            kind: NativeKind::RdsAutomatedBackup,
            count: (!db.automated_snapshots.is_empty()).then_some(db.automated_snapshots.count),
            latest: db.latest_restorable_time.clone(),
            detail: Some(format!("{days} day{}", if days == 1 { "" } else { "s" })),
        });
    }
    if !db.manual_snapshots.is_empty() {
        native.push(NativeBackup {
            kind: match db.kind {
                DbKind::Cluster => NativeKind::RdsClusterSnapshot,
                DbKind::Instance => NativeKind::RdsSnapshot,
            },
            count: Some(db.manual_snapshots.count),
            latest: db.manual_snapshots.latest.clone(),
            detail: (db.manual_snapshots.allocated_gib > 0)
                .then(|| format!("{} GiB allocated", db.manual_snapshots.allocated_gib)),
        });
    }
    native
}

fn rds_tags(tags: &[Tag]) -> Tags {
    to_tags(
        tags.iter()
            .map(|t| (t.key().unwrap_or_default(), t.value().unwrap_or_default())),
    )
}

#[cfg(test)]
mod tests {
    use aws_sdk_rds::types::{
        DbClusterMember, DbSubnetGroup, Endpoint, GlobalClusterMember, RestoreWindow,
        ServerlessV2ScalingConfigurationInfo,
    };

    use super::*;

    fn at(secs: i64) -> DateTime {
        DateTime::from_secs(secs)
    }

    fn member_instance(id: &str, class: &str, public: bool) -> DbInstance {
        DbInstance::builder()
            .db_instance_identifier(id)
            .db_instance_class(class)
            .db_cluster_identifier("orders")
            .publicly_accessible(public)
            .db_subnet_group(DbSubnetGroup::builder().vpc_id("vpc-1").build())
            .build()
    }

    fn aurora() -> DbCluster {
        DbCluster::builder()
            .db_cluster_identifier("orders")
            .db_cluster_arn("arn:aws:rds:ap-southeast-2:1:cluster:orders")
            .engine("aurora-postgresql")
            .engine_version("16.4")
            .engine_mode("provisioned")
            .allocated_storage(1)
            .storage_encrypted(true)
            .backup_retention_period(7)
            .latest_restorable_time(at(1_792_713_600))
            .db_subnet_group("default")
            .db_cluster_members(
                DbClusterMember::builder()
                    .db_instance_identifier("orders-1")
                    .is_cluster_writer(true)
                    .build(),
            )
            .db_cluster_members(
                DbClusterMember::builder()
                    .db_instance_identifier("orders-2")
                    .is_cluster_writer(false)
                    .build(),
            )
            .serverless_v2_scaling_configuration(
                ServerlessV2ScalingConfigurationInfo::builder()
                    .min_capacity(0.5)
                    .max_capacity(16.0)
                    .build(),
            )
            .tag_list(Tag::builder().key("env").value("prod").build())
            .build()
    }

    #[test]
    fn splits_rds_api_engines_into_clumio_services() {
        assert!(matches!(family("neptune"), Family::Neptune));
        assert!(matches!(family("docdb"), Family::DocumentDb));
        for engine in ["aurora-postgresql", "aurora-mysql", "postgres", "mysql"] {
            assert!(matches!(family(engine), Family::Rds), "{engine}");
        }
        let services = Services {
            rds: false,
            ..Services::default()
        };
        assert!(!Family::Rds.enabled(&services));
        assert!(Family::Neptune.enabled(&services));
    }

    #[test]
    fn volume_metrics_only_for_aurora_style_storage() {
        assert_eq!(volume_namespace("aurora-mysql"), Some("AWS/RDS"));
        assert_eq!(volume_namespace("docdb"), Some("AWS/DocDB"));
        assert_eq!(volume_namespace("neptune"), Some("AWS/Neptune"));
        assert_eq!(volume_namespace("postgres"), None);
    }

    #[test]
    fn maps_a_cluster_with_its_members() {
        let instances = [
            member_instance("orders-1", "db.r6g.large", false),
            member_instance("orders-2", "db.serverless", true),
        ];
        let by_id: HashMap<&str, &DbInstance> = instances
            .iter()
            .map(|i| (i.db_instance_identifier().unwrap(), i))
            .collect();
        let db = cluster_database("ap-southeast-2", &aurora(), &by_id);

        assert_eq!(db.kind, DbKind::Cluster);
        assert_eq!(db.engine, "aurora-postgresql");
        assert_eq!(db.members.len(), 2);
        assert!(db.members[0].writer && !db.members[1].writer);
        assert_eq!(
            db.members[1].instance_class.as_deref(),
            Some("db.serverless")
        );
        assert_eq!(db.members[1].publicly_accessible, Some(true));
        assert_eq!(db.vpc_id.as_deref(), Some("vpc-1"));
        assert_eq!(db.allocated_storage_gib, None, "aurora placeholder ignored");
        let capacity = db.serverless_v2.as_ref().unwrap();
        assert_eq!(
            (capacity.min_acu, capacity.max_acu),
            (Some(0.5), Some(16.0))
        );
        assert_eq!(
            db.latest_restorable_time.as_deref(),
            Some("2026-10-23T00:00:00Z")
        );
        assert_eq!(db.tags.get("env").map(String::as_str), Some("prod"));
    }

    #[test]
    fn maps_a_standalone_instance() {
        let instance = DbInstance::builder()
            .db_instance_identifier("billing")
            .db_instance_arn("arn:aws:rds:us-east-1:1:db:billing")
            .engine("postgres")
            .engine_version("16.4")
            .db_instance_class("db.t3.micro")
            .allocated_storage(20)
            .publicly_accessible(true)
            .backup_retention_period(1)
            .dbi_resource_id("db-ABC")
            .endpoint(
                Endpoint::builder()
                    .address("billing.x.rds.amazonaws.com")
                    .build(),
            )
            .db_subnet_group(
                DbSubnetGroup::builder()
                    .db_subnet_group_name("default")
                    .vpc_id("vpc-9")
                    .build(),
            )
            .build();
        let db = instance_database("us-east-1", &instance);

        assert_eq!(db.kind, DbKind::Instance);
        assert_eq!(db.instance_class.as_deref(), Some("db.t3.micro"));
        assert_eq!(db.size_bytes(), Some(20 * 1024 * 1024 * 1024));
        assert_eq!(db.vpc_id.as_deref(), Some("vpc-9"));
        assert_eq!(db.endpoint.as_deref(), Some("billing.x.rds.amazonaws.com"));
        assert_eq!(db.resource_id.as_deref(), Some("db-ABC"));
        assert_eq!(db.summary(), "postgres 16.4 · db.t3.micro · 20 GiB");
    }

    fn snapshot(owner: &str, id: &str, kind: &str, created: i64, gib: i32) -> DbSnapshot {
        DbSnapshot::builder()
            .db_instance_identifier(owner)
            .db_snapshot_identifier(id)
            .snapshot_type(kind)
            .status("available")
            .snapshot_create_time(at(created))
            .allocated_storage(gib)
            .build()
    }

    #[test]
    fn aggregates_snapshots_per_database_and_type() {
        let in_flight = DbSnapshot::builder()
            .db_instance_identifier("billing")
            .db_snapshot_identifier("in-flight")
            .snapshot_type("manual")
            .status("creating")
            .build();
        let snapshots = [
            snapshot("billing", "rds:billing-1", "automated", 1_000, 20),
            snapshot("billing", "rds:billing-2", "automated", 2_000, 20),
            snapshot("billing", "pre-upgrade", "manual", 1_500, 20),
            snapshot("billing", "awsbackup:job-1", "awsbackup", 3_000, 20),
            snapshot("ledger", "ledger-final", "manual", 500, 100),
            in_flight,
        ];
        let by_owner = aggregate(snapshots.iter().filter_map(db_snapshot_row).collect());

        let billing = &by_owner["billing"];
        assert_eq!(billing.automated.count, 2);
        assert_eq!(
            billing.automated.latest.as_deref(),
            Some("1970-01-01T00:33:20Z")
        );
        assert_eq!(billing.automated.allocated_gib, 40);
        assert_eq!(billing.manual.count, 1);
        assert_eq!(billing.manual_ids, ["pre-upgrade"]);
        assert_eq!(by_owner["ledger"].manual.allocated_gib, 100);
    }

    #[test]
    fn cluster_snapshots_key_by_cluster() {
        let row = cluster_snapshot_row(
            &DbClusterSnapshot::builder()
                .db_cluster_identifier("orders")
                .db_cluster_snapshot_identifier("orders-manual")
                .snapshot_type("manual")
                .status("available")
                .build(),
        )
        .unwrap();
        assert_eq!((row.owner.as_str(), row.manual), ("orders", true));
    }

    #[test]
    fn native_backups_reflect_retention_and_manual_snapshots() {
        let mut db = Database {
            kind: DbKind::Cluster,
            backup_retention_days: Some(7),
            latest_restorable_time: Some("2026-10-23T00:00:00Z".into()),
            ..Database::default()
        };
        db.automated_snapshots
            .add(Some("2026-10-22T00:00:00Z".into()), None);
        db.manual_snapshots
            .add(Some("2026-10-01T00:00:00Z".into()), Some(5));
        let native = native_backups(&db);

        assert_eq!(native.len(), 2);
        assert_eq!(native[0].kind, NativeKind::RdsAutomatedBackup);
        assert_eq!(native[0].count, Some(1));
        assert_eq!(native[0].detail.as_deref(), Some("7 days"));
        assert_eq!(native[0].latest.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(native[1].kind, NativeKind::RdsClusterSnapshot);
        assert_eq!(native[1].detail.as_deref(), Some("5 GiB allocated"));

        let off = Database {
            backup_retention_days: Some(0),
            ..Database::default()
        };
        assert!(native_backups(&off).is_empty());
    }

    #[test]
    fn automated_backups_attach_live_and_keep_deleted_as_retained() {
        let window = RestoreWindow::builder()
            .earliest_time(at(0))
            .latest_time(at(86_400))
            .build();
        let backups = [
            DbInstanceAutomatedBackup::builder()
                .dbi_resource_id("db-LIVE")
                .db_instance_identifier("billing")
                .status("active")
                .restore_window(window.clone())
                .build(),
            DbInstanceAutomatedBackup::builder()
                .dbi_resource_id("db-GONE")
                .db_instance_identifier("billing")
                .db_instance_automated_backups_arn("arn:aws:rds:us-east-1:1:auto-backup:ab-1")
                .status("retained")
                .restore_window(window)
                .build(),
        ];
        let mut dbs = vec![Database {
            identifier: "billing".into(),
            resource_id: Some("db-LIVE".into()),
            ..Database::default()
        }];
        let live = attach_automated_backups(&mut dbs, &backups);

        assert_eq!(
            dbs[0].earliest_restorable_time.as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
        assert_eq!(
            dbs[0].latest_restorable_time.as_deref(),
            Some("1970-01-02T00:00:00Z")
        );
        let retained = retained_backups("us-east-1", &backups, &live, |_| true);
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].resource_id.as_deref(), Some("db-GONE"));
        assert!(retained_backups("us-east-1", &backups, &live, |_| false).is_empty());
    }

    #[test]
    fn global_members_get_their_role() {
        let global = GlobalCluster::builder()
            .global_cluster_identifier("orders-global")
            .global_cluster_members(
                GlobalClusterMember::builder()
                    .db_cluster_arn("arn:a")
                    .is_writer(true)
                    .build(),
            )
            .global_cluster_members(
                GlobalClusterMember::builder()
                    .db_cluster_arn("arn:b")
                    .is_writer(false)
                    .build(),
            )
            .build();
        let by_arn = global_memberships(&[global]);
        assert_eq!(by_arn["arn:a"].role, Some(GlobalRole::Primary));
        assert_eq!(by_arn["arn:b"].role, Some(GlobalRole::Secondary));
        assert_eq!(by_arn["arn:b"].id, "orders-global");
    }

    #[test]
    fn restore_attribute_marks_public_and_shared_snapshots() {
        let values = vec!["all".to_string(), "222222222222".to_string()];
        let other = vec!["x".to_string()];
        let accounts = restore_values([
            (Some("restore"), values.as_slice()),
            (Some("other"), other.as_slice()),
        ]);
        assert_eq!(accounts, ["all", "222222222222"]);

        let mut db = Database::default();
        record_restorers(&mut db, "snap-a", accounts);
        record_restorers(
            &mut db,
            "snap-b",
            vec!["111111111111".into(), "222222222222".into()],
        );
        assert_eq!(db.public_snapshots, ["snap-a"]);
        assert_eq!(db.snapshot_shared_with, ["111111111111", "222222222222"]);
    }

    #[test]
    fn latest_datapoint_wins() {
        let points = [
            Datapoint::builder().timestamp(at(200)).average(5.0).build(),
            Datapoint::builder().timestamp(at(300)).average(7.0).build(),
            Datapoint::builder().timestamp(at(100)).average(9.0).build(),
            Datapoint::builder().average(11.0).build(),
        ];
        assert_eq!(latest_average(&points), Some(7));
        assert_eq!(latest_average(&[]), None);
    }
}
