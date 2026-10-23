use aws_sdk_glue::types::{Database, Table, TableOptimizer, TableOptimizerType};
use aws_sdk_glue::Client;
use futures::future::join_all;

use cd_model::{DiscoveryError, GlueDatabase, GlueOptimizer, GlueTable, Protection, Tags};

use crate::{failure, timestamp, Scope, Section};

const SERVICE: &str = "glue";

const VERSION_PAGE_SIZE: i32 = 100;
const VERSION_PAGE_CAP: usize = 10;

const OPTIMIZERS: [TableOptimizerType; 3] = [
    TableOptimizerType::Compaction,
    TableOptimizerType::Retention,
    TableOptimizerType::OrphanFileDeletion,
];

pub(crate) const ACTIONS: &[&str] = &[
    "glue:GetDatabases",
    "glue:GetTableOptimizer",
    "glue:GetTableVersions",
    "glue:GetTables",
];

pub async fn discover(scope: &Scope<'_>) -> Section<GlueDatabase> {
    let client = &Client::new(scope.sdk());
    let listed = match list_databases(scope, client).await {
        Ok(listed) => listed,
        Err(err) => return Section::failed(err),
    };

    let mut databases = Vec::new();
    let mut errors = Vec::new();
    for (database, database_errors) in
        join_all(listed.into_iter().map(|d| database(scope, client, d))).await
    {
        errors.extend(database_errors);
        databases.extend(database);
    }
    Section::ok(databases, errors)
}

async fn list_databases(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<GlueDatabase>, DiscoveryError> {
    let mut pages = client.get_databases().into_paginator().send();
    let mut databases = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "GetDatabases", None, e))?;
        databases.extend(page.database_list().iter().map(|d| {
            database_from(
                scope.arn(SERVICE, &format!("database/{}", d.name())),
                &scope.region,
                d,
            )
        }));
    }
    Ok(databases)
}

async fn database(
    scope: &Scope<'_>,
    client: &Client,
    mut database: GlueDatabase,
) -> (Option<GlueDatabase>, Vec<DiscoveryError>) {
    let tables = match iceberg_tables(scope, client, &database.name).await {
        Ok(tables) => tables,
        Err(err) => return (None, vec![err]),
    };
    if tables.is_empty() {
        return (None, Vec::new());
    }
    let mut errors = Vec::new();
    for (table, table_errors) in join_all(
        tables
            .into_iter()
            .map(|t| table_detail(scope, client, &database.name, t)),
    )
    .await
    {
        errors.extend(table_errors);
        database.iceberg_tables.push(table);
    }
    (Some(database), errors)
}

async fn iceberg_tables(
    scope: &Scope<'_>,
    client: &Client,
    database: &str,
) -> Result<Vec<GlueTable>, DiscoveryError> {
    let mut pages = client
        .get_tables()
        .database_name(database)
        .into_paginator()
        .send();
    let mut tables = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "GetTables", Some(database), e))?;
        tables.extend(page.table_list().iter().filter(|t| is_iceberg(t)).map(|t| {
            table_from(
                scope.arn(SERVICE, &format!("table/{database}/{}", t.name())),
                t,
            )
        }));
    }
    Ok(tables)
}

async fn table_detail(
    scope: &Scope<'_>,
    client: &Client,
    database: &str,
    mut table: GlueTable,
) -> (GlueTable, Vec<DiscoveryError>) {
    let mut errors = Vec::new();
    let (versions, optimizers) = tokio::join!(
        count_versions(scope, client, database, &table.name),
        async {
            if scope.ctx.deep() {
                Some(optimizers(scope, client, database, &table.name).await)
            } else {
                None
            }
        },
    );
    match versions {
        Ok((count, capped)) => {
            table.version_count = Some(count);
            table.version_count_capped = capped;
        }
        Err(err) => errors.push(err),
    }
    if let Some((optimizers, optimizer_errors)) = optimizers {
        errors.extend(optimizer_errors);
        table.optimizers = Some(optimizers);
    }
    (table, errors)
}

async fn count_versions(
    scope: &Scope<'_>,
    client: &Client,
    database: &str,
    table: &str,
) -> Result<(u64, bool), DiscoveryError> {
    let mut count = 0;
    let mut token: Option<String> = None;
    for _ in 0..VERSION_PAGE_CAP {
        let output = scope
            .call(
                client
                    .get_table_versions()
                    .database_name(database)
                    .table_name(table)
                    .max_results(VERSION_PAGE_SIZE)
                    .set_next_token(token.clone())
                    .send(),
            )
            .await
            .map_err(|e| {
                failure(
                    SERVICE,
                    "GetTableVersions",
                    Some(format!("{database}.{table}").as_str()),
                    e,
                )
            })?;
        count += output.table_versions().len() as u64;
        match output.next_token().filter(|next| !next.is_empty()) {
            Some(next) if token.as_deref() != Some(next) => token = Some(next.to_string()),
            _ => return Ok((count, false)),
        }
    }
    Ok((count, true))
}

