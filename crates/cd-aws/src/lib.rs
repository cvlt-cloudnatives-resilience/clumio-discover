use std::future::Future;
use std::time::SystemTime;

use aws_config::retry::RetryConfig;
use aws_config::{BehaviorVersion, SdkConfig};
use aws_smithy_types::date_time::Format;
use aws_smithy_types::error::display::DisplayErrorContext;
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use aws_smithy_types::DateTime;
use aws_types::region::Region;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;

use cd_model::progress::GLOBAL;
use cd_model::{
    BackupVault, Config, DiscoveryError, DocumentDbCluster, EbsVolume, Filters, Inventory,
    NeptuneCluster, Progress, ProtectedResource, RdsDatabase, RecycleBinRule, Service, Tags,
    SCHEMA_VERSION,
};

mod account;
mod backup;
mod cloudtrail;
mod databases;
mod dynamodb;
mod ebs;
mod ec2;
mod glue;
mod policy;
mod rbin;
mod regions;
mod s3;
mod s3tables;

pub use policy::document as policy_document;

const FALLBACK_HOME_REGION: &str = "us-east-1";

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("could not use AWS credentials: {0}")]
    CallerIdentity(String),
}

pub fn actions() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = [
        account::ACTIONS,
        backup::ACTIONS,
        cloudtrail::ACTIONS,
        databases::ACTIONS,
        dynamodb::ACTIONS,
        ebs::ACTIONS,
        ec2::ACTIONS,
        glue::ACTIONS,
        rbin::ACTIONS,
        regions::ACTIONS,
        s3::ACTIONS,
        s3tables::ACTIONS,
    ]
    .concat();
    all.sort_unstable();
    all.dedup();
    all
}

pub(crate) struct Ctx {
    home: SdkConfig,
    pub partition: String,
    pub account: String,
    pub config: Config,
    limit: Semaphore,
    progress: Option<UnboundedSender<Progress>>,
}

impl Ctx {
    pub fn home(&self) -> &SdkConfig {
        &self.home
    }

    pub fn sdk_for(&self, region: &str) -> SdkConfig {
        self.home
            .to_builder()
            .region(Region::new(region.to_string()))
            .build()
    }

    pub fn scope(&self, region: &str) -> Scope<'_> {
        Scope {
            ctx: self,
            region: region.to_string(),
            sdk: self.sdk_for(region),
        }
    }

    pub fn filters(&self) -> &Filters {
        &self.config.filters
    }

    pub fn deep(&self) -> bool {
        self.config.depth == cd_model::Depth::Deep
    }

    pub async fn call<F: Future>(&self, call: F) -> F::Output {
        let _permit = self.limit.acquire().await.expect("limit semaphore closed");
        call.await
    }

    fn emit(&self, event: Progress) {
        if let Some(tx) = &self.progress {
            let _ = tx.send(event);
        }
    }

    async fn track<S: Counted>(
        &self,
        region: &str,
        service: Service,
        section: impl Future<Output = S>,
    ) -> S {
        self.emit(Progress::Started {
            region: region.to_string(),
            service,
        });
        let out = Box::pin(section).await;
        self.emit(Progress::Finished {
            region: region.to_string(),
            service,
            count: out.count(),
            errors: out.error_count(),
        });
        out
    }
}

pub(crate) struct Scope<'a> {
    pub ctx: &'a Ctx,
    pub region: String,
    sdk: SdkConfig,
}

impl Scope<'_> {
    pub fn sdk(&self) -> &SdkConfig {
        &self.sdk
    }

    pub fn arn(&self, service: &str, resource: &str) -> String {
        format!(
            "arn:{}:{service}:{}:{}:{resource}",
            self.ctx.partition, self.region, self.ctx.account
        )
    }

    pub fn filters(&self) -> &Filters {
        self.ctx.filters()
    }

    pub async fn call<F: Future>(&self, call: F) -> F::Output {
        self.ctx.call(call).await
    }
}

