use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod account;
mod asset;
mod backup;
mod databases;
mod dynamodb;
mod ebs;
mod ec2;
mod iceberg;
pub mod progress;
mod s3;

pub use account::*;
pub use asset::*;
pub use backup::*;
pub use databases::*;
pub use dynamodb::*;
pub use ebs::*;
pub use ec2::*;
pub use iceberg::*;
pub use progress::Progress;
pub use s3::*;

pub type Tags = BTreeMap<String, String>;

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub region: Option<String>,
    pub regions: Vec<String>,
    pub profile: Option<String>,
    pub depth: Depth,
    pub concurrency: usize,
    pub services: Services,
    pub filters: Filters,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            region: None,
            regions: Vec::new(),
            profile: None,
            depth: Depth::Standard,
            concurrency: 16,
            services: Services::default(),
            filters: Filters::default(),
        }
    }
}

impl Config {
    pub fn requested_regions(&self) -> Vec<String> {
        let mut regions = self.regions.clone();
        if let Some(region) = &self.region {
            if !regions.contains(region) {
                regions.push(region.clone());
            }
        }
        regions
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    #[default]
    Standard,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Services {
    pub s3: bool,
    pub ebs: bool,
    pub ec2: bool,
    pub rds: bool,
    pub documentdb: bool,
    pub neptune: bool,
    pub dynamodb: bool,
    pub s3tables: bool,
    pub glue: bool,
    pub aws_backup: bool,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            s3: true,
            ebs: true,
            ec2: true,
            rds: true,
            documentdb: true,
            neptune: true,
            dynamodb: true,
            s3tables: true,
            glue: true,
            aws_backup: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_tag: Option<AwsTag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwsTag {
    pub key: String,
    pub value: String,
}

impl Filters {
    pub fn admits(&self, tags: &Tags) -> bool {
        match &self.aws_tag {
            None => true,
            Some(tag) => tags.get(&tag.key) == Some(&tag.value),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.aws_tag.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file at {path}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl Config {
    pub fn default_path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".config")
            .join("clumio-discover")
            .join("config.toml")
    }

    pub fn load(explicit: Option<&Path>) -> Result<Self, ConfigError> {
        match explicit {
            Some(path) => Self::load_from(path),
            None => {
                let path = Self::default_path();
                if path.exists() {
                    Self::load_from(&path)
                } else {
                    Ok(Self::default())
                }
            }
        }
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn parse(raw: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryError {
    pub service: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default)]
    pub schema_version: u32,
    pub account: AccountInfo,
    #[serde(default)]
    pub regions: Vec<String>,
    pub generated_at: String,
    #[serde(default)]
    pub depth: Depth,
    #[serde(default, skip_serializing_if = "Filters::is_empty")]
    pub filters: Filters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<Vec<S3Bucket>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ebs: Option<Vec<EbsVolume>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ec2: Option<Vec<Ec2Instance>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rds: Option<Vec<RdsDatabase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentdb: Option<Vec<DocumentDbCluster>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neptune: Option<Vec<NeptuneCluster>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rds_retained_backups: Option<Vec<RetainedBackup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamodb: Option<Vec<DynamoTable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3tables: Option<Vec<TableBucket>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glue: Option<Vec<GlueDatabase>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_backup: Option<Vec<ProtectedResource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_vaults: Option<Vec<BackupVault>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rbin_rules: Option<Vec<RecycleBinRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trails: Option<Vec<Trail>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<DiscoveryError>,
}

impl Inventory {
    pub fn summary(&self) -> Vec<(Service, usize)> {
        Service::ASSETS
            .into_iter()
            .filter_map(|s| self.rows(s).map(|rows| (s, rows.len())))
            .collect()
    }

    pub fn rows(&self, service: Service) -> Option<Vec<AssetRow>> {
        fn of<A: Asset>(section: &Option<Vec<A>>) -> Option<Vec<AssetRow>> {
            section
                .as_ref()
                .map(|items| items.iter().map(AssetRow::of).collect())
        }
        match service {
            Service::S3 => of(&self.s3),
            Service::Ebs => of(&self.ebs),
            Service::Ec2 => of(&self.ec2),
            Service::Rds => of(&self.rds),
            Service::Documentdb => of(&self.documentdb),
            Service::Neptune => of(&self.neptune),
            Service::Dynamodb => of(&self.dynamodb),
            Service::S3tables => of(&self.s3tables),
            Service::Glue => of(&self.glue),
            _ => None,
        }
    }

    pub fn finalize(&mut self) {
        let mut coverage: HashMap<String, AwsBackupCoverage> = HashMap::new();
        for resource in self.aws_backup.iter().flatten() {
            coverage
                .entry(resource.arn.clone())
                .or_default()
                .merge(&resource.coverage);
        }

        fn settle<A: Asset>(
            section: &mut Option<Vec<A>>,
            coverage: &HashMap<String, AwsBackupCoverage>,
            findings: &mut Vec<Finding>,
        ) {
            for asset in section.iter_mut().flatten() {
                let covered = asset
                    .backup_arns()
                    .iter()
                    .find_map(|arn| coverage.get(arn))
                    .cloned();
                if covered.is_some() {
                    asset.protection_mut().aws_backup = covered;
                }
                asset.protection_mut().settle();
                findings.extend(asset.findings());
                if asset.protection().status == ProtectionStatus::Unprotected
                    && asset.protection_scanned()
                {
                    findings.push(asset.finding(
                        Severity::High,
                        &format!("{}.unprotected", A::SERVICE.as_str()),
                        "no backup of any kind",
                    ));
                }
            }
        }

        let mut findings = Vec::new();
        settle(&mut self.s3, &coverage, &mut findings);
        settle(&mut self.ebs, &coverage, &mut findings);
        settle(&mut self.ec2, &coverage, &mut findings);
        settle(&mut self.rds, &coverage, &mut findings);
        settle(&mut self.documentdb, &coverage, &mut findings);
        settle(&mut self.neptune, &coverage, &mut findings);
        settle(&mut self.dynamodb, &coverage, &mut findings);
        settle(&mut self.s3tables, &coverage, &mut findings);
        settle(&mut self.glue, &coverage, &mut findings);
        findings.extend(self.account_findings());
        findings.sort_by(|a, b| {
            (a.severity, a.service, &a.region, &a.name)
                .cmp(&(b.severity, b.service, &b.region, &b.name))
        });
        self.findings = findings;
    }

    fn account_findings(&self) -> Vec<Finding> {
        let failed = |service: &str, region: &str| {
            self.errors
                .iter()
                .any(|e| e.service == service && e.region.as_deref() == Some(region))
        };
        let across = |severity, code: &str, service, regions: Vec<&String>, message: &str| {
            let list = regions
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let scope = if regions.len() == self.regions.len() && regions.len() > 1 {
                "all scanned regions".to_string()
            } else if regions.len() > 3 {
                format!("{} regions", regions.len())
            } else {
                list.clone()
            };
            Finding {
                severity,
                code: code.to_string(),
                service,
                arn: String::new(),
                name: scope.clone(),
                region: scope,
                message: format!("{message}: {list}"),
            }
        };
        let mut findings = Vec::new();

        if let Some(trails) = &self.trails {
            let multi = trails.iter().any(|t| t.multi_region == Some(true));
            if !multi && self.account.org_id.is_none() {
                let unlogged: Vec<&String> = self
                    .regions
                    .iter()
                    .filter(|r| !failed("cloudtrail", r) && !trails.iter().any(|t| &t.region == *r))
                    .collect();
                if !unlogged.is_empty() {
                    findings.push(across(
                        Severity::Low,
                        "cloudtrail.region_not_logged",
                        Service::Cloudtrail,
                        unlogged,
                        "no CloudTrail trail logs",
                    ));
                }
            }
        }

        if let Some(vaults) = &self.backup_vaults {
            let vaultless: Vec<&String> = self
                .regions
                .iter()
                .filter(|r| !failed("backup", r) && !vaults.iter().any(|v| &v.region == *r))
                .collect();
            if !vaultless.is_empty() {
                findings.push(across(
                    Severity::Info,
                    "aws_backup.no_vault",
                    Service::AwsBackup,
                    vaultless,
                    "AWS Backup has no vault in",
                ));
            }
            for vault in vaults
                .iter()
                .filter(|v| v.locked != Some(true) && v.recovery_points.unwrap_or(0) > 0)
            {
                findings.push(Finding {
                    severity: Severity::Low,
                    code: "aws_backup.vault_unlocked".to_string(),
                    service: Service::AwsBackup,
                    arn: vault.arn.clone(),
                    name: vault.name.clone(),
                    region: vault.region.clone(),
                    message: "vault holds recovery points without Vault Lock".to_string(),
                });
            }
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_enables_every_service() {
        let config = Config::parse("").unwrap();
        assert_eq!(config, Config::default());
        assert!(config.services.s3 && config.services.glue && config.services.aws_backup);
        assert_eq!(config.depth, Depth::Standard);
        assert_eq!(config.concurrency, 16);
        assert!(config.requested_regions().is_empty());
        assert!(config.filters.is_empty());
    }

    #[test]
    fn parses_the_documented_config() {
        let config = Config::parse(
            r#"
            regions = ["ap-southeast-2", "us-east-1"]
            profile = "default"
            depth = "deep"
            concurrency = 8

            [services]
            s3tables = false

            [filters]
            aws_tag = { key = "team", value = "data-platform" }
            "#,
        )
        .unwrap();

        assert_eq!(
            config.requested_regions(),
            vec!["ap-southeast-2", "us-east-1"]
        );
        assert_eq!(config.profile.as_deref(), Some("default"));
        assert_eq!(config.depth, Depth::Deep);
        assert_eq!(config.concurrency, 8);
        assert!(!config.services.s3tables);
        assert!(config.services.neptune);
        assert_eq!(
            config.filters.aws_tag,
            Some(AwsTag {
                key: "team".into(),
                value: "data-platform".into()
            })
        );
    }

    #[test]
    fn single_region_shorthand_still_loads() {
        let config = Config::parse("region = \"ap-southeast-2\"").unwrap();
        assert_eq!(config.requested_regions(), vec!["ap-southeast-2"]);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(Config::parse("[services]\nlambda = true").is_err());
        assert!(Config::parse("[services]\ntag_sweep = true").is_err());
        assert!(Config::parse("role_arn = \"arn:aws:iam::1:role/x\"").is_err());
    }

    #[test]
    fn explicit_missing_config_is_an_error() {
        let err = Config::load(Some(Path::new("/nonexistent/clumio-discover.toml")));
        assert!(matches!(err, Err(ConfigError::Read { .. })));
    }

    #[test]
    fn tag_filter_requires_exact_key_and_value() {
        let filters = Filters {
            aws_tag: Some(AwsTag {
                key: "Owner".into(),
                value: "platform".into(),
            }),
        };
        let tagged = |k: &str, v: &str| Tags::from([(k.to_string(), v.to_string())]);

        assert!(filters.admits(&tagged("Owner", "platform")));
        assert!(!filters.admits(&tagged("Owner", "someone-else")));
        assert!(!filters.admits(&tagged("owner", "platform")));
        assert!(!filters.admits(&Tags::new()));
        assert!(Filters::default().admits(&Tags::new()));
    }

    fn volume(id: &str) -> EbsVolume {
        EbsVolume {
            id: id.into(),
            arn: format!("arn:aws:ec2:ap-southeast-2:1:volume/{id}"),
            region: "ap-southeast-2".into(),
            size_gib: Some(8),
            encrypted: Some(true),
            snapshots: Some(SnapshotSummary::default()),
            ..EbsVolume::default()
        }
    }

    #[test]
    fn finalize_joins_aws_backup_by_arn_and_flags_unprotected() {
        let mut inventory = Inventory {
            ebs: Some(vec![volume("vol-a"), volume("vol-b")]),
            aws_backup: Some(vec![ProtectedResource {
                arn: "arn:aws:ec2:ap-southeast-2:1:volume/vol-a".into(),
                region: "ap-southeast-2".into(),
                resource_type: Some("EBS".into()),
                resource_name: None,
                coverage: AwsBackupCoverage {
                    last_backup: Some("2026-10-22T00:00:00Z".into()),
                    recovery_points: 3,
                    ..AwsBackupCoverage::default()
                },
                ..ProtectedResource::default()
            }]),
            ..Inventory::default()
        };
        inventory.finalize();

        let ebs = inventory.ebs.as_ref().unwrap();
        assert!(ebs.iter().all(|v| v.snapshots.is_some()));
        assert_eq!(ebs[0].protection.status, ProtectionStatus::Protected);
        assert!(ebs[0].protection.aws_backup.is_some());
        assert_eq!(ebs[1].protection.status, ProtectionStatus::Unprotected);
        let codes: Vec<_> = inventory
            .findings
            .iter()
            .map(|f| (f.code.as_str(), f.name.as_str()))
            .collect();
        assert!(codes.contains(&("ebs.unprotected", "vol-b")));
        assert!(!codes.contains(&("ebs.unprotected", "vol-a")));
    }

    #[test]
    fn failed_snapshot_listing_suppresses_the_unprotected_finding() {
        let mut inventory = Inventory {
            ebs: Some(vec![EbsVolume {
                snapshots: None,
                ..volume("vol-c")
            }]),
            ..Inventory::default()
        };
        inventory.finalize();
        assert!(inventory
            .findings
            .iter()
            .all(|f| f.code != "ebs.unprotected"));
    }

    #[test]
    fn region_checks_fold_into_one_finding_per_code() {
        let mut inventory = Inventory {
            regions: vec!["ap-southeast-2".into(), "us-east-1".into()],
            trails: Some(Vec::new()),
            backup_vaults: Some(Vec::new()),
            errors: vec![DiscoveryError {
                service: "backup".into(),
                operation: "ListBackupVaults".into(),
                region: Some("us-east-1".into()),
                message: "AccessDenied".into(),
            }],
            ..Inventory::default()
        };
        inventory.finalize();
        let found: Vec<_> = inventory
            .findings
            .iter()
            .map(|f| (f.code.as_str(), f.region.as_str()))
            .collect();
        assert_eq!(
            found,
            vec![
                ("cloudtrail.region_not_logged", "all scanned regions"),
                ("aws_backup.no_vault", "ap-southeast-2"),
            ]
        );
    }

    #[test]
    fn disabled_sections_are_omitted_from_json() {
        let inventory = Inventory {
            schema_version: SCHEMA_VERSION,
            account: AccountInfo {
                id: "1".into(),
                ..AccountInfo::default()
            },
            regions: vec!["ap-southeast-2".into()],
            generated_at: "2026-10-23T00:00:00Z".into(),
            s3: Some(Vec::new()),
            ..Inventory::default()
        };
        let json = serde_json::to_value(&inventory).unwrap();

        assert_eq!(json["s3"], serde_json::json!([]));
        assert!(json.get("dynamodb").is_none());
        assert!(json.get("errors").is_none());
        assert!(json.get("filters").is_none());
        assert_eq!(inventory.summary(), vec![(Service::S3, 0)]);

        let back: Inventory = serde_json::from_value(json).unwrap();
        assert_eq!(back, inventory);
    }
}
