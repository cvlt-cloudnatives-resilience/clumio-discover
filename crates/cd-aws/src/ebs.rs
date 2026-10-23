use std::collections::{BTreeSet, HashMap, HashSet};

use aws_sdk_ec2::operation::describe_snapshot_attribute::DescribeSnapshotAttributeOutput;
use aws_sdk_ec2::types::{
    FastSnapshotRestoreStateCode, LockState, PermissionGroup, Snapshot, SnapshotAttributeName,
    SnapshotState, Volume, VolumeAttributeName,
};
use aws_sdk_ec2::Client;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use aws_smithy_types::DateTime;
use futures::future::join_all;

use cd_model::{
    DiscoveryError, EbsVolume, NativeBackup, NativeKind, RecycleBinRule, SnapshotSummary, Tags,
};

use crate::ec2::ec2_tags;
use crate::{failure, rbin, timestamp, EbsSections, Scope};

const SERVICE: &str = "ebs";

pub(crate) const ACTIONS: &[&str] = &[
    "ec2:DescribeFastSnapshotRestores",
    "ec2:DescribeLockedSnapshots",
    "ec2:DescribeSnapshotAttribute",
    "ec2:DescribeSnapshots",
    "ec2:DescribeVolumeAttribute",
    "ec2:DescribeVolumeStatus",
    "ec2:DescribeVolumes",
    "kms:DescribeKey",
];

