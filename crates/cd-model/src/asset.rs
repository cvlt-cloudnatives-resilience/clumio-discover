use serde::{Deserialize, Serialize};

use crate::Tags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Account,
    S3,
    Ebs,
    Ec2,
    RecycleBin,
    Rds,
    Documentdb,
    Neptune,
    Dynamodb,
    S3tables,
    Glue,
    AwsBackup,
    Cloudtrail,
}

impl Service {
    pub const ASSETS: [Service; 9] = [
        Service::S3,
        Service::Ebs,
        Service::Ec2,
        Service::Rds,
        Service::Dynamodb,
        Service::Documentdb,
        Service::Neptune,
        Service::S3tables,
        Service::Glue,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Service::Account => "account",
            Service::S3 => "s3",
            Service::Ebs => "ebs",
            Service::Ec2 => "ec2",
            Service::RecycleBin => "rbin",
            Service::Rds => "rds",
            Service::Documentdb => "documentdb",
            Service::Neptune => "neptune",
            Service::Dynamodb => "dynamodb",
            Service::S3tables => "s3tables",
            Service::Glue => "glue",
            Service::AwsBackup => "aws_backup",
            Service::Cloudtrail => "cloudtrail",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Service::Account => "Account",
            Service::S3 => "S3",
            Service::Ebs => "EBS",
            Service::Ec2 => "EC2",
            Service::RecycleBin => "Recycle Bin",
            Service::Rds => "RDS",
            Service::Documentdb => "DocumentDB",
            Service::Neptune => "Neptune",
            Service::Dynamodb => "DynamoDB",
            Service::S3tables => "S3 Tables",
            Service::Glue => "Glue Iceberg",
            Service::AwsBackup => "AWS Backup",
            Service::Cloudtrail => "CloudTrail",
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionStatus {
    AwsBackup,
    #[default]
    #[serde(alias = "native_only")]
    Unprotected,
}

impl ProtectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            ProtectionStatus::AwsBackup => "AWS Backup",
            ProtectionStatus::Unprotected => "Not protected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeKind {
    EbsSnapshot,
    Ami,
    RdsAutomatedBackup,
    RdsSnapshot,
    RdsClusterSnapshot,
    DynamodbPitr,
    DynamodbBackup,
    S3Versioning,
    S3Replication,
    S3ObjectLock,
}

impl NativeKind {
    pub fn is_backup(self) -> bool {
        !matches!(
            self,
            NativeKind::S3Versioning | NativeKind::S3Replication | NativeKind::S3ObjectLock
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            NativeKind::EbsSnapshot => "EBS snapshots",
            NativeKind::Ami => "AMIs",
            NativeKind::RdsAutomatedBackup => "automated backups",
            NativeKind::RdsSnapshot | NativeKind::RdsClusterSnapshot => "manual snapshots",
            NativeKind::DynamodbPitr => "point-in-time recovery",
            NativeKind::DynamodbBackup => "on-demand backups",
            NativeKind::S3Versioning => "versioning",
            NativeKind::S3Replication => "replication",
            NativeKind::S3ObjectLock => "object lock",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeBackup {
    pub kind: NativeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsBackupCoverage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backup: Option<String>,
    #[serde(default)]
    pub recovery_points: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_point_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vaults: Vec<String>,
}

impl AwsBackupCoverage {
    pub fn merge(&mut self, other: &AwsBackupCoverage) {
        self.last_backup = self.last_backup.take().max(other.last_backup.clone());
        self.recovery_points += other.recovery_points;
        self.recovery_point_bytes = match (self.recovery_point_bytes, other.recovery_point_bytes) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        for vault in &other.vaults {
            if !self.vaults.contains(vault) {
                self.vaults.push(vault.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protection {
    #[serde(default)]
    pub status: ProtectionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws_backup: Option<AwsBackupCoverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native: Vec<NativeBackup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rbin_rules: Vec<String>,
}

impl Protection {
    pub fn settle(&mut self) {
        self.status = if self.aws_backup.is_some() {
            ProtectionStatus::AwsBackup
        } else {
            ProtectionStatus::Unprotected
        };
    }

    pub fn native_backups(&self) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        for kind in self.native.iter().map(|n| n.kind).filter(|k| k.is_backup()) {
            if !kinds.contains(&kind.label()) {
                kinds.push(kind.label());
            }
        }
        kinds
    }

    pub fn last_backup(&self) -> Option<&str> {
        self.aws_backup
            .as_ref()
            .and_then(|c| c.last_backup.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub service: Service,
    pub arn: String,
    pub name: String,
    pub region: String,
    pub message: String,
}

pub trait Asset: Serialize {
    const SERVICE: Service;

    fn name(&self) -> String;
    fn arn(&self) -> &str;
    fn region(&self) -> &str;
    fn tags(&self) -> &crate::Tags;
    fn protection(&self) -> &Protection;
    fn protection_mut(&mut self) -> &mut Protection;

    fn size_bytes(&self) -> Option<u64> {
        None
    }

    fn written_bytes(&self) -> Option<u64> {
        None
    }

    fn summary(&self) -> String {
        String::new()
    }

    fn findings(&self) -> Vec<Finding> {
        Vec::new()
    }

    fn protection_scanned(&self) -> bool {
        true
    }

    fn backup_arns(&self) -> Vec<String> {
        vec![self.arn().to_string()]
    }

    fn finding(&self, severity: Severity, code: &str, message: impl Into<String>) -> Finding
    where
        Self: Sized,
    {
        Finding {
            severity,
            code: code.to_string(),
            service: Self::SERVICE,
            arn: self.arn().to_string(),
            name: self.name(),
            region: self.region().to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetRow {
    pub service: Service,
    pub name: String,
    pub arn: String,
    pub region: String,
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_bytes: Option<u64>,
    pub status: ProtectionStatus,
    pub last_backup: Option<String>,
    pub summary: String,
    pub tags: Tags,
    pub detail: serde_json::Value,
}

impl AssetRow {
    pub fn of<A: Asset>(asset: &A) -> Self {
        let protection = asset.protection();
        Self {
            service: A::SERVICE,
            name: asset.name(),
            arn: asset.arn().to_string(),
            region: asset.region().to_string(),
            size_bytes: asset.size_bytes(),
            written_bytes: asset.written_bytes(),
            status: protection.status,
            last_backup: protection.last_backup().map(str::to_string),
            summary: asset.summary(),
            tags: asset.tags().clone(),
            detail: serde_json::to_value(asset).unwrap_or(serde_json::Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(kind: NativeKind, latest: &str) -> NativeBackup {
        NativeBackup {
            kind,
            count: Some(1),
            latest: Some(latest.into()),
            detail: None,
        }
    }

    #[test]
    fn only_aws_backup_protects() {
        let mut p = Protection::default();
        p.settle();
        assert_eq!(p.status, ProtectionStatus::Unprotected);
        assert!(p.native_backups().is_empty());

        p.native
            .push(native(NativeKind::S3Versioning, "2026-10-01T00:00:00Z"));
        p.settle();
        assert_eq!(p.status, ProtectionStatus::Unprotected);
        assert!(p.native_backups().is_empty());

        for kind in [
            NativeKind::RdsSnapshot,
            NativeKind::RdsClusterSnapshot,
            NativeKind::RdsAutomatedBackup,
        ] {
            p.native.push(native(kind, "2026-10-02T00:00:00Z"));
        }
        p.settle();
        assert_eq!(p.status, ProtectionStatus::Unprotected);
        assert_eq!(
            p.native_backups(),
            vec!["manual snapshots", "automated backups"]
        );
        assert_eq!(p.last_backup(), None);

        p.aws_backup = Some(AwsBackupCoverage {
            last_backup: Some("2026-10-03T00:00:00Z".into()),
            ..AwsBackupCoverage::default()
        });
        p.settle();
        assert_eq!(p.status, ProtectionStatus::AwsBackup);
        assert_eq!(p.last_backup(), Some("2026-10-03T00:00:00Z"));
    }

    #[test]
    fn reads_statuses_from_older_inventory_files() {
        for (json, status) in [
            ("\"aws_backup\"", ProtectionStatus::AwsBackup),
            ("\"native_only\"", ProtectionStatus::Unprotected),
            ("\"unprotected\"", ProtectionStatus::Unprotected),
        ] {
            assert_eq!(
                serde_json::from_str::<ProtectionStatus>(json).unwrap(),
                status
            );
        }
    }

    #[test]
    fn coverage_merges_cross_region_copies() {
        let mut a = AwsBackupCoverage {
            last_backup: Some("2026-10-01T00:00:00Z".into()),
            recovery_points: 2,
            recovery_point_bytes: Some(10),
            vaults: vec!["Default".into()],
        };
        a.merge(&AwsBackupCoverage {
            last_backup: Some("2026-10-05T00:00:00Z".into()),
            recovery_points: 1,
            recovery_point_bytes: None,
            vaults: vec!["Default".into(), "dr".into()],
        });
        assert_eq!(a.last_backup.as_deref(), Some("2026-10-05T00:00:00Z"));
        assert_eq!(a.recovery_points, 3);
        assert_eq!(a.recovery_point_bytes, Some(10));
        assert_eq!(a.vaults, vec!["Default", "dr"]);
    }
}
