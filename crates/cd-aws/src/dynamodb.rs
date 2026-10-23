use std::collections::{BTreeSet, HashMap};

use aws_sdk_applicationautoscaling::types::{
    ScalableDimension, ScalableTarget, ScalingPolicy, ServiceNamespace,
};
use aws_sdk_dynamodb::types::{
    AutoScalingSettingsDescription, BackupStatus, BackupSummary, BackupType, BackupTypeFilter,
    ContinuousBackupsDescription, GlobalTableDescription, ReplicaSettingsDescription,
    TableAutoScalingDescription, TableDescription,
};
use aws_sdk_dynamodb::Client;
use futures::future::join_all;

use cd_model::{
    DiscoveryError, DynamoAutoscaling, DynamoBackups, DynamoCapacityRange, DynamoReplica,
    DynamoStream, DynamoTable, NativeBackup, NativeKind, Protection, Tags,
    LEGACY_GLOBAL_TABLE_VERSION,
};

use crate::{failure, timestamp, to_tags, Scope, Section};

const SERVICE: &str = "dynamodb";
const AUTOSCALING: &str = "application-autoscaling";
const KMS: &str = "kms";

pub(crate) const ACTIONS: &[&str] = &[
    "application-autoscaling:DescribeScalableTargets",
    "application-autoscaling:DescribeScalingPolicies",
    "dynamodb:DescribeBackup",
    "dynamodb:DescribeContinuousBackups",
    "dynamodb:DescribeContributorInsights",
    "dynamodb:DescribeGlobalTable",
    "dynamodb:DescribeGlobalTableSettings",
    "dynamodb:DescribeTable",
    "dynamodb:DescribeTableReplicaAutoScaling",
    "dynamodb:DescribeTimeToLive",
    "dynamodb:GetResourcePolicy",
    "dynamodb:ListBackups",
    "dynamodb:ListGlobalTables",
    "dynamodb:ListTables",
    "dynamodb:ListTagsOfResource",
    "kms:DescribeKey",
];

pub async fn discover(scope: &Scope<'_>) -> Section<DynamoTable> {
    let client = &Client::new(scope.sdk());
    let names = match list_tables(scope, client).await {
        Ok(names) => names,
        Err(err) => return Section::failed(err),
    };
    if names.is_empty() {
        return Section::ok(Vec::new(), Vec::new());
    }

    let (listed, legacy, (scaling, mut errors)) = tokio::join!(
        join_all(names.iter().map(|name| table(scope, client, name))),
        legacy_global_table_names(scope, client),
        autoscaling(scope),
    );
    let mut tables = Vec::new();
    for (table, table_errors) in listed {
        errors.extend(table_errors);
        tables.extend(table);
    }
    for table in &mut tables {
        table.autoscaling = scaling.get(&table.name).cloned();
    }

    let legacy = legacy.unwrap_or_else(|err| {
        errors.push(err);
        Vec::new()
    });
    let legacy: Vec<&String> = legacy
        .iter()
        .filter(|name| tables.iter().any(|t| &t.name == *name))
        .collect();
    let keys: BTreeSet<String> = tables
        .iter()
        .filter_map(|t| t.kms_key_arn.clone())
        .collect();
    let kms = &aws_sdk_kms::Client::new(scope.sdk());
    let (globals, managers) = tokio::join!(
        join_all(
            legacy
                .iter()
                .map(|name| legacy_global_table(scope, client, name))
        ),
        join_all(keys.iter().map(|arn| key_manager(scope, kms, arn))),
    );

    for (name, (replicas, global_errors)) in legacy.iter().zip(globals) {
        errors.extend(global_errors);
        if let Some(table) = tables.iter_mut().find(|t| &t.name == *name) {
            table.global_table_version = Some(LEGACY_GLOBAL_TABLE_VERSION.to_string());
            table.replicas = replicas;
        }
    }
    let mut manager_of = HashMap::new();
    for (arn, manager) in keys.iter().zip(managers) {
        match manager {
            Ok(manager) => {
                manager_of.insert(arn, manager);
            }
            Err(err) => errors.push(err),
        }
    }
    for table in &mut tables {
        if let Some(arn) = &table.kms_key_arn {
            table.kms_key_manager = manager_of.get(arn).cloned().flatten();
        }
        table.protection.native = native(table);
    }

    Section::ok(tables, errors)
}

async fn list_tables(scope: &Scope<'_>, client: &Client) -> Result<Vec<String>, DiscoveryError> {
    let mut pages = client.list_tables().into_paginator().send();
    let mut names = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListTables", None, e))?;
        names.extend(page.table_names().iter().cloned());
    }
    Ok(names)
}