pub async fn discover(scope: &Scope<'_>) -> EbsSections {
    let client = &Client::new(scope.sdk());
    let (rules, listed) = tokio::join!(rbin::rules(scope), list_volumes(scope, client));
    let mut errors = rules.errors;
    let mut volumes = match listed {
        Ok(volumes) => volumes,
        Err(err) => {
            errors.push(err);
            return EbsSections {
                volumes: None,
                rbin_rules: rules.items,
                errors,
            };
        }
    };
    if volumes.is_empty() {
        return EbsSections {
            volumes: Some(volumes),
            rbin_rules: rules.items,
            errors,
        };
    }

    let (statuses, snapshots, locked, fast_restores) = tokio::join!(
        volume_statuses(scope, client),
        list_snapshots(scope, client),
        locked_snapshots(scope, client),
        fast_restores(scope, client),
    );
    let statuses = ok_or_log(statuses, &mut errors).unwrap_or_default();
    let snapshots = ok_or_log(snapshots, &mut errors);
    let locked = ok_or_log(locked, &mut errors).unwrap_or_default();
    let fast_restores = ok_or_log(fast_restores, &mut errors).unwrap_or_default();

    for volume in &mut volumes {
        volume.status = statuses.get(&volume.id).cloned();
        let Some(groups) = &snapshots else { continue };
        let snaps = groups
            .get(&volume.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let summary = summarize(snaps, &locked, &fast_restores);
        volume.protection.native.extend(native_backup(&summary));
        volume.snapshots = Some(summary);
        if let Some(rules) = &rules.items {
            volume.rbin_scanned = true;
            volume.protection.rbin_rules = covering_rules(rules, snaps);
        }
    }

    let kms = &aws_sdk_kms::Client::new(scope.sdk());
    let keys: BTreeSet<String> = volumes
        .iter()
        .filter_map(|v| v.kms_key_id.clone())
        .collect();
    let deep = scope.ctx.deep();
    let snapshot_ids: Vec<String> = match (&snapshots, deep) {
        (Some(groups), true) => volumes
            .iter()
            .filter_map(|v| groups.get(&v.id))
            .flatten()
            .map(|s| s.id.clone())
            .collect(),
        _ => Vec::new(),
    };
    let volume_ids: Vec<String> = if deep {
        volumes.iter().map(|v| v.id.clone()).collect()
    } else {
        Vec::new()
    };
    let (keys, attributes, permissions) = tokio::join!(
        join_all(keys.into_iter().map(|key| describe_key(scope, kms, key))),
        join_all(
            volume_ids
                .into_iter()
                .map(|id| volume_attributes(scope, client, id))
        ),
        join_all(
            snapshot_ids
                .into_iter()
                .map(|id| snapshot_permissions(scope, client, id))
        ),
    );

    let mut key_info = HashMap::new();
    for (key, result) in keys {
        match result {
            Ok(info) => {
                key_info.insert(key, info);
            }
            Err(err) => errors.push(err),
        }
    }
    let mut volume_attrs = HashMap::new();
    for (id, mut attrs) in attributes {
        errors.append(&mut attrs.errors);
        volume_attrs.insert(id, attrs);
    }
    let mut snapshot_access = HashMap::new();
    for (id, result) in permissions {
        match result {
            Ok(Some(access)) => {
                snapshot_access.insert(id, access);
            }
            Ok(None) => {}
            Err(err) => errors.push(err),
        }
    }

    for volume in &mut volumes {
        if let Some((managed_by, state)) = volume.kms_key_id.as_ref().and_then(|k| key_info.get(k))
        {
            volume.kms_key_managed_by.clone_from(managed_by);
            volume.kms_key_state.clone_from(state);
        }
        if let Some(attrs) = volume_attrs.remove(&volume.id) {
            volume.auto_enable_io = attrs.auto_enable_io;
            volume.product_codes = attrs.product_codes;
        }
        if let Some(groups) = &snapshots {
            let snaps = groups
                .get(&volume.id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let (public, shared) = exposure(snaps, &snapshot_access);
            volume.public_snapshots = public;
            volume.shared_snapshot_accounts = shared;
        }
    }

    EbsSections {
        volumes: Some(volumes),
        rbin_rules: rules.items,
        errors,
    }
}

fn ok_or_log<T>(result: Result<T, DiscoveryError>, errors: &mut Vec<DiscoveryError>) -> Option<T> {
    result.map_err(|err| errors.push(err)).ok()
}

fn has_code(err: &impl ProvideErrorMetadata, codes: &[&str]) -> bool {
    err.code().is_some_and(|code| codes.contains(&code))
}

async fn list_volumes(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<EbsVolume>, DiscoveryError> {
    let mut pages = client
        .describe_volumes()
        .max_results(500)
        .into_paginator()
        .send();
    let mut volumes = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeVolumes", None, e))?;
        for volume in page.volumes() {
            let id = volume.volume_id().unwrap_or_default();
            let arn = scope.arn("ec2", &format!("volume/{id}"));
            let volume = map_volume(volume, &scope.region, arn);
            if scope.filters().admits(&volume.tags) {
                volumes.push(volume);
            }
        }
    }
    Ok(volumes)
}

fn map_volume(volume: &Volume, region: &str, arn: String) -> EbsVolume {
    EbsVolume {
        id: volume.volume_id().unwrap_or_default().to_string(),
        arn,
        region: region.to_string(),
        state: volume.state().map(|s| s.as_str().to_string()),
        volume_type: volume.volume_type().map(|t| t.as_str().to_string()),
        size_gib: volume.size(),
        iops: volume.iops(),
        throughput: volume.throughput(),
        multi_attach: volume.multi_attach_enabled(),
        availability_zone: volume.availability_zone().map(str::to_string),
        created_at: volume.create_time().and_then(timestamp),
        encrypted: volume.encrypted(),
        kms_key_id: volume.kms_key_id().map(str::to_string),
        attached_to: volume
            .attachments()
            .iter()
            .filter_map(|a| a.instance_id())
            .map(str::to_string)
            .collect(),
        tags: ec2_tags(volume.tags()),
        ..EbsVolume::default()
    }
}

async fn volume_statuses(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashMap<String, String>, DiscoveryError> {
    let mut pages = client
        .describe_volume_status()
        .max_results(1000)
        .into_paginator()
        .send();
    let mut statuses = HashMap::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeVolumeStatus", None, e))?;
        for item in page.volume_statuses() {
            let status = item.volume_status().and_then(|s| s.status());
            if let (Some(id), Some(status)) = (item.volume_id(), status) {
                statuses.insert(id.to_string(), status.as_str().to_string());
            }
        }
    }
    Ok(statuses)
}

struct Snap {
    id: String,
    started: Option<DateTime>,
    full_bytes: Option<i64>,
    tags: Tags,
}

type SnapshotGroups = HashMap<String, Vec<Snap>>;

async fn list_snapshots(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<SnapshotGroups, DiscoveryError> {
    let mut pages = client
        .describe_snapshots()
        .owner_ids("self")
        .max_results(1000)
        .into_paginator()
        .send();
    let mut groups = SnapshotGroups::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeSnapshots", None, e))?;
        group_snapshots(&mut groups, page.snapshots());
    }
    Ok(groups)
}

fn group_snapshots(groups: &mut SnapshotGroups, snapshots: &[Snapshot]) {
    for snapshot in snapshots {
        if snapshot.state() == Some(&SnapshotState::Error) {
            continue;
        }
        let (Some(volume), Some(id)) = (snapshot.volume_id(), snapshot.snapshot_id()) else {
            continue;
        };
        groups.entry(volume.to_string()).or_default().push(Snap {
            id: id.to_string(),
            started: snapshot.start_time().copied(),
            full_bytes: snapshot.full_snapshot_size_in_bytes(),
            tags: ec2_tags(snapshot.tags()),
        });
    }
}

async fn locked_snapshots(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashSet<String>, DiscoveryError> {
    let mut locked = HashSet::new();
    let mut token: Option<String> = None;
    loop {
        let request = client
            .describe_locked_snapshots()
            .max_results(1000)
            .set_next_token(token.clone());
        let output = match scope.call(request.send()).await {
            Ok(output) => output,
            Err(err) if has_code(&err, &["UnsupportedOperation"]) => return Ok(locked),
            Err(err) => return Err(failure(SERVICE, "DescribeLockedSnapshots", None, err)),
        };
        locked.extend(
            output
                .snapshots()
                .iter()
                .filter(|s| s.lock_state().is_some_and(is_locked))
                .filter_map(|s| s.snapshot_id())
                .map(str::to_string),
        );
        match output.next_token() {
            Some(next) if !next.is_empty() && token.as_deref() != Some(next) => {
                token = Some(next.to_string());
            }
            _ => break,
        }
    }
    Ok(locked)
}

fn is_locked(state: &LockState) -> bool {
    matches!(
        state,
        LockState::Compliance | LockState::ComplianceCooloff | LockState::Governance
    )
}

async fn fast_restores(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<HashSet<String>, DiscoveryError> {
    let mut pages = client
        .describe_fast_snapshot_restores()
        .max_results(200)
        .into_paginator()
        .send();
    let mut enabled = HashSet::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "DescribeFastSnapshotRestores", None, e))?;
        enabled.extend(
            page.fast_snapshot_restores()
                .iter()
                .filter(|r| r.state().is_some_and(fast_restore_on))
                .filter_map(|r| r.snapshot_id())
                .map(str::to_string),
        );
    }
    Ok(enabled)
}

fn fast_restore_on(state: &FastSnapshotRestoreStateCode) -> bool {
    matches!(
        state,
        FastSnapshotRestoreStateCode::Enabling
            | FastSnapshotRestoreStateCode::Optimizing
            | FastSnapshotRestoreStateCode::Enabled
    )
}

fn summarize(snaps: &[Snap], locked: &HashSet<String>, fast: &HashSet<String>) -> SnapshotSummary {
    let latest = snaps.iter().max_by_key(|s| s.started);
    SnapshotSummary {
        count: snaps.len() as u64,
        latest: latest.and_then(|s| s.started.as_ref()).and_then(timestamp),
        latest_id: latest.map(|s| s.id.clone()),
        latest_full_bytes: latest
            .and_then(|s| s.full_bytes)
            .and_then(|b| u64::try_from(b).ok()),
        locked: snaps.iter().filter(|s| locked.contains(&s.id)).count() as u64,
        fast_restore: snaps.iter().filter(|s| fast.contains(&s.id)).count() as u64,
    }
}

fn native_backup(summary: &SnapshotSummary) -> Option<NativeBackup> {
    if summary.count == 0 {
        return None;
    }
    let detail: Vec<String> = [
        (summary.locked, "locked"),
        (summary.fast_restore, "fast restore"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    Some(NativeBackup {
        kind: NativeKind::EbsSnapshot,
        count: Some(summary.count),
        latest: summary.latest.clone(),
        detail: (!detail.is_empty()).then(|| detail.join(", ")),
    })
}

fn covering_rules(rules: &[RecycleBinRule], snaps: &[Snap]) -> Vec<String> {
    rules
        .iter()
        .filter(|rule| {
            snaps
                .iter()
                .any(|s| rule.retains(RecycleBinRule::EBS_SNAPSHOT, &s.tags))
        })
        .map(|rule| rule.id.clone())
        .collect()
}

type KeyInfo = (Option<String>, Option<String>);

async fn describe_key(
    scope: &Scope<'_>,
    kms: &aws_sdk_kms::Client,
    key: String,
) -> (String, Result<KeyInfo, DiscoveryError>) {
    let result = match scope.call(kms.describe_key().key_id(&key).send()).await {
        Ok(output) => {
            let meta = output.key_metadata();
            Ok((
                meta.and_then(|m| m.key_manager())
                    .map(|m| m.as_str().to_string()),
                meta.and_then(|m| m.key_state())
                    .map(|s| s.as_str().to_string()),
            ))
        }
        Err(err) if has_code(&err, &["NotFoundException"]) => {
            Ok((None, Some("NotFound".to_string())))
        }
        Err(err) => Err(failure("kms", "DescribeKey", Some(&key), err)),
    };
    (key, result)
}

#[derive(Default)]
struct VolumeAttributes {
    auto_enable_io: Option<bool>,
    product_codes: Vec<String>,
    errors: Vec<DiscoveryError>,
}

const VOLUME_GONE: &[&str] = &["InvalidVolume.NotFound"];
const SNAPSHOT_GONE: &[&str] = &["InvalidSnapshot.NotFound"];

async fn volume_attributes(
    scope: &Scope<'_>,
    client: &Client,
    id: String,
) -> (String, VolumeAttributes) {
    let request = |attribute| {
        client
            .describe_volume_attribute()
            .volume_id(&id)
            .attribute(attribute)
            .send()
    };
    let (auto_enable_io, product_codes) = tokio::join!(
        scope.call(request(VolumeAttributeName::AutoEnableIo)),
        scope.call(request(VolumeAttributeName::ProductCodes)),
    );
    let mut attrs = VolumeAttributes::default();
    match auto_enable_io {
        Ok(output) => attrs.auto_enable_io = output.auto_enable_io().and_then(|v| v.value()),
        Err(err) if has_code(&err, VOLUME_GONE) => {}
        Err(err) => attrs
            .errors
            .push(failure(SERVICE, "DescribeVolumeAttribute", Some(&id), err)),
    }
    match product_codes {
        Ok(output) => {
            attrs.product_codes = output
                .product_codes()
                .iter()
                .filter_map(|c| c.product_code_id())
                .map(str::to_string)
                .collect();
        }
        Err(err) if has_code(&err, VOLUME_GONE) => {}
        Err(err) => attrs
            .errors
            .push(failure(SERVICE, "DescribeVolumeAttribute", Some(&id), err)),
    }
    (id, attrs)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SnapshotAccess {
    public: bool,
    accounts: Vec<String>,
}

async fn snapshot_permissions(
    scope: &Scope<'_>,
    client: &Client,
    id: String,
) -> (String, Result<Option<SnapshotAccess>, DiscoveryError>) {
    let request = client
        .describe_snapshot_attribute()
        .snapshot_id(&id)
        .attribute(SnapshotAttributeName::CreateVolumePermission)
        .send();
    let result = match scope.call(request).await {
        Ok(output) => Ok(Some(access(&output))),
        Err(err) if has_code(&err, SNAPSHOT_GONE) => Ok(None),
        Err(err) => Err(failure(
            SERVICE,
            "DescribeSnapshotAttribute",
            Some(&id),
            err,
        )),
    };
    (id, result)
}

fn access(output: &DescribeSnapshotAttributeOutput) -> SnapshotAccess {
    let permissions = output.create_volume_permissions();
    SnapshotAccess {
        public: permissions
            .iter()
            .any(|p| p.group() == Some(&PermissionGroup::All)),
        accounts: permissions
            .iter()
            .filter_map(|p| p.user_id())
            .map(str::to_string)
            .collect(),
    }
}

fn exposure(
    snaps: &[Snap],
    access: &HashMap<String, SnapshotAccess>,
) -> (Vec<String>, Vec<String>) {
    let mut public = Vec::new();
    let mut accounts = BTreeSet::new();
    for snap in snaps {
        let Some(found) = access.get(&snap.id) else {
            continue;
        };
        if found.public {
            public.push(snap.id.clone());
        }
        accounts.extend(found.accounts.iter().cloned());
    }
    public.sort();
    (public, accounts.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use aws_sdk_ec2::types::{
        CreateVolumePermission, Tag, VolumeAttachment, VolumeState, VolumeType,
    };
    use cd_model::{Asset, RuleTag};

    use super::*;

    fn tag(key: &str, value: &str) -> Tag {
        Tag::builder().key(key).value(value).build()
    }

    fn snapshot(id: &str, volume: &str, started: i64) -> Snapshot {
        Snapshot::builder()
            .snapshot_id(id)
            .volume_id(volume)
            .state(SnapshotState::Completed)
            .start_time(DateTime::from_secs(started))
            .build()
    }

    fn snap(id: &str, tags: &[(&str, &str)]) -> Snap {
        Snap {
            id: id.into(),
            started: None,
            full_bytes: None,
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn ids(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn maps_a_described_volume() {
        let volume = Volume::builder()
            .volume_id("vol-1")
            .state(VolumeState::InUse)
            .volume_type(VolumeType::Gp3)
            .size(100)
            .iops(3000)
            .throughput(125)
            .multi_attach_enabled(false)
            .availability_zone("ap-southeast-2a")
            .create_time(DateTime::from_secs(1_792_713_600))
            .encrypted(true)
            .kms_key_id("arn:aws:kms:ap-southeast-2:1:key/k1")
            .attachments(VolumeAttachment::builder().instance_id("i-1").build())
            .tags(tag("Name", "db-data"))
            .build();
        let v = map_volume(&volume, "ap-southeast-2", "arn:vol-1".into());

        assert_eq!(v.id, "vol-1");
        assert_eq!(v.arn, "arn:vol-1");
        assert_eq!(v.state.as_deref(), Some("in-use"));
        assert_eq!(v.volume_type.as_deref(), Some("gp3"));
        assert_eq!((v.iops, v.throughput), (Some(3000), Some(125)));
        assert_eq!(v.multi_attach, Some(false));
        assert_eq!(v.availability_zone.as_deref(), Some("ap-southeast-2a"));
        assert_eq!(v.created_at.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(
            v.kms_key_id.as_deref(),
            Some("arn:aws:kms:ap-southeast-2:1:key/k1")
        );
        assert_eq!(v.attached_to, vec!["i-1"]);
        assert_eq!(v.name(), "db-data");
        assert!(v.snapshots.is_none());
    }

    #[test]
    fn groups_snapshots_by_volume_and_skips_failed_ones() {
        let mut groups = SnapshotGroups::new();
        group_snapshots(
            &mut groups,
            &[
                snapshot("snap-1", "vol-1", 100),
                snapshot("snap-2", "vol-1", 200),
                snapshot("snap-3", "vol-2", 100),
                Snapshot::builder()
                    .snapshot_id("snap-4")
                    .volume_id("vol-1")
                    .state(SnapshotState::Error)
                    .build(),
                Snapshot::builder().snapshot_id("snap-5").build(),
            ],
        );
        group_snapshots(&mut groups, &[snapshot("snap-6", "vol-2", 300)]);

        let of =
            |volume: &str| -> Vec<&str> { groups[volume].iter().map(|s| s.id.as_str()).collect() };
        assert_eq!(groups.len(), 2);
        assert_eq!(of("vol-1"), vec!["snap-1", "snap-2"]);
        assert_eq!(of("vol-2"), vec!["snap-3", "snap-6"]);
    }

    #[test]
    fn summarizes_latest_locked_and_fast_restore() {
        let mut groups = SnapshotGroups::new();
        group_snapshots(
            &mut groups,
            &[
                snapshot("snap-old", "vol-1", 1_792_000_000),
                Snapshot::builder()
                    .snapshot_id("snap-new")
                    .volume_id("vol-1")
                    .state(SnapshotState::Completed)
                    .start_time(DateTime::from_secs(1_792_713_600))
                    .full_snapshot_size_in_bytes(4096)
                    .build(),
                snapshot("snap-mid", "vol-1", 1_792_500_000),
            ],
        );
        let summary = summarize(
            &groups["vol-1"],
            &ids(&["snap-old", "snap-mid", "snap-elsewhere"]),
            &ids(&["snap-new"]),
        );
        assert_eq!(
            summary,
            SnapshotSummary {
                count: 3,
                latest: Some("2026-10-23T00:00:00Z".into()),
                latest_id: Some("snap-new".into()),
                latest_full_bytes: Some(4096),
                locked: 2,
                fast_restore: 1,
            }
        );

        let native = native_backup(&summary).unwrap();
        assert_eq!(native.kind, NativeKind::EbsSnapshot);
        assert_eq!(native.count, Some(3));
        assert_eq!(native.latest.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(native.detail.as_deref(), Some("2 locked, 1 fast restore"));

        let empty = summarize(&[], &HashSet::new(), &HashSet::new());
        assert_eq!(empty.count, 0);
        assert!(native_backup(&empty).is_none());
    }

    #[test]
    fn plain_snapshots_carry_no_detail() {
        let summary = SnapshotSummary {
            count: 2,
            ..SnapshotSummary::default()
        };
        assert_eq!(native_backup(&summary).unwrap().detail, None);
    }

    #[test]
    fn lock_and_fast_restore_states() {
        assert!(is_locked(&LockState::Compliance));
        assert!(is_locked(&LockState::ComplianceCooloff));
        assert!(is_locked(&LockState::Governance));
        assert!(!is_locked(&LockState::Expired));
        assert!(fast_restore_on(&FastSnapshotRestoreStateCode::Optimizing));
        assert!(!fast_restore_on(&FastSnapshotRestoreStateCode::Disabled));
    }

    fn rule(id: &str, resource_type: &str, resource_tags: Vec<RuleTag>) -> RecycleBinRule {
        RecycleBinRule {
            id: id.into(),
            resource_type: resource_type.into(),
            status: Some("available".into()),
            resource_tags,
            exclude_tags: vec![RuleTag {
                key: "rbin".into(),
                value: Some("skip".into()),
            }],
            ..RecycleBinRule::default()
        }
    }

    #[test]
    fn rbin_coverage_follows_snapshot_tags() {
        let rules = [
            rule("region-wide", RecycleBinRule::EBS_SNAPSHOT, Vec::new()),
            rule(
                "prod-only",
                RecycleBinRule::EBS_SNAPSHOT,
                vec![RuleTag {
                    key: "env".into(),
                    value: Some("prod".into()),
                }],
            ),
            rule("amis", RecycleBinRule::EC2_IMAGE, Vec::new()),
        ];

        let prod = [snap("snap-1", &[("env", "prod")])];
        assert_eq!(
            covering_rules(&rules, &prod),
            vec!["region-wide", "prod-only"]
        );

        let excluded = [snap("snap-1", &[("rbin", "skip")])];
        assert!(covering_rules(&rules, &excluded).is_empty());

        let one_of_two = [
            snap("snap-1", &[("rbin", "skip")]),
            snap("snap-2", &[("env", "prod"), ("rbin", "skip")]),
        ];
        assert_eq!(covering_rules(&rules, &one_of_two), vec!["prod-only"]);

        assert!(covering_rules(&rules, &[]).is_empty());
    }

    #[test]
    fn snapshot_permissions_reveal_public_and_shared_access() {
        let output = DescribeSnapshotAttributeOutput::builder()
            .snapshot_id("snap-1")
            .create_volume_permissions(
                CreateVolumePermission::builder()
                    .group(PermissionGroup::All)
                    .build(),
            )
            .create_volume_permissions(
                CreateVolumePermission::builder()
                    .user_id("222233334444")
                    .build(),
            )
            .build();
        assert_eq!(
            access(&output),
            SnapshotAccess {
                public: true,
                accounts: vec!["222233334444".into()],
            }
        );
        let private = DescribeSnapshotAttributeOutput::builder()
            .snapshot_id("snap-2")
            .build();
        assert_eq!(access(&private), SnapshotAccess::default());
    }

    #[test]
    fn exposure_rolls_up_per_volume() {
        let snaps = [
            snap("snap-2", &[]),
            snap("snap-1", &[]),
            snap("snap-3", &[]),
        ];
        let access = HashMap::from([
            (
                "snap-2".to_string(),
                SnapshotAccess {
                    public: true,
                    accounts: vec!["2".into(), "1".into()],
                },
            ),
            (
                "snap-1".to_string(),
                SnapshotAccess {
                    public: true,
                    accounts: vec!["1".into()],
                },
            ),
        ]);
        let (public, shared) = exposure(&snaps, &access);
        assert_eq!(public, vec!["snap-1", "snap-2"]);
        assert_eq!(shared, vec!["1", "2"]);
    }
}
