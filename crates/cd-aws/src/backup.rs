use std::collections::{BTreeMap, BTreeSet};

use aws_sdk_backup::operation::describe_recovery_point::DescribeRecoveryPointOutput;
use aws_sdk_backup::types::{
    BackupVaultListMember, ProtectedResource as Listed, RecoveryPointByBackupVault,
    RecoveryPointCreator, RecoveryPointStatus,
};
use aws_sdk_backup::Client;
use aws_smithy_types::DateTime;
use futures::future::join_all;

use cd_model::{AwsBackupCoverage, BackupVault, DiscoveryError, ProtectedResource, RecoveryPoint};

use crate::{failure, timestamp, BackupSections, Scope};

const SERVICE: &str = "backup";

pub(crate) const ACTIONS: &[&str] = &[
    "backup:DescribeRecoveryPoint",
    "backup:ListBackupVaults",
    "backup:ListProtectedResources",
    "backup:ListRecoveryPointsByBackupVault",
];

pub async fn discover(scope: &Scope<'_>) -> BackupSections {
    let client = &Client::new(scope.sdk());
    let mut errors = Vec::new();
    let (listed, vaults) = tokio::join!(
        protected_resources(client, scope),
        backup_vaults(client, scope)
    );
    let listed = listed.map_err(|e| errors.push(e)).ok();
    let mut vaults = vaults.map_err(|e| errors.push(e)).ok();

    let mut points = Vec::new();
    let mut points_listed = false;
    if let Some(vaults) = &mut vaults {
        let listings = join_all(
            vaults
                .iter()
                .map(|vault| recovery_points(client, scope, &vault.name)),
        )
        .await;
        for (vault, listing) in vaults.iter_mut().zip(listings) {
            match listing {
                Ok(listing) => {
                    points_listed = true;
                    vault_totals(vault, &listing);
                    points.extend(listing.into_iter().map(|p| (vault.name.clone(), p)));
                }
                Err(err) => errors.push(err),
            }
        }
    }

    let mut rows = resources(
        &scope.region,
        listed.as_deref().unwrap_or_default(),
        &points,
    );
    if scope.ctx.deep() {
        describe_latest(client, scope, &mut rows, &mut errors).await;
    }

    BackupSections {
        protected: (listed.is_some() || points_listed).then_some(rows),
        vaults,
        errors,
    }
}

async fn protected_resources(
    client: &Client,
    scope: &Scope<'_>,
) -> Result<Vec<Listed>, DiscoveryError> {
    let mut pages = client.list_protected_resources().into_paginator().send();
    let mut resources = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListProtectedResources", None, e))?;
        resources.extend(page.results().iter().cloned());
    }
    Ok(resources)
}

async fn backup_vaults(
    client: &Client,
    scope: &Scope<'_>,
) -> Result<Vec<BackupVault>, DiscoveryError> {
    let mut pages = client.list_backup_vaults().into_paginator().send();
    let mut vaults = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListBackupVaults", None, e))?;
        vaults.extend(
            page.backup_vault_list()
                .iter()
                .filter(|v| v.backup_vault_name().is_some())
                .map(|v| vault(v, &scope.region)),
        );
    }
    Ok(vaults)
}

async fn recovery_points(
    client: &Client,
    scope: &Scope<'_>,
    vault: &str,
) -> Result<Vec<RecoveryPointByBackupVault>, DiscoveryError> {
    let mut pages = client
        .list_recovery_points_by_backup_vault()
        .backup_vault_name(vault)
        .into_paginator()
        .send();
    let mut points = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page =
            page.map_err(|e| failure(SERVICE, "ListRecoveryPointsByBackupVault", Some(vault), e))?;
        points.extend(page.recovery_points().iter().cloned());
    }
    Ok(points)
}

async fn describe_latest(
    client: &Client,
    scope: &Scope<'_>,
    rows: &mut [ProtectedResource],
    errors: &mut Vec<DiscoveryError>,
) {
    let described = join_all(rows.iter().map(|row| async move {
        let latest = row.latest.as_ref()?;
        let call = client
            .describe_recovery_point()
            .backup_vault_name(&latest.vault)
            .recovery_point_arn(&latest.arn)
            .send();
        Some(
            scope
                .call(call)
                .await
                .map_err(|e| failure(SERVICE, "DescribeRecoveryPoint", Some(&latest.arn), e)),
        )
    }))
    .await;
    for (row, result) in rows.iter_mut().zip(described) {
        match (row.latest.as_mut(), result) {
            (Some(latest), Some(Ok(output))) => apply_described(latest, &output),
            (_, Some(Err(err))) => errors.push(err),
            _ => {}
        }
    }
}