pub(crate) struct Section<T> {
    pub items: Option<Vec<T>>,
    pub errors: Vec<DiscoveryError>,
}

impl<T> Default for Section<T> {
    fn default() -> Self {
        Self {
            items: None,
            errors: Vec::new(),
        }
    }
}

impl<T> Section<T> {
    pub fn failed(error: DiscoveryError) -> Self {
        Self {
            items: None,
            errors: vec![error],
        }
    }

    pub fn ok(items: Vec<T>, errors: Vec<DiscoveryError>) -> Self {
        Self {
            items: Some(items),
            errors,
        }
    }
}

#[derive(Default)]
pub(crate) struct DbSections {
    pub rds: Option<Vec<RdsDatabase>>,
    pub documentdb: Option<Vec<DocumentDbCluster>>,
    pub neptune: Option<Vec<NeptuneCluster>>,
    pub retained_backups: Option<Vec<cd_model::RetainedBackup>>,
    pub errors: Vec<DiscoveryError>,
}

#[derive(Default)]
pub(crate) struct EbsSections {
    pub volumes: Option<Vec<EbsVolume>>,
    pub rbin_rules: Option<Vec<RecycleBinRule>>,
    pub errors: Vec<DiscoveryError>,
}

#[derive(Default)]
pub(crate) struct BackupSections {
    pub protected: Option<Vec<ProtectedResource>>,
    pub vaults: Option<Vec<BackupVault>>,
    pub errors: Vec<DiscoveryError>,
}

trait Counted {
    fn count(&self) -> Option<usize>;
    fn error_count(&self) -> usize;
}

impl<T> Counted for Section<T> {
    fn count(&self) -> Option<usize> {
        self.items.as_ref().map(Vec::len)
    }
    fn error_count(&self) -> usize {
        self.errors.len()
    }
}

fn sum_counts(counts: &[Option<usize>]) -> Option<usize> {
    counts.iter().flatten().copied().reduce(|a, b| a + b)
}

impl Counted for DbSections {
    fn count(&self) -> Option<usize> {
        sum_counts(&[
            self.rds.as_ref().map(Vec::len),
            self.documentdb.as_ref().map(Vec::len),
            self.neptune.as_ref().map(Vec::len),
        ])
    }
    fn error_count(&self) -> usize {
        self.errors.len()
    }
}

impl Counted for EbsSections {
    fn count(&self) -> Option<usize> {
        self.volumes.as_ref().map(Vec::len)
    }
    fn error_count(&self) -> usize {
        self.errors.len()
    }
}

impl Counted for BackupSections {
    fn count(&self) -> Option<usize> {
        self.protected.as_ref().map(Vec::len)
    }
    fn error_count(&self) -> usize {
        self.errors.len()
    }
}

impl Counted for (cd_model::AccountInfo, Vec<DiscoveryError>) {
    fn count(&self) -> Option<usize> {
        Some(1)
    }
    fn error_count(&self) -> usize {
        self.1.len()
    }
}

pub(crate) fn failure(
    service: &str,
    operation: &str,
    target: Option<&str>,
    err: impl ProvideErrorMetadata + std::error::Error,
) -> DiscoveryError {
    let detail = match (err.code(), err.message()) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code.to_string(),
        _ => DisplayErrorContext(&err).to_string(),
    };
    DiscoveryError {
        service: service.to_string(),
        operation: operation.to_string(),
        region: None,
        message: match target {
            Some(target) => format!("{target}: {detail}"),
            None => detail,
        },
    }
}

