use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::SystemTime;

use aws_sdk_cloudwatch::types::{Datapoint, Dimension, Statistic};
use aws_sdk_s3::operation::get_bucket_notification_configuration::GetBucketNotificationConfigurationOutput;
use aws_sdk_s3::operation::get_bucket_tagging::GetBucketTaggingOutput;
use aws_sdk_s3::operation::get_bucket_versioning::GetBucketVersioningOutput;
use aws_sdk_s3::types::{
    BucketLocationConstraint, ExpirationStatus, LifecycleRule, LoggingEnabled, MfaDeleteStatus,
    ObjectLockConfiguration, ObjectLockEnabled, PublicAccessBlockConfiguration,
    ReplicationConfiguration, ReplicationRuleStatus, ServerSideEncryptionConfiguration,
};
use aws_sdk_s3::Client;
use aws_sdk_s3control::types::MultiRegionAccessPointReport;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use aws_smithy_types::DateTime;
use futures::future::join_all;

use cd_model::{
    DiscoveryError, NativeBackup, NativeKind, Protection, PublicAccessBlock, S3Bucket,
    S3Encryption, S3Lifecycle, S3Logging, S3Notifications, S3ObjectLock, S3Replication, Tags,
};

use crate::{failure, timestamp, to_tags, Ctx, Section};

const SERVICE: &str = "s3";

pub(crate) const ACTIONS: &[&str] = &[
    "cloudwatch:GetMetricStatistics",
    "s3:GetBucketLocation",
    "s3:GetBucketLogging",
    "s3:GetBucketNotification",
    "s3:GetBucketObjectLockConfiguration",
    "s3:GetBucketPolicy",
    "s3:GetBucketPublicAccessBlock",
    "s3:GetBucketTagging",
    "s3:GetBucketVersioning",
    "s3:GetEncryptionConfiguration",
    "s3:GetInventoryConfiguration",
    "s3:GetLifecycleConfiguration",
    "s3:GetMetricsConfiguration",
    "s3:GetReplicationConfiguration",
    "s3:ListAllMyBuckets",
    "s3:ListMultiRegionAccessPoints",
];

const MRAP_CONTROL_REGION: &str = "us-west-2";

const STORAGE_TYPES: &[&str] = &[
    "StandardStorage",
    "StandardIAStorage",
    "OneZoneIAStorage",
    "IntelligentTieringFAStorage",
    "IntelligentTieringIAStorage",
    "IntelligentTieringAIAStorage",
    "GlacierInstantRetrievalStorage",
    "GlacierStorage",
    "DeepArchiveStorage",
    "IntelligentTieringAAStorage",
    "IntelligentTieringDAAStorage",
    "ReducedRedundancyStorage",
];

const DEEP_STORAGE_TYPES: &[&str] = &[
    "StandardIASizeOverhead",
    "StandardIAObjectOverhead",
    "OneZoneIASizeOverhead",
    "GlacierInstantRetrievalSizeOverhead",
    "GlacierStagingStorage",
    "GlacierObjectOverhead",
    "GlacierS3ObjectOverhead",
    "DeepArchiveObjectOverhead",
    "DeepArchiveS3ObjectOverhead",
    "DeepArchiveStagingStorage",
];

const DAY_SECS: i32 = 86_400;
const METRIC_WINDOW_DAYS: i64 = 3;

struct Listed {
    name: String,
    region: Option<String>,
    created_at: Option<String>,
}

struct Clients {
    s3: Client,
    cloudwatch: aws_sdk_cloudwatch::Client,
}