fn vault(v: &BackupVaultListMember, region: &str) -> BackupVault {
    BackupVault {
        name: v.backup_vault_name().unwrap_or_default().to_string(),
        arn: v.backup_vault_arn().unwrap_or_default().to_string(),
        region: region.to_string(),
        recovery_points: Some(v.number_of_recovery_points()),
        locked: v.locked(),
        vault_type: v.vault_type().map(|t| t.as_str().to_string()),
        vault_state: v.vault_state().map(|s| s.as_str().to_string()),
        created: v.creation_date().and_then(timestamp),
        encryption_key_arn: v.encryption_key_arn().map(str::to_string),
        encryption_key_type: v.encryption_key_type().map(|t| t.as_str().to_string()),
        min_retention_days: v.min_retention_days(),
        max_retention_days: v.max_retention_days(),
        lock_date: v.lock_date().and_then(timestamp),
        ..BackupVault::default()
    }
}

fn vault_totals(vault: &mut BackupVault, points: &[RecoveryPointByBackupVault]) {
    let resources: BTreeSet<&str> = points.iter().filter_map(|p| p.resource_arn()).collect();
    vault.recovery_point_bytes = Some(points.iter().filter_map(size).sum());
    vault.resources = Some(resources.len() as u64);
    vault.points_by_status = status_counts(points.iter());
    vault.latest_recovery_point = points
        .iter()
        .filter_map(|p| p.creation_date())
        .max()
        .and_then(timestamp);
}

#[derive(Default)]
struct Tally<'a> {
    resource_type: Option<&'a str>,
    resource_name: Option<&'a str>,
    last_backup_time: Option<&'a DateTime>,
    points: Vec<(&'a str, &'a RecoveryPointByBackupVault)>,
}

fn resources(
    region: &str,
    listed: &[Listed],
    points: &[(String, RecoveryPointByBackupVault)],
) -> Vec<ProtectedResource> {
    let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
    for resource in listed {
        let Some(arn) = resource.resource_arn() else {
            continue;
        };
        let tally = tallies.entry(arn).or_default();
        tally.resource_type = resource.resource_type();
        tally.resource_name = resource.resource_name();
        tally.last_backup_time = resource.last_backup_time();
    }
    for (vault, point) in points {
        let Some(arn) = point.resource_arn() else {
            continue;
        };
        let tally = tallies.entry(arn).or_default();
        tally.resource_type = tally.resource_type.or(point.resource_type());
        tally.resource_name = tally.resource_name.or(point.resource_name());
        tally.points.push((vault, point));
    }
    tallies
        .into_iter()
        .map(|(arn, tally)| resource(arn, region, tally))
        .collect()
}

fn resource(arn: &str, region: &str, tally: Tally<'_>) -> ProtectedResource {
    let points = &tally.points;
    let last_backup = if points.is_empty() {
        tally.last_backup_time
    } else {
        points
            .iter()
            .filter(|(_, p)| restorable(p.status()))
            .filter_map(|(_, p)| p.creation_date())
            .max()
    };
    let vaults: BTreeSet<&str> = points.iter().map(|(vault, _)| *vault).collect();
    let plans: BTreeSet<String> = points
        .iter()
        .filter_map(|(_, p)| p.created_by().and_then(plan_name))
        .collect();
    let latest = points
        .iter()
        .max_by_key(|(_, p)| p.creation_date())
        .map(|(vault, p)| recovery_point(vault, p));

    ProtectedResource {
        arn: arn.to_string(),
        region: region.to_string(),
        resource_type: tally.resource_type.map(str::to_string),
        resource_name: tally.resource_name.map(str::to_string),
        coverage: AwsBackupCoverage {
            last_backup: last_backup.and_then(timestamp),
            recovery_points: points.len() as u64,
            recovery_point_bytes: points
                .iter()
                .filter_map(|(_, p)| size(p))
                .reduce(|a, b| a + b),
            vaults: vaults.into_iter().map(str::to_string).collect(),
        },
        oldest_backup: points
            .iter()
            .filter_map(|(_, p)| p.creation_date())
            .min()
            .and_then(timestamp),
        points_by_status: status_counts(points.iter().map(|(_, p)| *p)),
        unencrypted_points: points.iter().filter(|(_, p)| !p.is_encrypted()).count() as u64,
        backup_plans: plans.into_iter().collect(),
        latest,
    }
}