async fn table(
    scope: &Scope<'_>,
    client: &Client,
    name: &str,
) -> (Option<DynamoTable>, Vec<DiscoveryError>) {
    let mut errors = Vec::new();
    let arn = scope.arn(SERVICE, &format!("table/{name}"));
    let tags = tags(scope, client, &arn).await.unwrap_or_else(|err| {
        errors.push(err);
        Tags::new()
    });
    if !scope.filters().admits(&tags) {
        return (None, errors);
    }

    let (described, continuous, backups, ttl, insights, policy) = tokio::join!(
        scope.call(client.describe_table().table_name(name).send()),
        scope.call(client.describe_continuous_backups().table_name(name).send()),
        list_backups(scope, client, name),
        scope.call(client.describe_time_to_live().table_name(name).send()),
        scope.call(
            client
                .describe_contributor_insights()
                .table_name(name)
                .send()
        ),
        scope.call(client.get_resource_policy().resource_arn(&arn).send()),
    );

    let mut table = match described {
        Ok(out) => from_description(name, &arn, &scope.region, out.table()),
        Err(err) => {
            errors.push(failure(SERVICE, "DescribeTable", Some(name), err));
            from_description(name, &arn, &scope.region, None)
        }
    };
    table.tags = tags;

    match continuous {
        Ok(out) => {
            if let Some(description) = out.continuous_backups_description() {
                apply_pitr(&mut table, description);
            }
        }
        Err(err) => errors.push(failure(
            SERVICE,
            "DescribeContinuousBackups",
            Some(name),
            err,
        )),
    }
    match ttl {
        Ok(out) => {
            let ttl = out.time_to_live_description();
            table.ttl_status = ttl
                .and_then(|t| t.time_to_live_status())
                .map(|s| s.as_str().to_string());
            table.ttl_attribute = ttl.and_then(|t| t.attribute_name()).map(str::to_string);
        }
        Err(err) => errors.push(failure(SERVICE, "DescribeTimeToLive", Some(name), err)),
    }
    match insights {
        Ok(out) => {
            table.contributor_insights = out
                .contributor_insights_status()
                .map(|s| s.as_str().to_string());
        }
        Err(err) => errors.push(failure(
            SERVICE,
            "DescribeContributorInsights",
            Some(name),
            err,
        )),
    }
    match policy {
        Ok(out) => table.resource_policy = Some(out.policy().is_some()),
        Err(err)
            if err
                .as_service_error()
                .is_some_and(|e| e.is_policy_not_found_exception()) =>
        {
            table.resource_policy = Some(false)
        }
        Err(err) => errors.push(failure(SERVICE, "GetResourcePolicy", Some(name), err)),
    }

    if !table.replicas.is_empty() {
        match scope
            .call(
                client
                    .describe_table_replica_auto_scaling()
                    .table_name(name)
                    .send(),
            )
            .await
        {
            Ok(out) => {
                if let Some(description) = out.table_auto_scaling_description() {
                    apply_replica_autoscaling(&mut table.replicas, description);
                }
            }
            Err(err) => errors.push(failure(
                SERVICE,
                "DescribeTableReplicaAutoScaling",
                Some(name),
                err,
            )),
        }
    }

    match backups {
        Ok(summaries) => {
            let mut tally = tally_backups(&summaries);
            if scope.ctx.deep() {
                let unsized_arns: Vec<&str> = summaries
                    .iter()
                    .filter(|b| b.backup_size_bytes().is_none())
                    .filter_map(|b| b.backup_arn())
                    .collect();
                if !unsized_arns.is_empty() {
                    let (bytes, size_errors) = backup_sizes(scope, client, &unsized_arns).await;
                    errors.extend(size_errors);
                    if let Some(bytes) = bytes {
                        *tally.size_bytes.get_or_insert(0) += bytes;
                    }
                }
            }
            table.backups = Some(tally);
        }
        Err(err) => errors.push(err),
    }

    (Some(table), errors)
}

async fn tags(scope: &Scope<'_>, client: &Client, arn: &str) -> Result<Tags, DiscoveryError> {
    let mut tags = Tags::new();
    let mut token: Option<String> = None;
    loop {
        let output = scope
            .call(
                client
                    .list_tags_of_resource()
                    .resource_arn(arn)
                    .set_next_token(token.clone())
                    .send(),
            )
            .await
            .map_err(|e| failure(SERVICE, "ListTagsOfResource", Some(arn), e))?;
        tags.extend(to_tags(output.tags().iter().map(|t| (t.key(), t.value()))));
        match output.next_token() {
            Some(next) if token.as_deref() != Some(next) => token = Some(next.to_string()),
            _ => break,
        }
    }
    Ok(tags)
}