async fn optimizers(
    scope: &Scope<'_>,
    client: &Client,
    database: &str,
    table: &str,
) -> (Vec<GlueOptimizer>, Vec<DiscoveryError>) {
    let results = join_all(OPTIMIZERS.iter().map(|kind| {
        scope.call(
            client
                .get_table_optimizer()
                .catalog_id(&scope.ctx.account)
                .database_name(database)
                .table_name(table)
                .r#type(kind.clone())
                .send(),
        )
    }))
    .await;
    let mut optimizers = Vec::new();
    let mut errors = Vec::new();
    for (kind, result) in OPTIMIZERS.iter().zip(results) {
        match result {
            Ok(out) => optimizers.push(optimizer_from(kind, out.table_optimizer())),
            Err(err)
                if err
                    .as_service_error()
                    .is_some_and(|e| e.is_entity_not_found_exception()) => {}
            Err(err) => errors.push(failure(
                SERVICE,
                "GetTableOptimizer",
                Some(format!("{database}.{table} ({})", kind.as_str()).as_str()),
                err,
            )),
        }
    }
    (optimizers, errors)
}

fn database_from(arn: String, region: &str, database: &Database) -> GlueDatabase {
    GlueDatabase {
        arn,
        name: database.name().to_string(),
        region: region.to_string(),
        description: database
            .description()
            .filter(|d| !d.is_empty())
            .map(str::to_string),
        location_uri: database
            .location_uri()
            .filter(|l| !l.is_empty())
            .map(str::to_string),
        created_at: database.create_time().and_then(timestamp),
        iceberg_tables: Vec::new(),
        tags: Tags::new(),
        protection: Protection::default(),
    }
}

fn table_from(arn: String, table: &Table) -> GlueTable {
    GlueTable {
        arn,
        name: table.name().to_string(),
        location: table
            .storage_descriptor()
            .and_then(|s| s.location())
            .map(str::to_string),
        table_type: table.table_type().map(str::to_string),
        metadata_location: parameter(table, "metadata_location").map(str::to_string),
        previous_metadata_location: parameter(table, "previous_metadata_location")
            .map(str::to_string),
        created_at: table.create_time().and_then(timestamp),
        updated_at: table.update_time().and_then(timestamp),
        version_id: table.version_id().map(str::to_string),
        version_count: None,
        version_count_capped: false,
        optimizers: None,
    }
}

fn optimizer_from(kind: &TableOptimizerType, optimizer: Option<&TableOptimizer>) -> GlueOptimizer {
    let configuration = optimizer.and_then(|o| o.configuration());
    let last_run = optimizer.and_then(|o| o.last_run());
    let detail = match kind {
        TableOptimizerType::Compaction => configuration
            .and_then(|c| c.compaction_configuration())
            .and_then(|c| c.iceberg_configuration())
            .and_then(|c| c.strategy())
            .map(|s| format!("strategy {}", s.as_str())),
        TableOptimizerType::Retention => configuration
            .and_then(|c| c.retention_configuration())
            .and_then(|c| c.iceberg_configuration())
            .map(|c| {
                let mut parts = Vec::new();
                if let Some(days) = c.snapshot_retention_period_in_days() {
                    parts.push(format!("snapshots kept {days} days"));
                }
                if let Some(n) = c.number_of_snapshots_to_retain() {
                    parts.push(format!("at least {n} snapshots"));
                }
                parts.join(", ")
            })
            .filter(|d| !d.is_empty()),
        TableOptimizerType::OrphanFileDeletion => configuration
            .and_then(|c| c.orphan_file_deletion_configuration())
            .and_then(|c| c.iceberg_configuration())
            .and_then(|c| c.orphan_file_retention_period_in_days())
            .map(|days| format!("orphan files kept {days} days")),
        _ => None,
    };
    GlueOptimizer {
        kind: kind.as_str().to_string(),
        enabled: configuration.and_then(|c| c.enabled()),
        last_run: last_run
            .and_then(|r| r.end_timestamp().or(r.start_timestamp()))
            .and_then(timestamp),
        last_run_status: last_run
            .and_then(|r| r.event_type())
            .map(|e| e.as_str().to_string()),
        detail,
    }
}

fn is_iceberg(table: &Table) -> bool {
    parameter(table, "table_type").is_some_and(|t| t.eq_ignore_ascii_case("iceberg"))
}