pub async fn discover(ctx: &Ctx, regions: &[String]) -> Section<S3Bucket> {
    let home = Client::new(ctx.home());
    let listed = match list_buckets(ctx, &home).await {
        Ok(listed) => listed,
        Err(err) => return Section::failed(err),
    };
    let mut errors = Vec::new();
    let located = join_all(listed.into_iter().map(|bucket| locate(ctx, &home, bucket))).await;
    let in_scope: Vec<Listed> = located
        .into_iter()
        .filter_map(|located| located.map_err(|err| errors.push(err)).ok())
        .filter(|b| b.region.as_ref().is_some_and(|r| regions.contains(r)))
        .collect();

    let clients: HashMap<&str, Clients> = in_scope
        .iter()
        .filter_map(|b| b.region.as_deref())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|region| {
            let sdk = ctx.sdk_for(region);
            let clients = Clients {
                s3: Client::new(&sdk),
                cloudwatch: aws_sdk_cloudwatch::Client::new(&sdk),
            };
            (region, clients)
        })
        .collect();

    let (described, mraps) = tokio::join!(
        join_all(in_scope.iter().map(|bucket| {
            let region = bucket.region.as_deref().unwrap_or_default();
            describe(ctx, &clients[region], bucket, region)
        })),
        async {
            if in_scope.is_empty() {
                Ok(HashMap::new())
            } else {
                mrap_members(ctx).await
            }
        },
    );
    let mraps = mraps.unwrap_or_else(|err| {
        errors.push(err);
        HashMap::new()
    });

    let mut buckets = Vec::new();
    for (bucket, bucket_errors) in described {
        errors.extend(bucket_errors);
        if let Some(mut bucket) = bucket {
            bucket.mraps = mraps.get(&bucket.name).cloned().unwrap_or_default();
            bucket.protection.native = native_protection(&bucket);
            buckets.push(bucket);
        }
    }
    Section::ok(buckets, errors)
}

async fn list_buckets(ctx: &Ctx, client: &Client) -> Result<Vec<Listed>, DiscoveryError> {
    let mut pages = client
        .list_buckets()
        .max_buckets(1000)
        .into_paginator()
        .send();
    let mut listed = Vec::new();
    while let Some(page) = ctx.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListBuckets", None, e))?;
        for bucket in page.buckets() {
            listed.push(Listed {
                name: bucket.name().unwrap_or_default().to_string(),
                region: bucket.bucket_region().map(str::to_string),
                created_at: bucket.creation_date().and_then(timestamp),
            });
        }
    }
    Ok(listed)
}

async fn locate(ctx: &Ctx, client: &Client, mut bucket: Listed) -> Result<Listed, DiscoveryError> {
    if bucket.region.is_none() {
        let output = ctx
            .call(client.get_bucket_location().bucket(&bucket.name).send())
            .await
            .map_err(|e| failure(SERVICE, "GetBucketLocation", Some(&bucket.name), e))?;
        bucket.region = Some(location_region(output.location_constraint()));
    }
    Ok(bucket)
}

fn location_region(constraint: Option<&BucketLocationConstraint>) -> String {
    match constraint.map(BucketLocationConstraint::as_str) {
        None | Some("") => "us-east-1",
        Some("EU") => "eu-west-1",
        Some(region) => region,
    }
    .to_string()
}