pub(crate) fn to_tags<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Tags {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

pub(crate) fn timestamp(value: &DateTime) -> Option<String> {
    value.fmt(Format::DateTime).ok()
}

async fn gated<T: Default>(enabled: bool, section: impl Future<Output = T>) -> T {
    if enabled {
        section.await
    } else {
        T::default()
    }
}

fn absorb<T>(into: &mut Option<Vec<T>>, from: Option<Vec<T>>) {
    if let Some(items) = from {
        into.get_or_insert_with(Vec::new).extend(items);
    }
}

struct RegionScan {
    region: String,
    ebs: EbsSections,
    ec2: Section<cd_model::Ec2Instance>,
    databases: DbSections,
    dynamodb: Section<cd_model::DynamoTable>,
    s3tables: Section<cd_model::TableBucket>,
    glue: Section<cd_model::GlueDatabase>,
    backup: BackupSections,
    trails: Section<cd_model::Trail>,
}

async fn scan_region(ctx: &Ctx, region: &str) -> RegionScan {
    let scope = ctx.scope(region);
    let s = ctx.config.services;
    let (ebs, ec2, databases, dynamodb, s3tables, glue, backup, trails) = tokio::join!(
        gated(
            s.ebs,
            ctx.track(region, Service::Ebs, ebs::discover(&scope))
        ),
        gated(
            s.ec2,
            ctx.track(region, Service::Ec2, ec2::discover(&scope))
        ),
        gated(
            s.rds || s.documentdb || s.neptune,
            ctx.track(region, Service::Rds, databases::discover(&scope))
        ),
        gated(
            s.dynamodb,
            ctx.track(region, Service::Dynamodb, dynamodb::discover(&scope))
        ),
        gated(
            s.s3tables,
            ctx.track(region, Service::S3tables, s3tables::discover(&scope))
        ),
        gated(
            s.glue,
            ctx.track(region, Service::Glue, glue::discover(&scope))
        ),
        gated(
            s.aws_backup,
            ctx.track(region, Service::AwsBackup, backup::discover(&scope))
        ),
        ctx.track(region, Service::Cloudtrail, cloudtrail::discover(&scope)),
    );
    RegionScan {
        region: region.to_string(),
        ebs,
        ec2,
        databases,
        dynamodb,
        s3tables,
        glue,
        backup,
        trails,
    }
}

pub async fn discover(
    config: &Config,
    profile: Option<&str>,
    progress: Option<UnboundedSender<Progress>>,
) -> Result<Inventory, DiscoverError> {
    let home = load_sdk_config(config, profile).await;

    let identity = aws_sdk_sts::Client::new(&home)
        .get_caller_identity()
        .send()
        .await
        .map_err(|e| DiscoverError::CallerIdentity(credentials_problem(e)))?;
    let account = identity.account().unwrap_or_default().to_string();
    let partition = identity
        .arn()
        .and_then(|arn| arn.split(':').nth(1))
        .unwrap_or("aws")
        .to_string();

    let ctx = Ctx {
        home,
        partition,
        account,
        config: config.clone(),
        limit: Semaphore::new(config.concurrency.max(1)),
        progress,
    };
    let generated_at = timestamp(&DateTime::from(SystemTime::now())).unwrap_or_default();

    let (account_info, mut errors) = ctx
        .track(GLOBAL, Service::Account, account::discover(&ctx))
        .await;
    let (regions, region_errors) = regions::resolve(&ctx).await;
    errors.extend(region_errors);
    ctx.emit(Progress::Regions(regions.clone()));

    let (s3, scans) = tokio::join!(
        gated(
            config.services.s3,
            ctx.track(GLOBAL, Service::S3, s3::discover(&ctx, &regions))
        ),
        futures::future::join_all(regions.iter().map(|r| Box::pin(scan_region(&ctx, r)))),
    );

    let mut inventory = Inventory {
        schema_version: SCHEMA_VERSION,
        account: account_info,
        regions,
        generated_at,
        depth: config.depth,
        filters: config.filters.clone(),
        s3: s3.items,
        ..Inventory::default()
    };
    errors.extend(s3.errors);

    for mut scan in scans {
        if let (Some(instances), Some(rules)) = (&mut scan.ec2.items, &scan.ebs.rbin_rules) {
            for instance in instances {
                instance.apply_rbin_rules(rules);
            }
        }
        let mut regional = Vec::new();
        absorb(&mut inventory.ebs, scan.ebs.volumes);
        absorb(&mut inventory.rbin_rules, scan.ebs.rbin_rules);
        regional.extend(scan.ebs.errors);
        absorb(&mut inventory.ec2, scan.ec2.items);
        regional.extend(scan.ec2.errors);
        absorb(&mut inventory.rds, scan.databases.rds);
        absorb(&mut inventory.documentdb, scan.databases.documentdb);
        absorb(&mut inventory.neptune, scan.databases.neptune);
        absorb(
            &mut inventory.rds_retained_backups,
            scan.databases.retained_backups,
        );
        regional.extend(scan.databases.errors);
        absorb(&mut inventory.dynamodb, scan.dynamodb.items);
        regional.extend(scan.dynamodb.errors);
        absorb(&mut inventory.s3tables, scan.s3tables.items);
        regional.extend(scan.s3tables.errors);
        absorb(&mut inventory.glue, scan.glue.items);
        regional.extend(scan.glue.errors);
        absorb(&mut inventory.aws_backup, scan.backup.protected);
        absorb(&mut inventory.backup_vaults, scan.backup.vaults);
        regional.extend(scan.backup.errors);
        absorb(&mut inventory.trails, scan.trails.items);
        regional.extend(scan.trails.errors);
        for error in &mut regional {
            error.region.get_or_insert_with(|| scan.region.clone());
        }
        errors.extend(regional);
    }

    inventory.errors = errors;
    inventory.finalize();
    Ok(inventory)
}

fn credentials_problem<E, R>(err: aws_sdk_sts::error::SdkError<E, R>) -> String
where
    E: ProvideErrorMetadata + std::error::Error + 'static,
    R: std::fmt::Debug,
{
    if let Some(service) = err.as_service_error() {
        return match (service.code(), service.message()) {
            (Some(code), Some(message)) => format!("{code}: {message}"),
            (Some(code), None) => code.to_string(),
            _ => DisplayErrorContext(&err).to_string(),
        };
    }
    let detail = DisplayErrorContext(&err).to_string();
    if detail.contains("CredentialsNotLoaded") || detail.contains("no providers in chain") {
        "none found. Run `aws configure` or `aws sso login`, set AWS_PROFILE, or pass --profile"
            .to_string()
    } else {
        detail
    }
}

async fn load_sdk_config(config: &Config, profile: Option<&str>) -> SdkConfig {
    let mut loader = aws_config::defaults(BehaviorVersion::latest())
        .retry_config(RetryConfig::adaptive().with_max_attempts(8));
    if let Some(profile) = profile {
        loader = loader.profile_name(profile);
    }
    if let Some(region) = config.requested_regions().first() {
        loader = loader.region(Region::new(region.clone()));
    }
    let sdk = loader.load().await;
    if sdk.region().is_some() {
        sdk
    } else {
        sdk.to_builder()
            .region(Region::new(FALLBACK_HOME_REGION))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_tag_pairs_to_a_sorted_map() {
        let tags = to_tags([("owner", "team-sre"), ("env", "prod")]);
        assert_eq!(
            tags.into_iter().collect::<Vec<_>>(),
            vec![
                ("env".to_string(), "prod".to_string()),
                ("owner".to_string(), "team-sre".to_string())
            ]
        );
    }

    #[test]
    fn timestamps_render_as_rfc3339() {
        let at = DateTime::from_secs(1_792_713_600);
        assert_eq!(timestamp(&at).as_deref(), Some("2026-10-23T00:00:00Z"));
    }

    #[test]
    fn actions_are_sorted_unique_and_read_only() {
        let all = actions();
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(all, sorted);
        for action in &all {
            let verb = action.split(':').nth(1).unwrap_or_default();
            assert!(
                ["Describe", "List", "Get"]
                    .iter()
                    .any(|prefix| verb.starts_with(prefix)),
                "{action} is not a read"
            );
        }
    }
}