async fn list_backups(
    scope: &Scope<'_>,
    client: &Client,
    name: &str,
) -> Result<Vec<BackupSummary>, DiscoveryError> {
    let mut summaries = Vec::new();
    let mut start: Option<String> = None;
    loop {
        let output = scope
            .call(
                client
                    .list_backups()
                    .table_name(name)
                    .backup_type(BackupTypeFilter::All)
                    .set_exclusive_start_backup_arn(start.clone())
                    .send(),
            )
            .await
            .map_err(|e| failure(SERVICE, "ListBackups", Some(name), e))?;
        summaries.extend(output.backup_summaries().iter().cloned());
        match output.last_evaluated_backup_arn() {
            Some(next) if start.as_deref() != Some(next) => start = Some(next.to_string()),
            _ => break,
        }
    }
    Ok(summaries)
}

async fn backup_sizes(
    scope: &Scope<'_>,
    client: &Client,
    arns: &[&str],
) -> (Option<u64>, Vec<DiscoveryError>) {
    let described = join_all(
        arns.iter()
            .map(|arn| scope.call(client.describe_backup().backup_arn(*arn).send())),
    )
    .await;
    let mut bytes = None;
    let mut errors = Vec::new();
    for (arn, result) in arns.iter().zip(described) {
        match result {
            Ok(out) => {
                if let Some(size) = out
                    .backup_description()
                    .and_then(|d| d.backup_details())
                    .and_then(|d| d.backup_size_bytes())
                {
                    *bytes.get_or_insert(0) += size.max(0) as u64;
                }
            }
            Err(err) => errors.push(failure(SERVICE, "DescribeBackup", Some(arn), err)),
        }
    }
    (bytes, errors)
}

async fn legacy_global_table_names(
    scope: &Scope<'_>,
    client: &Client,
) -> Result<Vec<String>, DiscoveryError> {
    let mut names = Vec::new();
    let mut start: Option<String> = None;
    loop {
        let output = scope
            .call(
                client
                    .list_global_tables()
                    .region_name(&scope.region)
                    .set_exclusive_start_global_table_name(start.clone())
                    .send(),
            )
            .await
            .map_err(|e| failure(SERVICE, "ListGlobalTables", None, e))?;
        names.extend(
            output
                .global_tables()
                .iter()
                .filter_map(|g| g.global_table_name())
                .map(str::to_string),
        );
        match output.last_evaluated_global_table_name() {
            Some(next) if start.as_deref() != Some(next) => start = Some(next.to_string()),
            _ => break,
        }
    }
    Ok(names)
}

async fn legacy_global_table(
    scope: &Scope<'_>,
    client: &Client,
    name: &str,
) -> (Vec<DynamoReplica>, Vec<DiscoveryError>) {
    let (described, settings) = tokio::join!(
        scope.call(
            client
                .describe_global_table()
                .global_table_name(name)
                .send()
        ),
        scope.call(
            client
                .describe_global_table_settings()
                .global_table_name(name)
                .send()
        ),
    );
    let mut errors = Vec::new();
    let described = match described {
        Ok(out) => Some(out),
        Err(err) => {
            errors.push(failure(SERVICE, "DescribeGlobalTable", Some(name), err));
            None
        }
    };
    let settings = match settings {
        Ok(out) => Some(out),
        Err(err) => {
            errors.push(failure(
                SERVICE,
                "DescribeGlobalTableSettings",
                Some(name),
                err,
            ));
            None
        }
    };
    let replicas = legacy_replicas(
        &scope.region,
        described
            .as_ref()
            .and_then(|d| d.global_table_description()),
        settings
            .as_ref()
            .map(|s| s.replica_settings())
            .unwrap_or_default(),
    );
    (replicas, errors)
}

async fn autoscaling(
    scope: &Scope<'_>,
) -> (HashMap<String, DynamoAutoscaling>, Vec<DiscoveryError>) {
    let client = &aws_sdk_applicationautoscaling::Client::new(scope.sdk());
    let (targets, policies) = tokio::join!(
        async {
            let mut pages = client
                .describe_scalable_targets()
                .service_namespace(ServiceNamespace::Dynamodb)
                .into_paginator()
                .send();
            let mut targets = Vec::new();
            while let Some(page) = scope.call(pages.next()).await {
                let page =
                    page.map_err(|e| failure(AUTOSCALING, "DescribeScalableTargets", None, e))?;
                targets.extend(page.scalable_targets().iter().cloned());
            }
            Ok::<_, DiscoveryError>(targets)
        },
        async {
            let mut pages = client
                .describe_scaling_policies()
                .service_namespace(ServiceNamespace::Dynamodb)
                .into_paginator()
                .send();
            let mut policies = Vec::new();
            while let Some(page) = scope.call(pages.next()).await {
                let page =
                    page.map_err(|e| failure(AUTOSCALING, "DescribeScalingPolicies", None, e))?;
                policies.extend(page.scaling_policies().iter().cloned());
            }
            Ok::<_, DiscoveryError>(policies)
        },
    );
    let mut errors = Vec::new();
    let targets = targets.unwrap_or_else(|err| {
        errors.push(err);
        Vec::new()
    });
    let policies = policies.unwrap_or_else(|err| {
        errors.push(err);
        Vec::new()
    });
    (autoscaling_by_table(&targets, &policies), errors)
}