async fn describe(
    ctx: &Ctx,
    clients: &Clients,
    listed: &Listed,
    region: &str,
) -> (Option<S3Bucket>, Vec<DiscoveryError>) {
    let s3 = &clients.s3;
    let name = listed.name.as_str();
    let mut errors = Vec::new();

    let tagging = ctx.call(s3.get_bucket_tagging().bucket(name).send()).await;
    let tags = read(
        tagging,
        &["NoSuchTagSet"],
        "GetBucketTagging",
        name,
        &mut errors,
        tags_of,
    )
    .unwrap_or_default();
    if !ctx.filters().admits(&tags) {
        return (None, with_region(errors, region));
    }

    let (
        versioning,
        encryption,
        public_access,
        policy,
        replication,
        lifecycle,
        logging,
        object_lock,
        notifications,
        metrics,
        inventory,
        (size, size_error),
    ) = tokio::join!(
        ctx.call(s3.get_bucket_versioning().bucket(name).send()),
        ctx.call(s3.get_bucket_encryption().bucket(name).send()),
        ctx.call(s3.get_public_access_block().bucket(name).send()),
        ctx.call(s3.get_bucket_policy().bucket(name).send()),
        ctx.call(s3.get_bucket_replication().bucket(name).send()),
        ctx.call(s3.get_bucket_lifecycle_configuration().bucket(name).send()),
        ctx.call(s3.get_bucket_logging().bucket(name).send()),
        ctx.call(s3.get_object_lock_configuration().bucket(name).send()),
        ctx.call(
            s3.get_bucket_notification_configuration()
                .bucket(name)
                .send()
        ),
        count_metrics_configurations(ctx, s3, name),
        count_inventory_configurations(ctx, s3, name),
        bucket_size(ctx, &clients.cloudwatch, name),
    );

    let versioning = read(
        versioning,
        &[],
        "GetBucketVersioning",
        name,
        &mut errors,
        versioning_of,
    );
    let encryption = read(
        encryption,
        &["ServerSideEncryptionConfigurationNotFoundError"],
        "GetBucketEncryption",
        name,
        &mut errors,
        |out| encryption_of(out.and_then(|o| o.server_side_encryption_configuration())),
    );
    let public_access_block = read(
        public_access,
        &["NoSuchPublicAccessBlockConfiguration"],
        "GetPublicAccessBlock",
        name,
        &mut errors,
        |out| public_access_block_of(out.and_then(|o| o.public_access_block_configuration())),
    );
    let has_policy = read(
        policy,
        &["NoSuchBucketPolicy"],
        "GetBucketPolicy",
        name,
        &mut errors,
        |out| out.and_then(|o| o.policy()).is_some_and(|p| !p.is_empty()),
    );
    let replication = read(
        replication,
        &["ReplicationConfigurationNotFoundError"],
        "GetBucketReplication",
        name,
        &mut errors,
        |out| replication_of(out.and_then(|o| o.replication_configuration())),
    );
    let lifecycle = read(
        lifecycle,
        &["NoSuchLifecycleConfiguration"],
        "GetBucketLifecycleConfiguration",
        name,
        &mut errors,
        |out| lifecycle_of(out.map(|o| o.rules()).unwrap_or_default()),
    );
    let logging = read(logging, &[], "GetBucketLogging", name, &mut errors, |out| {
        logging_of(out.and_then(|o| o.logging_enabled()))
    });
    let object_lock = read(
        object_lock,
        &["ObjectLockConfigurationNotFoundError"],
        "GetObjectLockConfiguration",
        name,
        &mut errors,
        |out| object_lock_of(out.and_then(|o| o.object_lock_configuration())),
    );
    let notifications = read(
        notifications,
        &[],
        "GetBucketNotificationConfiguration",
        name,
        &mut errors,
        notifications_of,
    );
    let metrics_configurations = metrics.map_err(|e| errors.push(e)).ok();
    let inventory_configurations = inventory.map_err(|e| errors.push(e)).ok();
    errors.extend(size_error);

    let (versioning, mfa_delete) = versioning.unzip();
    let bucket = S3Bucket {
        name: listed.name.clone(),
        arn: format!("arn:{}:s3:::{name}", ctx.partition),
        region: region.to_string(),
        created_at: listed.created_at.clone(),
        versioning,
        mfa_delete: mfa_delete.flatten(),
        encryption,
        public_access_block,
        has_policy,
        replication,
        lifecycle,
        logging,
        object_lock,
        notifications,
        metrics_configurations,
        inventory_configurations,
        size_bytes: size.total,
        object_count: size.objects,
        storage_classes: size.classes,
        mraps: Vec::new(),
        tags,
        protection: Protection::default(),
    };
    (Some(bucket), with_region(errors, region))
}

fn with_region(mut errors: Vec<DiscoveryError>, region: &str) -> Vec<DiscoveryError> {
    for error in &mut errors {
        error.region.get_or_insert_with(|| region.to_string());
    }
    errors
}

fn read<O, E: ProvideErrorMetadata + std::error::Error, T>(
    result: Result<O, E>,
    absent: &[&str],
    operation: &str,
    bucket: &str,
    errors: &mut Vec<DiscoveryError>,
    map: impl FnOnce(Option<&O>) -> T,
) -> Option<T> {
    match result {
        Ok(output) => Some(map(Some(&output))),
        Err(err) if err.code().is_some_and(|code| absent.contains(&code)) => Some(map(None)),
        Err(err) => {
            errors.push(failure(SERVICE, operation, Some(bucket), err));
            None
        }
    }
}

fn tags_of(output: Option<&GetBucketTaggingOutput>) -> Tags {
    to_tags(
        output
            .map(|o| o.tag_set())
            .unwrap_or_default()
            .iter()
            .map(|t| (t.key(), t.value())),
    )
}

fn versioning_of(output: Option<&GetBucketVersioningOutput>) -> (String, Option<bool>) {
    let status = output
        .and_then(|o| o.status())
        .map_or("Disabled", |s| s.as_str());
    let mfa_delete = output.map(|o| o.mfa_delete() == Some(&MfaDeleteStatus::Enabled));
    (status.to_string(), mfa_delete)
}