fn recovery_point(vault: &str, p: &RecoveryPointByBackupVault) -> RecoveryPoint {
    RecoveryPoint {
        arn: p.recovery_point_arn().unwrap_or_default().to_string(),
        vault: vault.to_string(),
        created: p.creation_date().and_then(timestamp),
        completed: p.completion_date().and_then(timestamp),
        status: p.status().map(|s| s.as_str().to_string()),
        status_message: p.status_message().map(str::to_string),
        size_bytes: size(p),
        encrypted: Some(p.is_encrypted()),
        encryption_key_arn: p.encryption_key_arn().map(str::to_string),
        delete_after_days: p.lifecycle().and_then(|l| l.delete_after_days()),
        cold_storage_after_days: p
            .lifecycle()
            .and_then(|l| l.move_to_cold_storage_after_days()),
        delete_at: p
            .calculated_lifecycle()
            .and_then(|l| l.delete_at())
            .and_then(timestamp),
        storage_class: None,
        backup_plan: p.created_by().and_then(plan_name),
        last_restore: p.last_restore_time().and_then(timestamp),
    }
}

fn apply_described(point: &mut RecoveryPoint, d: &DescribeRecoveryPointOutput) {
    if let Some(status) = d.status() {
        point.status = Some(status.as_str().to_string());
    }
    if let Some(message) = d.status_message() {
        point.status_message = Some(message.to_string());
    }
    point.encrypted = Some(d.is_encrypted());
    if let Some(key) = d.encryption_key_arn() {
        point.encryption_key_arn = Some(key.to_string());
    }
    if let Some(lifecycle) = d.lifecycle() {
        point.delete_after_days = lifecycle.delete_after_days();
        point.cold_storage_after_days = lifecycle.move_to_cold_storage_after_days();
    }
    if let Some(at) = d.calculated_lifecycle().and_then(|l| l.delete_at()) {
        point.delete_at = timestamp(at);
    }
    point.storage_class = d.storage_class().map(|s| s.as_str().to_string());
}

fn restorable(status: Option<&RecoveryPointStatus>) -> bool {
    !matches!(
        status,
        Some(
            RecoveryPointStatus::Creating
                | RecoveryPointStatus::Partial
                | RecoveryPointStatus::Expired
                | RecoveryPointStatus::Deleting
        )
    )
}

fn plan_name(creator: &RecoveryPointCreator) -> Option<String> {
    creator
        .backup_plan_name()
        .or(creator.backup_plan_id())
        .map(str::to_string)
}

fn size(p: &RecoveryPointByBackupVault) -> Option<u64> {
    p.backup_size_in_bytes().and_then(|b| u64::try_from(b).ok())
}