async fn key_manager(
    scope: &Scope<'_>,
    kms: &aws_sdk_kms::Client,
    arn: &str,
) -> Result<Option<String>, DiscoveryError> {
    let output = scope
        .call(kms.describe_key().key_id(arn).send())
        .await
        .map_err(|e| failure(KMS, "DescribeKey", Some(arn), e))?;
    Ok(output
        .key_metadata()
        .and_then(|m| m.key_manager())
        .map(|m| m.as_str().to_string()))
}

fn from_description(
    name: &str,
    arn: &str,
    region: &str,
    table: Option<&TableDescription>,
) -> DynamoTable {
    let sse = table.and_then(|t| t.sse_description());
    DynamoTable {
        name: name.to_string(),
        arn: table.and_then(|t| t.table_arn()).unwrap_or(arn).to_string(),
        region: region.to_string(),
        status: table
            .and_then(|t| t.table_status())
            .map(|s| s.as_str().to_string()),
        billing_mode: table
            .map(|t| {
                t.billing_mode_summary()
                    .and_then(|s| s.billing_mode())
                    .map(|m| m.as_str())
                    .unwrap_or("PROVISIONED")
                    .to_string()
            })
            .unwrap_or_default(),
        table_class: table.map(|t| {
            t.table_class_summary()
                .and_then(|c| c.table_class())
                .map(|c| c.as_str())
                .unwrap_or("STANDARD")
                .to_string()
        }),
        created_at: table
            .and_then(|t| t.creation_date_time())
            .and_then(timestamp),
        item_count: table.and_then(|t| t.item_count()),
        size_bytes: table.and_then(|t| t.table_size_bytes()),
        read_capacity_units: table
            .and_then(|t| t.provisioned_throughput())
            .and_then(|p| p.read_capacity_units())
            .filter(|&units| units > 0),
        write_capacity_units: table
            .and_then(|t| t.provisioned_throughput())
            .and_then(|p| p.write_capacity_units())
            .filter(|&units| units > 0),
        gsi_count: table.map_or(0, |t| t.global_secondary_indexes().len() as u32),
        lsi_count: table.map_or(0, |t| t.local_secondary_indexes().len() as u32),
        deletion_protection: table.and_then(|t| t.deletion_protection_enabled()),
        sse_type: sse
            .and_then(|s| s.sse_type())
            .map(|s| s.as_str().to_string()),
        kms_key_arn: sse.and_then(|s| s.kms_master_key_arn()).map(str::to_string),
        kms_key_manager: None,
        stream: table.and_then(|t| {
            t.stream_specification()
                .filter(|s| s.stream_enabled())
                .map(|s| DynamoStream {
                    view_type: s.stream_view_type().map(|v| v.as_str().to_string()),
                    arn: t.latest_stream_arn().map(str::to_string),
                })
        }),
        global_table_version: table
            .and_then(|t| t.global_table_version())
            .map(str::to_string),
        replicas: table
            .map(|t| t.replicas())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                Some(DynamoReplica {
                    region: r.region_name()?.to_string(),
                    status: r.replica_status().map(|s| s.as_str().to_string()),
                    ..DynamoReplica::default()
                })
            })
            .collect(),
        pitr: None,
        pitr_earliest_restorable: None,
        pitr_latest_restorable: None,
        pitr_recovery_period_days: None,
        backups: None,
        ttl_status: None,
        ttl_attribute: None,
        contributor_insights: None,
        resource_policy: None,
        autoscaling: None,
        tags: Tags::new(),
        protection: Protection::default(),
    }
}

fn apply_pitr(table: &mut DynamoTable, description: &ContinuousBackupsDescription) {
    let pitr = description.point_in_time_recovery_description();
    table.pitr = pitr
        .and_then(|p| p.point_in_time_recovery_status())
        .map(|s| s.as_str().to_string());
    table.pitr_earliest_restorable = pitr
        .and_then(|p| p.earliest_restorable_date_time())
        .and_then(timestamp);
    table.pitr_latest_restorable = pitr
        .and_then(|p| p.latest_restorable_date_time())
        .and_then(timestamp);
    table.pitr_recovery_period_days = pitr.and_then(|p| p.recovery_period_in_days());
}

