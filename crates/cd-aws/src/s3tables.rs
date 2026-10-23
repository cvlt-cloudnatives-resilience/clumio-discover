use std::collections::HashMap;

use aws_sdk_s3tables::operation::get_table::GetTableOutput;
use aws_sdk_s3tables::types::{
    NamespaceSummary, TableBucketMaintenanceConfigurationValue, TableBucketMaintenanceSettings,
    TableBucketMaintenanceType, TableBucketSummary, TableMaintenanceConfigurationValue,
    TableMaintenanceSettings, TableMaintenanceType, TableSummary,
};
use aws_sdk_s3tables::Client;
use futures::future::join_all;

use cd_model::{
    DiscoveryError, Protection, S3Table, S3TableCompaction, S3TableFileRemoval, S3TableNamespace,
    S3TableSnapshotManagement, TableBucket, Tags,
};

use crate::{failure, timestamp, to_tags, Scope, Section};

const SERVICE: &str = "s3tables";

pub(crate) const ACTIONS: &[&str] = &[
    "s3tables:GetTable",
    "s3tables:GetTableBucketMaintenanceConfiguration",
    "s3tables:GetTableMaintenanceConfiguration",
    "s3tables:ListNamespaces",
    "s3tables:ListTableBuckets",
    "s3tables:ListTables",
    "s3tables:ListTagsForResource",
];

pub async fn discover(scope: &Scope<'_>) -> Section<TableBucket> {
    let client = &Client::new(scope.sdk());
    let listed = match list_table_buckets(scope, client).await {
        Ok(listed) => listed,
        Err(err) => return Section::failed(err),
    };

    let mut buckets = Vec::new();
    let mut errors = Vec::new();
    for (bucket, bucket_errors) in
        join_all(listed.into_iter().map(|b| bucket(scope, client, b))).await
    {
        errors.extend(bucket_errors);
        buckets.extend(bucket);
    }
    Section::ok(buckets, errors)
}

async fn list_table_buckets(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<TableBucket>, DiscoveryError> {
    let mut pages = client.list_table_buckets().into_paginator().send();
    let mut buckets = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListTableBuckets", None, e))?;
        buckets.extend(
            page.table_buckets()
                .iter()
                .map(|b| bucket_from(b, &scope.region)),
        );
    }
    Ok(buckets)
}

async fn bucket(
    scope: &Scope<'_>,
    client: &Client,
    mut bucket: TableBucket,
) -> (Option<TableBucket>, Vec<DiscoveryError>) {
    let mut errors = Vec::new();
    bucket.tags = tags(scope, client, &bucket.arn)
        .await
        .unwrap_or_else(|err| {
            errors.push(err);
            Tags::new()
        });
    if !scope.filters().admits(&bucket.tags) {
        return (None, errors);
    }

    let arn = bucket.arn.as_str();
    let (maintenance, namespaces, tables) = tokio::join!(
        scope.call(
            client
                .get_table_bucket_maintenance_configuration()
                .table_bucket_arn(arn)
                .send()
        ),
        list_namespaces(scope, client, arn),
        list_tables(scope, client, arn),
    );
    match maintenance {
        Ok(out) => bucket.unreferenced_file_removal = file_removal(out.configuration()),
        Err(err)
            if err
                .as_service_error()
                .is_some_and(|e| e.is_not_found_exception()) => {}
        Err(err) => errors.push(failure(
            SERVICE,
            "GetTableBucketMaintenanceConfiguration",
            Some(arn),
            err,
        )),
    }
    match namespaces {
        Ok(namespaces) => bucket.namespaces = namespaces,
        Err(err) => errors.push(err),
    }
    let summaries = tables.unwrap_or_else(|err| {
        errors.push(err);
        Vec::new()
    });

    let detailed = join_all(summaries.iter().map(|t| table(scope, client, arn, t))).await;
    for (table, table_errors) in detailed {
        errors.extend(table_errors);
        bucket.tables.push(table);
    }
    (Some(bucket), errors)
}