fn parameter<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    table
        .parameters()
        .and_then(|p| p.get(key))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_glue::types::{
        CompactionConfiguration, CompactionStrategy, IcebergCompactionConfiguration,
        IcebergRetentionConfiguration, RetentionConfiguration, StorageDescriptor,
        TableOptimizerConfiguration, TableOptimizerEventType, TableOptimizerRun,
    };
    use aws_smithy_types::DateTime;

    fn iceberg_table() -> Table {
        Table::builder()
            .name("events")
            .table_type("EXTERNAL_TABLE")
            .parameters("table_type", "ICEBERG")
            .parameters("metadata_location", "s3://lake/events/metadata/2.json")
            .parameters(
                "previous_metadata_location",
                "s3://lake/events/metadata/1.json",
            )
            .storage_descriptor(
                StorageDescriptor::builder()
                    .location("s3://lake/events")
                    .build(),
            )
            .create_time(DateTime::from_secs(1_792_713_600))
            .update_time(DateTime::from_secs(1_792_800_000))
            .version_id("7")
            .build()
            .unwrap()
    }

    #[test]
    fn recognises_iceberg_tables_by_parameter() {
        assert!(is_iceberg(&iceberg_table()));
        let hive = Table::builder()
            .name("raw")
            .parameters("table_type", "hive")
            .build()
            .unwrap();
        assert!(!is_iceberg(&hive));
        assert!(!is_iceberg(&Table::builder().name("bare").build().unwrap()));
    }

    #[test]
    fn maps_database_and_iceberg_table() {
        let database = database_from(
            "arn:aws:glue:ap-southeast-2:1:database/analytics".into(),
            "ap-southeast-2",
            &Database::builder()
                .name("analytics")
                .description("")
                .location_uri("s3://lake/")
                .create_time(DateTime::from_secs(1_792_713_600))
                .build()
                .unwrap(),
        );
        assert_eq!(database.name, "analytics");
        assert_eq!(database.description, None);
        assert_eq!(database.location_uri.as_deref(), Some("s3://lake/"));
        assert_eq!(database.created_at.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert!(database.tags.is_empty());

        let table = table_from("arn:t".into(), &iceberg_table());
        assert_eq!(
            table,
            GlueTable {
                name: "events".into(),
                arn: "arn:t".into(),
                location: Some("s3://lake/events".into()),
                table_type: Some("EXTERNAL_TABLE".into()),
                metadata_location: Some("s3://lake/events/metadata/2.json".into()),
                previous_metadata_location: Some("s3://lake/events/metadata/1.json".into()),
                created_at: Some("2026-10-23T00:00:00Z".into()),
                updated_at: Some("2026-10-24T00:00:00Z".into()),
                version_id: Some("7".into()),
                version_count: None,
                version_count_capped: false,
                optimizers: None,
            }
        );
    }

    #[test]
    fn optimizer_maps_state_and_settings() {
        let retention = TableOptimizer::builder()
            .r#type(TableOptimizerType::Retention)
            .configuration(
                TableOptimizerConfiguration::builder()
                    .enabled(true)
                    .retention_configuration(
                        RetentionConfiguration::builder()
                            .iceberg_configuration(
                                IcebergRetentionConfiguration::builder()
                                    .snapshot_retention_period_in_days(5)
                                    .number_of_snapshots_to_retain(1)
                                    .build(),
                            )
                            .build(),
                    )
                    .build(),
            )
            .last_run(
                TableOptimizerRun::builder()
                    .event_type(TableOptimizerEventType::Completed)
                    .start_timestamp(DateTime::from_secs(1_792_000_000))
                    .end_timestamp(DateTime::from_secs(1_792_713_600))
                    .build(),
            )
            .build();
        assert_eq!(
            optimizer_from(&TableOptimizerType::Retention, Some(&retention)),
            GlueOptimizer {
                kind: "retention".into(),
                enabled: Some(true),
                last_run: Some("2026-10-23T00:00:00Z".into()),
                last_run_status: Some("completed".into()),
                detail: Some("snapshots kept 5 days, at least 1 snapshots".into()),
            }
        );

        let compaction = TableOptimizer::builder()
            .configuration(
                TableOptimizerConfiguration::builder()
                    .enabled(false)
                    .compaction_configuration(
                        CompactionConfiguration::builder()
                            .iceberg_configuration(
                                IcebergCompactionConfiguration::builder()
                                    .strategy(CompactionStrategy::Binpack)
                                    .build(),
                            )
                            .build(),
                    )
                    .build(),
            )
            .build();
        let mapped = optimizer_from(&TableOptimizerType::Compaction, Some(&compaction));
        assert_eq!(mapped.kind, "compaction");
        assert_eq!(mapped.enabled, Some(false));
        assert_eq!(mapped.detail.as_deref(), Some("strategy binpack"));
        assert_eq!(mapped.last_run, None);

        let empty = optimizer_from(&TableOptimizerType::OrphanFileDeletion, None);
        assert_eq!(empty.kind, "orphan_file_deletion");
        assert_eq!(empty.enabled, None);
        assert_eq!(empty.detail, None);
    }
}