fn tally_backups(summaries: &[BackupSummary]) -> DynamoBackups {
    let mut tally = DynamoBackups::default();
    for backup in summaries
        .iter()
        .filter(|b| b.backup_status() != Some(&BackupStatus::Deleted))
    {
        tally.count += 1;
        match backup.backup_type() {
            Some(BackupType::User) => tally.user += 1,
            Some(BackupType::System) => tally.system += 1,
            Some(BackupType::AwsBackup) => tally.aws_backup += 1,
            _ => {}
        }
        if let Some(size) = backup.backup_size_bytes() {
            *tally.size_bytes.get_or_insert(0) += size.max(0) as u64;
        }
        let created = backup.backup_creation_date_time().and_then(timestamp);
        tally.latest = tally.latest.take().max(created);
    }
    tally
}

fn capacity_range(settings: &AutoScalingSettingsDescription) -> Option<DynamoCapacityRange> {
    if settings.auto_scaling_disabled() == Some(true) {
        return None;
    }
    Some(DynamoCapacityRange {
        min: settings.minimum_units()?,
        max: settings.maximum_units()?,
    })
}

fn legacy_replicas(
    home: &str,
    description: Option<&GlobalTableDescription>,
    settings: &[ReplicaSettingsDescription],
) -> Vec<DynamoReplica> {
    let mut replicas: Vec<DynamoReplica> = description
        .map(|d| d.replication_group())
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some(DynamoReplica {
                region: r.region_name()?.to_string(),
                status: r.replica_status().map(|s| s.as_str().to_string()),
                ..DynamoReplica::default()
            })
        })
        .collect();
    for setting in settings {
        let index = match replicas
            .iter()
            .position(|r| r.region == setting.region_name())
        {
            Some(index) => index,
            None => {
                replicas.push(DynamoReplica {
                    region: setting.region_name().to_string(),
                    status: setting.replica_status().map(|s| s.as_str().to_string()),
                    ..DynamoReplica::default()
                });
                replicas.len() - 1
            }
        };
        let replica = &mut replicas[index];
        replica.billing_mode = setting
            .replica_billing_mode_summary()
            .and_then(|b| b.billing_mode())
            .map(|m| m.as_str().to_string());
        replica.read_capacity_units = setting
            .replica_provisioned_read_capacity_units()
            .filter(|&units| units > 0);
        replica.write_capacity_units = setting
            .replica_provisioned_write_capacity_units()
            .filter(|&units| units > 0);
        replica.read_autoscaling = setting
            .replica_provisioned_read_capacity_auto_scaling_settings()
            .and_then(capacity_range);
        replica.write_autoscaling = setting
            .replica_provisioned_write_capacity_auto_scaling_settings()
            .and_then(capacity_range);
    }
    replicas.retain(|r| r.region != home);
    replicas.sort_by(|a, b| a.region.cmp(&b.region));
    replicas
}

fn apply_replica_autoscaling(
    replicas: &mut [DynamoReplica],
    description: &TableAutoScalingDescription,
) {
    for scaling in description.replicas() {
        let Some(replica) = replicas
            .iter_mut()
            .find(|r| Some(r.region.as_str()) == scaling.region_name())
        else {
            continue;
        };
        replica.read_autoscaling = scaling
            .replica_provisioned_read_capacity_auto_scaling_settings()
            .and_then(capacity_range);
        replica.write_autoscaling = scaling
            .replica_provisioned_write_capacity_auto_scaling_settings()
            .and_then(capacity_range);
    }
}

fn scaling_resource(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix("table/")?;
    Some(match rest.split_once("/index/") {
        Some((table, index)) => (table, Some(index)),
        None => (rest, None),
    })
}

fn autoscaling_by_table(
    targets: &[ScalableTarget],
    policies: &[ScalingPolicy],
) -> HashMap<String, DynamoAutoscaling> {
    let mut by_table: HashMap<String, DynamoAutoscaling> = HashMap::new();
    let mut indexes: HashMap<String, BTreeSet<String>> = HashMap::new();
    for target in targets {
        let Some((table, index)) = scaling_resource(target.resource_id()) else {
            continue;
        };
        let scaling = by_table.entry(table.to_string()).or_default();
        let range = DynamoCapacityRange {
            min: target.min_capacity().into(),
            max: target.max_capacity().into(),
        };
        match target.scalable_dimension() {
            ScalableDimension::DynamoDbTableReadCapacityUnits => scaling.read = Some(range),
            ScalableDimension::DynamoDbTableWriteCapacityUnits => scaling.write = Some(range),
            _ => {}
        }
        if let Some(index) = index {
            indexes
                .entry(table.to_string())
                .or_default()
                .insert(index.to_string());
        }
    }
    for (table, names) in indexes {
        if let Some(scaling) = by_table.get_mut(&table) {
            scaling.indexes = names.len() as u32;
        }
    }
    for policy in policies {
        if let Some(scaling) =
            scaling_resource(policy.resource_id()).and_then(|(table, _)| by_table.get_mut(table))
        {
            scaling.policies += 1;
        }
    }
    by_table
}