fn encryption_of(config: Option<&ServerSideEncryptionConfiguration>) -> S3Encryption {
    let rule = config.and_then(|c| c.rules().first());
    let default = rule.and_then(|r| r.apply_server_side_encryption_by_default());
    S3Encryption {
        algorithm: default.map(|d| d.sse_algorithm().as_str().to_string()),
        kms_key_id: default
            .and_then(|d| d.kms_master_key_id())
            .map(str::to_string),
        bucket_key: rule.and_then(|r| r.bucket_key_enabled()),
    }
}

fn public_access_block_of(config: Option<&PublicAccessBlockConfiguration>) -> PublicAccessBlock {
    let on = |flag: fn(&PublicAccessBlockConfiguration) -> Option<bool>| {
        config.and_then(flag).unwrap_or(false)
    };
    PublicAccessBlock {
        block_public_acls: on(PublicAccessBlockConfiguration::block_public_acls),
        ignore_public_acls: on(PublicAccessBlockConfiguration::ignore_public_acls),
        block_public_policy: on(PublicAccessBlockConfiguration::block_public_policy),
        restrict_public_buckets: on(PublicAccessBlockConfiguration::restrict_public_buckets),
    }
}

fn replication_of(config: Option<&ReplicationConfiguration>) -> S3Replication {
    let rules = config.map(|c| c.rules()).unwrap_or_default();
    let mut replication = S3Replication {
        rules: rules.len() as u32,
        ..S3Replication::default()
    };
    for rule in rules
        .iter()
        .filter(|r| *r.status() == ReplicationRuleStatus::Enabled)
    {
        replication.enabled_rules += 1;
        if let Some(destination) = rule.destination() {
            let bucket = destination.bucket().to_string();
            if !replication.destinations.contains(&bucket) {
                replication.destinations.push(bucket);
            }
        }
    }
    replication
}

fn lifecycle_of(rules: &[LifecycleRule]) -> S3Lifecycle {
    let enabled = || {
        rules
            .iter()
            .filter(|r| *r.status() == ExpirationStatus::Enabled)
    };
    S3Lifecycle {
        rules: rules.len() as u32,
        transitions: enabled()
            .any(|r| !r.transitions().is_empty() || !r.noncurrent_version_transitions().is_empty()),
        expiration: enabled().any(|r| {
            r.expiration()
                .is_some_and(|e| e.days().is_some() || e.date().is_some())
        }),
        noncurrent_expiration: enabled().any(|r| r.noncurrent_version_expiration().is_some()),
    }
}

fn logging_of(enabled: Option<&LoggingEnabled>) -> S3Logging {
    S3Logging {
        target_bucket: enabled.map(|l| l.target_bucket().to_string()),
    }
}

fn object_lock_of(config: Option<&ObjectLockConfiguration>) -> S3ObjectLock {
    let retention = config
        .and_then(|c| c.rule())
        .and_then(|r| r.default_retention());
    S3ObjectLock {
        enabled: config.and_then(|c| c.object_lock_enabled()) == Some(&ObjectLockEnabled::Enabled),
        mode: retention
            .and_then(|r| r.mode())
            .map(|m| m.as_str().to_string()),
        days: retention.and_then(|r| r.days()),
        years: retention.and_then(|r| r.years()),
    }
}

fn notifications_of(output: Option<&GetBucketNotificationConfigurationOutput>) -> S3Notifications {
    let Some(output) = output else {
        return S3Notifications::default();
    };
    S3Notifications {
        eventbridge: output.event_bridge_configuration().is_some(),
        topics: output.topic_configurations().len() as u32,
        queues: output.queue_configurations().len() as u32,
        lambdas: output.lambda_function_configurations().len() as u32,
    }
}

async fn count_metrics_configurations(
    ctx: &Ctx,
    client: &Client,
    bucket: &str,
) -> Result<u32, DiscoveryError> {
    let mut count = 0;
    let mut token = None;
    loop {
        let page = ctx
            .call(
                client
                    .list_bucket_metrics_configurations()
                    .bucket(bucket)
                    .set_continuation_token(token)
                    .send(),
            )
            .await
            .map_err(|e| failure(SERVICE, "ListBucketMetricsConfigurations", Some(bucket), e))?;
        count += page.metrics_configuration_list().len() as u32;
        match page.next_continuation_token() {
            Some(next) if page.is_truncated() == Some(true) => token = Some(next.to_string()),
            _ => return Ok(count),
        }
    }
}