async fn list_namespaces(
    scope: &Scope<'_>,
    client: &Client,
    bucket_arn: &str,
) -> Result<Vec<S3TableNamespace>, DiscoveryError> {
    let mut pages = client
        .list_namespaces()
        .table_bucket_arn(bucket_arn)
        .into_paginator()
        .send();
    let mut namespaces = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListNamespaces", Some(bucket_arn), e))?;
        namespaces.extend(page.namespaces().iter().map(namespace_from));
    }
    Ok(namespaces)
}

async fn list_tables(
    scope: &Scope<'_>,
    client: &Client,
    bucket_arn: &str,
) -> Result<Vec<TableSummary>, DiscoveryError> {
    let mut pages = client
        .list_tables()
        .table_bucket_arn(bucket_arn)
        .into_paginator()
        .send();
    let mut tables = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListTables", Some(bucket_arn), e))?;
        tables.extend(page.tables().iter().cloned());
    }
    Ok(tables)
}

async fn table(
    scope: &Scope<'_>,
    client: &Client,
    bucket_arn: &str,
    summary: &TableSummary,
) -> (S3Table, Vec<DiscoveryError>) {
    let mut table = table_from(summary);
    let mut errors = Vec::new();
    let (got, maintenance) = tokio::join!(
        scope.call(
            client
                .get_table()
                .table_bucket_arn(bucket_arn)
                .namespace(&table.namespace)
                .name(&table.name)
                .send()
        ),
        scope.call(
            client
                .get_table_maintenance_configuration()
                .table_bucket_arn(bucket_arn)
                .namespace(&table.namespace)
                .name(&table.name)
                .send()
        ),
    );
    match got {
        Ok(out) => apply_get_table(&mut table, &out),
        Err(err) => errors.push(failure(SERVICE, "GetTable", Some(&table.arn), err)),
    }
    match maintenance {
        Ok(out) => apply_table_maintenance(&mut table, out.configuration()),
        Err(err)
            if err
                .as_service_error()
                .is_some_and(|e| e.is_not_found_exception()) => {}
        Err(err) => errors.push(failure(
            SERVICE,
            "GetTableMaintenanceConfiguration",
            Some(&table.arn),
            err,
        )),
    }
    (table, errors)
}

async fn tags(scope: &Scope<'_>, client: &Client, arn: &str) -> Result<Tags, DiscoveryError> {
    let output = scope
        .call(client.list_tags_for_resource().resource_arn(arn).send())
        .await
        .map_err(|e| failure(SERVICE, "ListTagsForResource", Some(arn), e))?;
    Ok(to_tags(
        output
            .tags()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.as_str(), v.as_str())),
    ))
}

fn bucket_from(summary: &TableBucketSummary, region: &str) -> TableBucket {
    TableBucket {
        name: summary.name().to_string(),
        arn: summary.arn().to_string(),
        region: region.to_string(),
        created_at: timestamp(summary.created_at()),
        bucket_type: summary.r#type().map(|t| t.as_str().to_string()),
        owner_account_id: Some(summary.owner_account_id().to_string()),
        unreferenced_file_removal: None,
        namespaces: Vec::new(),
        tables: Vec::new(),
        tags: Tags::new(),
        protection: Protection::default(),
    }
}

fn namespace_from(summary: &NamespaceSummary) -> S3TableNamespace {
    S3TableNamespace {
        name: summary.namespace().join("."),
        created_at: timestamp(summary.created_at()),
        created_by: Some(summary.created_by().to_string()),
        owner_account_id: Some(summary.owner_account_id().to_string()),
    }
}