fn native(table: &DynamoTable) -> Vec<NativeBackup> {
    let mut native = Vec::new();
    if table.pitr_enabled() {
        native.push(NativeBackup {
            kind: NativeKind::DynamodbPitr,
            count: None,
            latest: table.pitr_latest_restorable.clone(),
            detail: table
                .pitr_recovery_period_days
                .map(|days| format!("{days}-day recovery window")),
        });
    }
    if let Some(backups) = table.backups.as_ref().filter(|b| b.count > 0) {
        let split: Vec<String> = [
            (backups.user, "user"),
            (backups.system, "system"),
            (backups.aws_backup, "AWS Backup"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect();
        native.push(NativeBackup {
            kind: NativeKind::DynamodbBackup,
            count: Some(backups.count),
            latest: backups.latest.clone(),
            detail: (!split.is_empty()).then(|| split.join(" · ")),
        });
    }
    native
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_dynamodb::types::{
        BillingMode, BillingModeSummary, GlobalSecondaryIndexDescription,
        PointInTimeRecoveryDescription, PointInTimeRecoveryStatus,
        ProvisionedThroughputDescription, ReplicaAutoScalingDescription, ReplicaDescription,
        ReplicaStatus, SseDescription, SseType, StreamSpecification, StreamViewType, TableClass,
        TableClassSummary, TableStatus,
    };
    use aws_smithy_types::DateTime;

    const ARN: &str = "arn:aws:dynamodb:ap-southeast-2:1:table/orders";

    fn at(secs: i64) -> DateTime {
        DateTime::from_secs(secs)
    }

    fn base() -> DynamoTable {
        from_description("orders", ARN, "ap-southeast-2", None)
    }

    #[test]
    fn maps_describe_table() {
        let described = TableDescription::builder()
            .table_arn(ARN)
            .table_status(TableStatus::Active)
            .creation_date_time(at(1_792_713_600))
            .billing_mode_summary(
                BillingModeSummary::builder()
                    .billing_mode(BillingMode::PayPerRequest)
                    .build(),
            )
            .table_class_summary(
                TableClassSummary::builder()
                    .table_class(TableClass::StandardInfrequentAccess)
                    .build(),
            )
            .provisioned_throughput(
                ProvisionedThroughputDescription::builder()
                    .read_capacity_units(0)
                    .write_capacity_units(0)
                    .build(),
            )
            .table_size_bytes(2048)
            .item_count(7)
            .deletion_protection_enabled(false)
            .sse_description(
                SseDescription::builder()
                    .sse_type(SseType::Kms)
                    .kms_master_key_arn("arn:aws:kms:ap-southeast-2:1:key/k")
                    .build(),
            )
            .stream_specification(
                StreamSpecification::builder()
                    .stream_enabled(true)
                    .stream_view_type(StreamViewType::NewAndOldImages)
                    .build()
                    .unwrap(),
            )
            .latest_stream_arn("arn:stream")
            .global_table_version("2019.11.21")
            .replicas(
                ReplicaDescription::builder()
                    .region_name("us-east-1")
                    .replica_status(ReplicaStatus::Active)
                    .build(),
            )
            .global_secondary_indexes(GlobalSecondaryIndexDescription::builder().build())
            .build();

        let t = from_description("orders", "fallback", "ap-southeast-2", Some(&described));
        assert_eq!(t.arn, ARN);
        assert_eq!(t.status.as_deref(), Some("ACTIVE"));
        assert_eq!(t.billing_mode, "PAY_PER_REQUEST");
        assert_eq!(t.table_class.as_deref(), Some("STANDARD_INFREQUENT_ACCESS"));
        assert_eq!(t.created_at.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(t.read_capacity_units, None);
        assert_eq!((t.gsi_count, t.lsi_count), (1, 0));
        assert_eq!(t.deletion_protection, Some(false));
        assert_eq!(t.sse_type.as_deref(), Some("KMS"));
        assert_eq!(
            t.kms_key_arn.as_deref(),
            Some("arn:aws:kms:ap-southeast-2:1:key/k")
        );
        assert_eq!(
            t.stream,
            Some(DynamoStream {
                view_type: Some("NEW_AND_OLD_IMAGES".into()),
                arn: Some("arn:stream".into()),
            })
        );
        assert_eq!(t.replicas.len(), 1);
        assert_eq!(t.replicas[0].region, "us-east-1");
        assert_eq!(t.replicas[0].status.as_deref(), Some("ACTIVE"));
    }

    #[test]
    fn describe_table_defaults_and_failure() {
        let described = TableDescription::builder()
            .provisioned_throughput(
                ProvisionedThroughputDescription::builder()
                    .read_capacity_units(5)
                    .write_capacity_units(10)
                    .build(),
            )
            .stream_specification(
                StreamSpecification::builder()
                    .stream_enabled(false)
                    .build()
                    .unwrap(),
            )
            .build();
        let t = from_description("orders", ARN, "ap-southeast-2", Some(&described));
        assert_eq!(t.billing_mode, "PROVISIONED");
        assert_eq!(t.table_class.as_deref(), Some("STANDARD"));
        assert_eq!(t.read_capacity_units, Some(5));
        assert_eq!(t.write_capacity_units, Some(10));
        assert_eq!(t.stream, None);
        assert_eq!(t.sse_type, None, "no SSE description means AWS owned key");

        let failed = base();
        assert_eq!(failed.arn, ARN);
        assert_eq!(failed.billing_mode, "");
        assert_eq!(failed.table_class, None);
    }

    #[test]
    fn maps_pitr_and_derives_native_protection() {
        let mut t = base();
        apply_pitr(
            &mut t,
            &ContinuousBackupsDescription::builder()
                .continuous_backups_status(
                    aws_sdk_dynamodb::types::ContinuousBackupsStatus::Enabled,
                )
                .point_in_time_recovery_description(
                    PointInTimeRecoveryDescription::builder()
                        .point_in_time_recovery_status(PointInTimeRecoveryStatus::Enabled)
                        .earliest_restorable_date_time(at(1_792_000_000))
                        .latest_restorable_date_time(at(1_792_713_600))
                        .recovery_period_in_days(14)
                        .build(),
                )
                .build()
                .unwrap(),
        );
        t.backups = Some(DynamoBackups {
            count: 3,
            user: 2,
            aws_backup: 1,
            latest: Some("2026-10-20T00:00:00Z".into()),
            ..DynamoBackups::default()
        });

        assert!(t.pitr_enabled());
        assert_eq!(t.pitr_recovery_period_days, Some(14));
        assert_eq!(
            native(&t),
            vec![
                NativeBackup {
                    kind: NativeKind::DynamodbPitr,
                    count: None,
                    latest: Some("2026-10-23T00:00:00Z".into()),
                    detail: Some("14-day recovery window".into()),
                },
                NativeBackup {
                    kind: NativeKind::DynamodbBackup,
                    count: Some(3),
                    latest: Some("2026-10-20T00:00:00Z".into()),
                    detail: Some("2 user · 1 AWS Backup".into()),
                },
            ]
        );

        t.pitr = Some("DISABLED".into());
        t.backups = Some(DynamoBackups::default());
        assert!(native(&t).is_empty());
    }

    #[test]
    fn tallies_backups_by_type_skipping_deleted() {
        let backup = |kind: BackupType, secs: i64, size: Option<i64>, status: BackupStatus| {
            BackupSummary::builder()
                .backup_type(kind)
                .backup_creation_date_time(at(secs))
                .set_backup_size_bytes(size)
                .backup_status(status)
                .build()
        };
        let tally = tally_backups(&[
            backup(BackupType::User, 100, Some(10), BackupStatus::Available),
            backup(
                BackupType::AwsBackup,
                1_792_713_600,
                Some(5),
                BackupStatus::Available,
            ),
            backup(BackupType::System, 200, None, BackupStatus::Creating),
            backup(
                BackupType::User,
                1_900_000_000,
                Some(99),
                BackupStatus::Deleted,
            ),
        ]);
        assert_eq!(
            tally,
            DynamoBackups {
                count: 3,
                user: 1,
                system: 1,
                aws_backup: 1,
                latest: Some("2026-10-23T00:00:00Z".into()),
                size_bytes: Some(15),
            }
        );
        assert_eq!(tally_backups(&[]), DynamoBackups::default());
    }

    fn target(resource: &str, dimension: ScalableDimension, min: i32, max: i32) -> ScalableTarget {
        ScalableTarget::builder()
            .service_namespace(ServiceNamespace::Dynamodb)
            .resource_id(resource)
            .scalable_dimension(dimension)
            .min_capacity(min)
            .max_capacity(max)
            .role_arn("arn:role")
            .creation_time(at(0))
            .build()
            .unwrap()
    }

    fn policy(resource: &str) -> ScalingPolicy {
        ScalingPolicy::builder()
            .policy_arn("arn:policy")
            .policy_name("p")
            .service_namespace(ServiceNamespace::Dynamodb)
            .resource_id(resource)
            .scalable_dimension(ScalableDimension::DynamoDbTableReadCapacityUnits)
            .policy_type(aws_sdk_applicationautoscaling::types::PolicyType::TargetTrackingScaling)
            .creation_time(at(0))
            .build()
            .unwrap()
    }

    #[test]
    fn groups_autoscaling_targets_by_table() {
        let by_table = autoscaling_by_table(
            &[
                target(
                    "table/orders",
                    ScalableDimension::DynamoDbTableReadCapacityUnits,
                    5,
                    100,
                ),
                target(
                    "table/orders",
                    ScalableDimension::DynamoDbTableWriteCapacityUnits,
                    1,
                    50,
                ),
                target(
                    "table/orders/index/by-customer",
                    ScalableDimension::DynamoDbIndexReadCapacityUnits,
                    1,
                    10,
                ),
                target(
                    "table/orders/index/by-customer",
                    ScalableDimension::DynamoDbIndexWriteCapacityUnits,
                    1,
                    10,
                ),
                target(
                    "keyspace/other/table/x",
                    ScalableDimension::DynamoDbTableReadCapacityUnits,
                    1,
                    1,
                ),
            ],
            &[
                policy("table/orders"),
                policy("table/orders/index/by-customer"),
                policy("table/gone"),
            ],
        );
        assert_eq!(by_table.len(), 1);
        assert_eq!(
            by_table["orders"],
            DynamoAutoscaling {
                read: Some(DynamoCapacityRange { min: 5, max: 100 }),
                write: Some(DynamoCapacityRange { min: 1, max: 50 }),
                indexes: 1,
                policies: 2,
            }
        );
    }

    fn scaling(min: i64, max: i64) -> AutoScalingSettingsDescription {
        AutoScalingSettingsDescription::builder()
            .minimum_units(min)
            .maximum_units(max)
            .auto_scaling_disabled(false)
            .build()
    }

    #[test]
    fn legacy_replicas_merge_description_and_settings() {
        let description = GlobalTableDescription::builder()
            .replication_group(
                ReplicaDescription::builder()
                    .region_name("ap-southeast-2")
                    .replica_status(ReplicaStatus::Active)
                    .build(),
            )
            .replication_group(
                ReplicaDescription::builder()
                    .region_name("us-west-2")
                    .replica_status(ReplicaStatus::Active)
                    .build(),
            )
            .build();
        let settings = [
            ReplicaSettingsDescription::builder()
                .region_name("us-west-2")
                .replica_billing_mode_summary(
                    BillingModeSummary::builder()
                        .billing_mode(BillingMode::Provisioned)
                        .build(),
                )
                .replica_provisioned_read_capacity_units(5)
                .replica_provisioned_write_capacity_units(5)
                .replica_provisioned_read_capacity_auto_scaling_settings(scaling(5, 40))
                .replica_provisioned_write_capacity_auto_scaling_settings(
                    AutoScalingSettingsDescription::builder()
                        .auto_scaling_disabled(true)
                        .build(),
                )
                .build()
                .unwrap(),
            ReplicaSettingsDescription::builder()
                .region_name("eu-west-1")
                .replica_status(ReplicaStatus::Creating)
                .build()
                .unwrap(),
        ];

        let replicas = legacy_replicas("ap-southeast-2", Some(&description), &settings);
        assert_eq!(
            replicas,
            vec![
                DynamoReplica {
                    region: "eu-west-1".into(),
                    status: Some("CREATING".into()),
                    ..DynamoReplica::default()
                },
                DynamoReplica {
                    region: "us-west-2".into(),
                    status: Some("ACTIVE".into()),
                    billing_mode: Some("PROVISIONED".into()),
                    read_capacity_units: Some(5),
                    write_capacity_units: Some(5),
                    read_autoscaling: Some(DynamoCapacityRange { min: 5, max: 40 }),
                    write_autoscaling: None,
                },
            ]
        );
        assert!(legacy_replicas("ap-southeast-2", None, &[]).is_empty());
    }

    #[test]
    fn replica_autoscaling_fills_known_replicas_only() {
        let mut replicas = vec![DynamoReplica {
            region: "us-east-1".into(),
            ..DynamoReplica::default()
        }];
        let description = TableAutoScalingDescription::builder()
            .replicas(
                ReplicaAutoScalingDescription::builder()
                    .region_name("us-east-1")
                    .replica_provisioned_read_capacity_auto_scaling_settings(scaling(1, 20))
                    .build(),
            )
            .replicas(
                ReplicaAutoScalingDescription::builder()
                    .region_name("ap-southeast-2")
                    .replica_provisioned_write_capacity_auto_scaling_settings(scaling(1, 9))
                    .build(),
            )
            .build();
        apply_replica_autoscaling(&mut replicas, &description);
        assert_eq!(replicas.len(), 1);
        assert_eq!(
            replicas[0].read_autoscaling,
            Some(DynamoCapacityRange { min: 1, max: 20 })
        );
        assert_eq!(replicas[0].write_autoscaling, None);
    }

    #[test]
    fn parses_scaling_resource_ids() {
        assert_eq!(scaling_resource("table/orders"), Some(("orders", None)));
        assert_eq!(
            scaling_resource("table/orders/index/gsi-1"),
            Some(("orders", Some("gsi-1")))
        );
        assert_eq!(scaling_resource("service/cluster/x"), None);
    }
}