fn status_counts<'a>(
    points: impl Iterator<Item = &'a RecoveryPointByBackupVault>,
) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for point in points {
        let status = point.status().map_or("UNKNOWN", |s| s.as_str());
        *counts.entry(status.to_string()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use aws_sdk_backup::types::builders::RecoveryPointByBackupVaultBuilder;
    use aws_sdk_backup::types::{CalculatedLifecycle, Lifecycle, StorageClass, VaultType};

    use super::*;

    const VOL: &str = "arn:aws:ec2:ap-southeast-2:111111111111:volume/vol-a";
    const BUCKET: &str = "arn:aws:s3:::logs";
    const EFS: &str = "arn:aws:elasticfilesystem:ap-southeast-2:111111111111:file-system/fs-1";

    fn at(secs: i64) -> DateTime {
        DateTime::from_secs(secs)
    }

    fn draft(
        arn: &str,
        resource: &str,
        created: i64,
        status: RecoveryPointStatus,
        bytes: i64,
    ) -> RecoveryPointByBackupVaultBuilder {
        RecoveryPointByBackupVault::builder()
            .recovery_point_arn(arn)
            .resource_arn(resource)
            .resource_type("EBS")
            .creation_date(at(created))
            .status(status)
            .backup_size_in_bytes(bytes)
            .is_encrypted(true)
    }

    fn point(
        arn: &str,
        resource: &str,
        created: i64,
        status: RecoveryPointStatus,
        bytes: i64,
    ) -> RecoveryPointByBackupVault {
        draft(arn, resource, created, status, bytes).build()
    }

    fn listed(resource: &str, kind: &str, last: i64) -> Listed {
        Listed::builder()
            .resource_arn(resource)
            .resource_type(kind)
            .last_backup_time(at(last))
            .build()
    }

    #[test]
    fn folds_recovery_points_from_every_vault_into_one_row_per_resource() {
        let latest = draft("rp-3", VOL, 300, RecoveryPointStatus::Partial, 5)
            .lifecycle(Lifecycle::builder().delete_after_days(35).build())
            .calculated_lifecycle(CalculatedLifecycle::builder().delete_at(at(400)).build())
            .is_encrypted(false)
            .created_by(
                RecoveryPointCreator::builder()
                    .backup_plan_name("daily")
                    .build(),
            )
            .build();
        let points = vec![
            (
                "Default".to_string(),
                point("rp-1", VOL, 100, RecoveryPointStatus::Completed, 10),
            ),
            (
                "dr".to_string(),
                point("rp-2", VOL, 200, RecoveryPointStatus::Completed, 20),
            ),
            ("dr".to_string(), latest),
        ];
        let rows = resources("ap-southeast-2", &[listed(VOL, "EBS", 300)], &points);

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.arn, VOL);
        assert_eq!(row.resource_type.as_deref(), Some("EBS"));
        assert_eq!(row.coverage.recovery_points, 3);
        assert_eq!(row.coverage.recovery_point_bytes, Some(35));
        assert_eq!(row.coverage.vaults, vec!["Default", "dr"]);
        assert_eq!(row.coverage.last_backup, timestamp(&at(200)));
        assert_eq!(row.oldest_backup, timestamp(&at(100)));
        assert_eq!(row.unencrypted_points, 1);
        assert_eq!(row.backup_plans, vec!["daily"]);
        assert_eq!(
            row.points_by_status,
            BTreeMap::from([("COMPLETED".to_string(), 2), ("PARTIAL".to_string(), 1)])
        );
        let latest = row.latest.as_ref().unwrap();
        assert_eq!(latest.arn, "rp-3");
        assert_eq!(latest.vault, "dr");
        assert_eq!(latest.status.as_deref(), Some("PARTIAL"));
        assert_eq!(latest.delete_after_days, Some(35));
        assert_eq!(latest.delete_at, timestamp(&at(400)));
        assert_eq!(latest.encrypted, Some(false));
    }

    #[test]
    fn keeps_listed_resources_without_points_and_points_without_listing() {
        let points = vec![(
            "Default".to_string(),
            point("rp-1", EFS, 100, RecoveryPointStatus::Completed, 7),
        )];
        let rows = resources("ap-southeast-2", &[listed(BUCKET, "S3", 500)], &points);

        assert_eq!(rows.len(), 2);
        let efs = rows.iter().find(|r| r.arn == EFS).unwrap();
        assert_eq!(efs.coverage.recovery_points, 1);
        assert_eq!(efs.coverage.last_backup, timestamp(&at(100)));
        let bucket = rows.iter().find(|r| r.arn == BUCKET).unwrap();
        assert_eq!(bucket.resource_type.as_deref(), Some("S3"));
        assert_eq!(bucket.coverage.recovery_points, 0);
        assert_eq!(bucket.coverage.recovery_point_bytes, None);
        assert_eq!(bucket.coverage.last_backup, timestamp(&at(500)));
        assert!(bucket.latest.is_none());
    }

    #[test]
    fn vault_totals_summarise_its_recovery_points() {
        let listed = BackupVaultListMember::builder()
            .backup_vault_name("dr")
            .backup_vault_arn("arn:aws:backup:ap-southeast-2:111111111111:backup-vault:dr")
            .vault_type(VaultType::LogicallyAirGappedBackupVault)
            .number_of_recovery_points(3)
            .locked(true)
            .min_retention_days(7)
            .lock_date(at(50))
            .build();
        let mut vault = vault(&listed, "ap-southeast-2");
        vault_totals(
            &mut vault,
            &[
                point("rp-1", VOL, 100, RecoveryPointStatus::Completed, 10),
                point("rp-2", VOL, 300, RecoveryPointStatus::Completed, 20),
                point("rp-3", BUCKET, 200, RecoveryPointStatus::Expired, 1),
            ],
        );

        assert_eq!(vault.name, "dr");
        assert_eq!(vault.recovery_points, Some(3));
        assert_eq!(vault.locked, Some(true));
        assert_eq!(
            vault.vault_type.as_deref(),
            Some("LOGICALLY_AIR_GAPPED_BACKUP_VAULT")
        );
        assert_eq!(vault.min_retention_days, Some(7));
        assert_eq!(vault.lock_date, timestamp(&at(50)));
        assert_eq!(vault.recovery_point_bytes, Some(31));
        assert_eq!(vault.resources, Some(2));
        assert_eq!(vault.latest_recovery_point, timestamp(&at(300)));
        assert_eq!(vault.points_by_status.get("EXPIRED"), Some(&1));
    }

    #[test]
    fn deep_describe_refreshes_the_latest_point() {
        let mut latest = recovery_point(
            "Default",
            &point("rp-1", VOL, 100, RecoveryPointStatus::Creating, 1),
        );
        let described = DescribeRecoveryPointOutput::builder()
            .status(RecoveryPointStatus::Completed)
            .is_encrypted(true)
            .storage_class(StorageClass::Cold)
            .lifecycle(
                Lifecycle::builder()
                    .move_to_cold_storage_after_days(30)
                    .delete_after_days(120)
                    .build(),
            )
            .build();
        apply_described(&mut latest, &described);

        assert_eq!(latest.status.as_deref(), Some("COMPLETED"));
        assert_eq!(latest.storage_class.as_deref(), Some("COLD"));
        assert_eq!(latest.cold_storage_after_days, Some(30));
        assert_eq!(latest.delete_after_days, Some(120));
    }
}