fn table_from(summary: &TableSummary) -> S3Table {
    S3Table {
        namespace: summary.namespace().join("."),
        name: summary.name().to_string(),
        arn: summary.table_arn().to_string(),
        table_type: Some(summary.r#type().as_str().to_string()),
        created_at: timestamp(summary.created_at()),
        modified_at: timestamp(summary.modified_at()),
        managed_by_service: summary.managed_by_service().map(str::to_string),
        ..S3Table::default()
    }
}

fn apply_get_table(table: &mut S3Table, out: &GetTableOutput) {
    table.table_type = Some(out.r#type().as_str().to_string());
    table.format = Some(out.format().as_str().to_string());
    table.created_at = timestamp(out.created_at());
    table.modified_at = timestamp(out.modified_at());
    table.managed_by_service = out.managed_by_service().map(str::to_string);
    table.metadata_location = out.metadata_location().map(str::to_string);
    table.warehouse_location =
        Some(out.warehouse_location().to_string()).filter(|location| !location.is_empty());
}

fn file_removal(
    configuration: &HashMap<TableBucketMaintenanceType, TableBucketMaintenanceConfigurationValue>,
) -> Option<S3TableFileRemoval> {
    let value = configuration.get(&TableBucketMaintenanceType::IcebergUnreferencedFileRemoval)?;
    let settings = match value.settings() {
        Some(TableBucketMaintenanceSettings::IcebergUnreferencedFileRemoval(s)) => Some(s),
        _ => None,
    };
    Some(S3TableFileRemoval {
        status: value.status().map(|s| s.as_str().to_string()),
        unreferenced_days: settings.and_then(|s| s.unreferenced_days()),
        non_current_days: settings.and_then(|s| s.non_current_days()),
    })
}

fn apply_table_maintenance(
    table: &mut S3Table,
    configuration: &HashMap<TableMaintenanceType, TableMaintenanceConfigurationValue>,
) {
    if let Some(value) = configuration.get(&TableMaintenanceType::IcebergCompaction) {
        let settings = match value.settings() {
            Some(TableMaintenanceSettings::IcebergCompaction(s)) => Some(s),
            _ => None,
        };
        table.compaction = Some(S3TableCompaction {
            status: value.status().map(|s| s.as_str().to_string()),
            target_file_size_mb: settings.and_then(|s| s.target_file_size_mb()),
            strategy: settings
                .and_then(|s| s.strategy())
                .map(|s| s.as_str().to_string()),
        });
    }
    if let Some(value) = configuration.get(&TableMaintenanceType::IcebergSnapshotManagement) {
        let settings = match value.settings() {
            Some(TableMaintenanceSettings::IcebergSnapshotManagement(s)) => Some(s),
            _ => None,
        };
        table.snapshot_management = Some(S3TableSnapshotManagement {
            status: value.status().map(|s| s.as_str().to_string()),
            min_snapshots_to_keep: settings.and_then(|s| s.min_snapshots_to_keep()),
            max_snapshot_age_hours: settings.and_then(|s| s.max_snapshot_age_hours()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3tables::types::{
        IcebergCompactionSettings, IcebergCompactionStrategy, IcebergSnapshotManagementSettings,
        IcebergUnreferencedFileRemovalSettings, MaintenanceStatus, OpenTableFormat,
        TableBucketType, TableType,
    };
    use aws_smithy_types::DateTime;

    const BUCKET: &str = "arn:aws:s3tables:ap-southeast-2:1:bucket/lake";
    const TABLE: &str = "arn:aws:s3tables:ap-southeast-2:1:bucket/lake/table/t-1";

    fn at(secs: i64) -> DateTime {
        DateTime::from_secs(secs)
    }

    fn summary() -> TableSummary {
        TableSummary::builder()
            .namespace("sales")
            .name("orders")
            .r#type(TableType::Customer)
            .table_arn(TABLE)
            .created_at(at(1_792_713_600))
            .modified_at(at(1_792_713_600))
            .build()
            .unwrap()
    }

    #[test]
    fn maps_bucket_namespace_and_table_summaries() {
        let bucket = bucket_from(
            &TableBucketSummary::builder()
                .arn(BUCKET)
                .name("lake")
                .owner_account_id("1")
                .created_at(at(1_792_713_600))
                .r#type(TableBucketType::Customer)
                .build()
                .unwrap(),
            "ap-southeast-2",
        );
        assert_eq!(bucket.name, "lake");
        assert_eq!(bucket.created_at.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(bucket.bucket_type.as_deref(), Some("customer"));
        assert_eq!(bucket.owner_account_id.as_deref(), Some("1"));

        let namespace = namespace_from(
            &NamespaceSummary::builder()
                .namespace("sales")
                .created_at(at(1_792_713_600))
                .created_by("1")
                .owner_account_id("1")
                .build()
                .unwrap(),
        );
        assert_eq!(namespace.name, "sales");
        assert_eq!(namespace.created_by.as_deref(), Some("1"));

        let table = table_from(&summary());
        assert_eq!(table.qualified_name(), "sales.orders");
        assert_eq!(table.table_type.as_deref(), Some("customer"));
        assert_eq!(table.format, None);
    }

    #[test]
    fn get_table_adds_format_and_locations() {
        let mut table = table_from(&summary());
        let out = GetTableOutput::builder()
            .name("orders")
            .r#type(TableType::Customer)
            .table_arn(TABLE)
            .namespace("sales")
            .version_token("v")
            .metadata_location("s3://w/metadata/00001.metadata.json")
            .warehouse_location("s3://w")
            .created_at(at(1_792_713_600))
            .created_by("1")
            .modified_at(at(1_792_800_000))
            .modified_by("1")
            .owner_account_id("1")
            .format(OpenTableFormat::Iceberg)
            .managed_by_service("glue")
            .build()
            .unwrap();
        apply_get_table(&mut table, &out);
        assert_eq!(table.format.as_deref(), Some("ICEBERG"));
        assert_eq!(
            table.metadata_location.as_deref(),
            Some("s3://w/metadata/00001.metadata.json")
        );
        assert_eq!(table.warehouse_location.as_deref(), Some("s3://w"));
        assert_eq!(table.modified_at.as_deref(), Some("2026-10-24T00:00:00Z"));
        assert_eq!(table.managed_by_service.as_deref(), Some("glue"));
    }

    #[test]
    fn maps_maintenance_configuration() {
        let bucket_config = HashMap::from([(
            TableBucketMaintenanceType::IcebergUnreferencedFileRemoval,
            TableBucketMaintenanceConfigurationValue::builder()
                .status(MaintenanceStatus::Enabled)
                .settings(
                    TableBucketMaintenanceSettings::IcebergUnreferencedFileRemoval(
                        IcebergUnreferencedFileRemovalSettings::builder()
                            .unreferenced_days(3)
                            .non_current_days(10)
                            .build(),
                    ),
                )
                .build(),
        )]);
        assert_eq!(
            file_removal(&bucket_config),
            Some(S3TableFileRemoval {
                status: Some("enabled".into()),
                unreferenced_days: Some(3),
                non_current_days: Some(10),
            })
        );
        assert_eq!(file_removal(&HashMap::new()), None);

        let mut table = table_from(&summary());
        apply_table_maintenance(
            &mut table,
            &HashMap::from([
                (
                    TableMaintenanceType::IcebergCompaction,
                    TableMaintenanceConfigurationValue::builder()
                        .status(MaintenanceStatus::Enabled)
                        .settings(TableMaintenanceSettings::IcebergCompaction(
                            IcebergCompactionSettings::builder()
                                .target_file_size_mb(512)
                                .strategy(IcebergCompactionStrategy::Binpack)
                                .build(),
                        ))
                        .build(),
                ),
                (
                    TableMaintenanceType::IcebergSnapshotManagement,
                    TableMaintenanceConfigurationValue::builder()
                        .status(MaintenanceStatus::Disabled)
                        .settings(TableMaintenanceSettings::IcebergSnapshotManagement(
                            IcebergSnapshotManagementSettings::builder()
                                .min_snapshots_to_keep(1)
                                .max_snapshot_age_hours(120)
                                .build(),
                        ))
                        .build(),
                ),
            ]),
        );
        assert_eq!(
            table.compaction,
            Some(S3TableCompaction {
                status: Some("enabled".into()),
                target_file_size_mb: Some(512),
                strategy: Some("binpack".into()),
            })
        );
        assert_eq!(
            table.snapshot_management,
            Some(S3TableSnapshotManagement {
                status: Some("disabled".into()),
                min_snapshots_to_keep: Some(1),
                max_snapshot_age_hours: Some(120),
            })
        );
    }
}