async fn count_inventory_configurations(
    ctx: &Ctx,
    client: &Client,
    bucket: &str,
) -> Result<u32, DiscoveryError> {
    let mut count = 0;
    let mut token = None;
    loop {
        let page = ctx
            .call(
                client
                    .list_bucket_inventory_configurations()
                    .bucket(bucket)
                    .set_continuation_token(token)
                    .send(),
            )
            .await
            .map_err(|e| {
                failure(
                    SERVICE,
                    "ListBucketInventoryConfigurations",
                    Some(bucket),
                    e,
                )
            })?;
        count += page.inventory_configuration_list().len() as u32;
        match page.next_continuation_token() {
            Some(next) if page.is_truncated() == Some(true) => token = Some(next.to_string()),
            _ => return Ok(count),
        }
    }
}

#[derive(Default)]
struct BucketSize {
    classes: BTreeMap<String, u64>,
    total: Option<u64>,
    objects: Option<u64>,
}

async fn bucket_size(
    ctx: &Ctx,
    client: &aws_sdk_cloudwatch::Client,
    bucket: &str,
) -> (BucketSize, Option<DiscoveryError>) {
    let end = DateTime::from(SystemTime::now());
    let start = DateTime::from_secs(end.secs() - METRIC_WINDOW_DAYS * i64::from(DAY_SECS));
    let query = |metric: &str, storage_type: &str| {
        ctx.call(
            client
                .get_metric_statistics()
                .namespace("AWS/S3")
                .metric_name(metric)
                .dimensions(
                    Dimension::builder()
                        .name("BucketName")
                        .value(bucket)
                        .build(),
                )
                .dimensions(
                    Dimension::builder()
                        .name("StorageType")
                        .value(storage_type)
                        .build(),
                )
                .start_time(start)
                .end_time(end)
                .period(DAY_SECS)
                .statistics(Statistic::Average)
                .send(),
        )
    };
    let deep: &[&str] = if ctx.deep() { DEEP_STORAGE_TYPES } else { &[] };
    let storage_types: Vec<&str> = STORAGE_TYPES.iter().chain(deep).copied().collect();
    let (sizes, objects) = tokio::join!(
        join_all(
            storage_types
                .iter()
                .map(|storage_type| query("BucketSizeBytes", storage_type))
        ),
        query("NumberOfObjects", "AllStorageTypes"),
    );

    let mut size = BucketSize::default();
    let mut error = None;
    let mut complete = true;
    for (storage_type, result) in storage_types.iter().zip(sizes) {
        match result {
            Ok(output) => {
                if let Some(bytes) = latest_average(output.datapoints()) {
                    size.classes.insert(storage_type.to_string(), bytes);
                }
            }
            Err(err) => {
                complete = false;
                error.get_or_insert_with(|| {
                    failure("cloudwatch", "GetMetricStatistics", Some(bucket), err)
                });
            }
        }
    }
    if complete && !size.classes.is_empty() {
        size.total = Some(
            size.classes
                .iter()
                .filter(|(class, _)| !class.ends_with("Overhead"))
                .map(|(_, bytes)| bytes)
                .sum(),
        );
    }
    match objects {
        Ok(output) => size.objects = latest_average(output.datapoints()),
        Err(err) => {
            error.get_or_insert_with(|| {
                failure("cloudwatch", "GetMetricStatistics", Some(bucket), err)
            });
        }
    }
    (size, error)
}

fn latest_average(datapoints: &[Datapoint]) -> Option<u64> {
    datapoints
        .iter()
        .filter(|d| d.timestamp().is_some())
        .max_by_key(|d| d.timestamp())
        .and_then(Datapoint::average)
        .map(|value| value.max(0.0).round() as u64)
}

async fn mrap_members(ctx: &Ctx) -> Result<HashMap<String, Vec<String>>, DiscoveryError> {
    if ctx.partition != "aws" {
        return Ok(HashMap::new());
    }
    let client = aws_sdk_s3control::Client::new(&ctx.sdk_for(MRAP_CONTROL_REGION));
    let mut pages = client
        .list_multi_region_access_points()
        .account_id(&ctx.account)
        .into_paginator()
        .send();
    let mut reports = Vec::new();
    while let Some(page) = ctx.call(pages.next()).await {
        let page = page.map_err(|e| failure(SERVICE, "ListMultiRegionAccessPoints", None, e))?;
        reports.extend(page.access_points().iter().cloned());
    }
    Ok(mrap_members_of(&reports))
}

fn mrap_members_of(reports: &[MultiRegionAccessPointReport]) -> HashMap<String, Vec<String>> {
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for report in reports {
        let Some(mrap) = report.name() else {
            continue;
        };
        for bucket in report.regions().iter().filter_map(|r| r.bucket()) {
            let names = members.entry(bucket.to_string()).or_default();
            if !names.iter().any(|n| n == mrap) {
                names.push(mrap.to_string());
            }
        }
    }
    members
}

fn native_protection(bucket: &S3Bucket) -> Vec<NativeBackup> {
    let record = |kind, count, detail| NativeBackup {
        kind,
        count,
        latest: None,
        detail: Some(detail),
    };
    let mut native = Vec::new();
    if let Some(status @ ("Enabled" | "Suspended")) = bucket.versioning.as_deref() {
        native.push(record(NativeKind::S3Versioning, None, status.to_string()));
    }
    if let Some(replication) = bucket.replication.as_ref().filter(|r| r.enabled_rules > 0) {
        let destinations: Vec<&str> = replication
            .destinations
            .iter()
            .map(|arn| arn.split_once(":::").map_or(arn.as_str(), |(_, name)| name))
            .collect();
        native.push(record(
            NativeKind::S3Replication,
            Some(u64::from(replication.enabled_rules)),
            destinations.join(", "),
        ));
    }
    if let Some(lock) = bucket.object_lock.as_ref().filter(|l| l.enabled) {
        native.push(record(NativeKind::S3ObjectLock, None, lock.describe()));
    }
    native
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3::config::http::HttpResponse;
    use aws_sdk_s3::error::SdkError;
    use aws_sdk_s3::operation::get_bucket_lifecycle_configuration::GetBucketLifecycleConfigurationError;
    use aws_sdk_s3::types::{
        DefaultRetention, Destination, EventBridgeConfiguration, LifecycleExpiration,
        NoncurrentVersionExpiration, ObjectLockRetentionMode, ObjectLockRule, QueueConfiguration,
        ReplicationRule, ServerSideEncryption, ServerSideEncryptionByDefault,
        ServerSideEncryptionRule, Tag, Transition,
    };
    use aws_sdk_s3control::types::RegionReport;
    use aws_smithy_types::body::SdkBody;

    #[test]
    fn maps_legacy_location_constraints() {
        assert_eq!(location_region(None), "us-east-1");
        let empty = BucketLocationConstraint::from("");
        assert_eq!(location_region(Some(&empty)), "us-east-1");
        assert_eq!(
            location_region(Some(&BucketLocationConstraint::Eu)),
            "eu-west-1"
        );
        assert_eq!(
            location_region(Some(&BucketLocationConstraint::ApSoutheast2)),
            "ap-southeast-2"
        );
    }

    fn lifecycle_error(code: &str) -> SdkError<GetBucketLifecycleConfigurationError> {
        let meta = aws_sdk_s3::error::ErrorMetadata::builder()
            .code(code)
            .message("no")
            .build();
        let response = HttpResponse::new(404u16.try_into().unwrap(), SdkBody::empty());
        SdkError::service_error(
            GetBucketLifecycleConfigurationError::generic(meta),
            response,
        )
    }

    #[test]
    fn absent_codes_read_as_not_configured() {
        let mut errors = Vec::new();
        let absent = ["NoSuchLifecycleConfiguration"];
        let op = "GetBucketLifecycleConfiguration";

        let missing: Result<(), _> = Err(lifecycle_error("NoSuchLifecycleConfiguration"));
        let got = read(missing, &absent, op, "data", &mut errors, |o| o.is_some());
        assert_eq!(got, Some(false));
        assert!(errors.is_empty());

        let denied: Result<(), _> = Err(lifecycle_error("AccessDenied"));
        let got = read(denied, &absent, op, "data", &mut errors, |o| o.is_some());
        assert_eq!(got, None);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].operation, op);
        assert_eq!(errors[0].message, "data: AccessDenied: no");
    }

    #[test]
    fn maps_tags() {
        let output = GetBucketTaggingOutput::builder()
            .tag_set(Tag::builder().key("env").value("prod").build().unwrap())
            .build()
            .unwrap();
        assert_eq!(tags_of(Some(&output)).get("env").unwrap(), "prod");
        assert!(tags_of(None).is_empty());
    }

    #[test]
    fn maps_versioning_and_mfa_delete() {
        let never = GetBucketVersioningOutput::builder().build();
        assert_eq!(
            versioning_of(Some(&never)),
            ("Disabled".into(), Some(false))
        );
        let on = GetBucketVersioningOutput::builder()
            .status("Enabled".into())
            .mfa_delete(MfaDeleteStatus::Enabled)
            .build();
        assert_eq!(versioning_of(Some(&on)), ("Enabled".into(), Some(true)));
    }

    #[test]
    fn maps_default_encryption() {
        assert_eq!(encryption_of(None), S3Encryption::default());
        let config = ServerSideEncryptionConfiguration::builder()
            .rules(
                ServerSideEncryptionRule::builder()
                    .apply_server_side_encryption_by_default(
                        ServerSideEncryptionByDefault::builder()
                            .sse_algorithm(ServerSideEncryption::AwsKms)
                            .kms_master_key_id("arn:aws:kms:ap-southeast-2:1:key/k")
                            .build()
                            .unwrap(),
                    )
                    .bucket_key_enabled(true)
                    .build(),
            )
            .build()
            .unwrap();
        assert_eq!(
            encryption_of(Some(&config)),
            S3Encryption {
                algorithm: Some("aws:kms".into()),
                kms_key_id: Some("arn:aws:kms:ap-southeast-2:1:key/k".into()),
                bucket_key: Some(true),
            }
        );
    }

    #[test]
    fn maps_public_access_block() {
        assert!(!public_access_block_of(None).fully_on());
        let config = PublicAccessBlockConfiguration::builder()
            .block_public_acls(true)
            .ignore_public_acls(true)
            .block_public_policy(true)
            .build();
        let block = public_access_block_of(Some(&config));
        assert_eq!(block.off(), vec!["RestrictPublicBuckets"]);
    }

    fn replication_rule(status: ReplicationRuleStatus, bucket: &str) -> ReplicationRule {
        ReplicationRule::builder()
            .status(status)
            .destination(Destination::builder().bucket(bucket).build().unwrap())
            .build()
            .unwrap()
    }

    #[test]
    fn maps_replication_from_enabled_rules() {
        assert_eq!(replication_of(None), S3Replication::default());
        let config = ReplicationConfiguration::builder()
            .role("arn:aws:iam::1:role/r")
            .rules(replication_rule(
                ReplicationRuleStatus::Enabled,
                "arn:aws:s3:::dr",
            ))
            .rules(replication_rule(
                ReplicationRuleStatus::Enabled,
                "arn:aws:s3:::dr",
            ))
            .rules(replication_rule(
                ReplicationRuleStatus::Disabled,
                "arn:aws:s3:::old",
            ))
            .build()
            .unwrap();
        assert_eq!(
            replication_of(Some(&config)),
            S3Replication {
                rules: 3,
                enabled_rules: 2,
                destinations: vec!["arn:aws:s3:::dr".into()],
            }
        );
    }

    #[test]
    fn lifecycle_flags_ignore_disabled_rules() {
        let transition = LifecycleRule::builder()
            .status(ExpirationStatus::Enabled)
            .transitions(Transition::builder().days(30).build())
            .noncurrent_version_expiration(
                NoncurrentVersionExpiration::builder()
                    .noncurrent_days(90)
                    .build(),
            )
            .build()
            .unwrap();
        let marker_only = LifecycleRule::builder()
            .status(ExpirationStatus::Enabled)
            .expiration(
                LifecycleExpiration::builder()
                    .expired_object_delete_marker(true)
                    .build(),
            )
            .build()
            .unwrap();
        let disabled = LifecycleRule::builder()
            .status(ExpirationStatus::Disabled)
            .expiration(LifecycleExpiration::builder().days(1).build())
            .build()
            .unwrap();
        assert_eq!(
            lifecycle_of(&[transition, marker_only, disabled]),
            S3Lifecycle {
                rules: 3,
                transitions: true,
                expiration: false,
                noncurrent_expiration: true,
            }
        );
        assert_eq!(lifecycle_of(&[]), S3Lifecycle::default());
    }

    #[test]
    fn maps_logging_and_notifications() {
        assert!(!logging_of(None).enabled());
        let target = LoggingEnabled::builder()
            .target_bucket("logs")
            .target_prefix("data/")
            .build()
            .unwrap();
        assert_eq!(
            logging_of(Some(&target)).target_bucket.as_deref(),
            Some("logs")
        );

        let output = GetBucketNotificationConfigurationOutput::builder()
            .event_bridge_configuration(EventBridgeConfiguration::builder().build())
            .queue_configurations(
                QueueConfiguration::builder()
                    .queue_arn("arn:aws:sqs:ap-southeast-2:1:q")
                    .events("s3:ObjectCreated:*".into())
                    .build()
                    .unwrap(),
            )
            .build();
        assert_eq!(
            notifications_of(Some(&output)),
            S3Notifications {
                eventbridge: true,
                topics: 0,
                queues: 1,
                lambdas: 0,
            }
        );
    }

    #[test]
    fn maps_object_lock() {
        assert_eq!(object_lock_of(None), S3ObjectLock::default());
        let config = ObjectLockConfiguration::builder()
            .object_lock_enabled(ObjectLockEnabled::Enabled)
            .rule(
                ObjectLockRule::builder()
                    .default_retention(
                        DefaultRetention::builder()
                            .mode(ObjectLockRetentionMode::Compliance)
                            .days(30)
                            .build(),
                    )
                    .build(),
            )
            .build();
        let lock = object_lock_of(Some(&config));
        assert!(lock.enabled);
        assert_eq!(lock.describe(), "COMPLIANCE 30d");
    }

    #[test]
    fn takes_the_latest_datapoint() {
        let point = |secs, average| {
            Datapoint::builder()
                .timestamp(DateTime::from_secs(secs))
                .average(average)
                .build()
        };
        assert_eq!(latest_average(&[]), None);
        assert_eq!(
            latest_average(&[point(200, 2048.4), point(100, 1024.0), point(150, 7.0)]),
            Some(2048)
        );
    }

    #[test]
    fn maps_mrap_members_by_bucket() {
        let report = |name: &str, buckets: &[&str]| {
            let mut builder = MultiRegionAccessPointReport::builder().name(name);
            for bucket in buckets {
                builder = builder.regions(RegionReport::builder().bucket(*bucket).build());
            }
            builder.build()
        };
        let members = mrap_members_of(&[
            report("global", &["data-use1", "data-apse2"]),
            report("dr", &["data-apse2", "data-apse2"]),
        ]);
        assert_eq!(members["data-use1"], vec!["global"]);
        assert_eq!(members["data-apse2"], vec!["global", "dr"]);
    }

    fn bucket() -> S3Bucket {
        S3Bucket {
            name: "data".into(),
            arn: "arn:aws:s3:::data".into(),
            region: "ap-southeast-2".into(),
            created_at: None,
            versioning: Some("Disabled".into()),
            mfa_delete: None,
            encryption: None,
            public_access_block: None,
            has_policy: None,
            replication: Some(S3Replication::default()),
            lifecycle: None,
            logging: None,
            object_lock: Some(S3ObjectLock::default()),
            notifications: None,
            metrics_configurations: None,
            inventory_configurations: None,
            size_bytes: None,
            object_count: None,
            storage_classes: BTreeMap::new(),
            mraps: Vec::new(),
            tags: Tags::new(),
            protection: Protection::default(),
        }
    }

    #[test]
    fn native_protection_records_versioning_replication_and_lock() {
        assert!(native_protection(&bucket()).is_empty());

        let protected = S3Bucket {
            versioning: Some("Enabled".into()),
            replication: Some(S3Replication {
                rules: 2,
                enabled_rules: 1,
                destinations: vec!["arn:aws:s3:::dr".into()],
            }),
            object_lock: Some(S3ObjectLock {
                enabled: true,
                mode: Some("GOVERNANCE".into()),
                days: None,
                years: Some(1),
            }),
            ..bucket()
        };
        let native = native_protection(&protected);
        let got: Vec<_> = native
            .iter()
            .map(|n| (n.kind, n.count, n.detail.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (NativeKind::S3Versioning, None, Some("Enabled")),
                (NativeKind::S3Replication, Some(1), Some("dr")),
                (NativeKind::S3ObjectLock, None, Some("GOVERNANCE 1y")),
            ]
        );

        let mut settled = Protection {
            native,
            ..Protection::default()
        };
        settled.settle();
        assert_eq!(settled.status, cd_model::ProtectionStatus::Unprotected);
    }
}
