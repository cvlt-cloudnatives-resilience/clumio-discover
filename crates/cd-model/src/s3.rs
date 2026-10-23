use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, Service, Severity, Tags};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Bucket {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub versioning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa_delete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<S3Encryption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_access_block: Option<PublicAccessBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_policy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replication: Option<S3Replication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<S3Lifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<S3Logging>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_lock: Option<S3ObjectLock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<S3Notifications>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics_configurations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_configurations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_count: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub storage_classes: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mraps: Vec<String>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Encryption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_key: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicAccessBlock {
    #[serde(default)]
    pub block_public_acls: bool,
    #[serde(default)]
    pub ignore_public_acls: bool,
    #[serde(default)]
    pub block_public_policy: bool,
    #[serde(default)]
    pub restrict_public_buckets: bool,
}

impl PublicAccessBlock {
    pub fn fully_on(&self) -> bool {
        self.off().is_empty()
    }

    pub fn off(&self) -> Vec<&'static str> {
        [
            (self.block_public_acls, "BlockPublicAcls"),
            (self.ignore_public_acls, "IgnorePublicAcls"),
            (self.block_public_policy, "BlockPublicPolicy"),
            (self.restrict_public_buckets, "RestrictPublicBuckets"),
        ]
        .into_iter()
        .filter(|(on, _)| !on)
        .map(|(_, name)| name)
        .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Replication {
    #[serde(default)]
    pub rules: u32,
    #[serde(default)]
    pub enabled_rules: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub destinations: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Lifecycle {
    #[serde(default)]
    pub rules: u32,
    #[serde(default)]
    pub transitions: bool,
    #[serde(default)]
    pub expiration: bool,
    #[serde(default)]
    pub noncurrent_expiration: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Logging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_bucket: Option<String>,
}

impl S3Logging {
    pub fn enabled(&self) -> bool {
        self.target_bucket.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3ObjectLock {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub years: Option<i32>,
}

impl S3ObjectLock {
    pub fn describe(&self) -> String {
        let period = match (self.days, self.years) {
            (Some(days), _) => Some(format!("{days}d")),
            (None, Some(years)) => Some(format!("{years}y")),
            (None, None) => None,
        };
        match (&self.mode, period) {
            (Some(mode), Some(period)) => format!("{mode} {period}"),
            (Some(mode), None) => mode.clone(),
            (None, _) => "no default retention".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Notifications {
    #[serde(default)]
    pub eventbridge: bool,
    #[serde(default)]
    pub topics: u32,
    #[serde(default)]
    pub queues: u32,
    #[serde(default)]
    pub lambdas: u32,
}

impl Asset for S3Bucket {
    const SERVICE: Service = Service::S3;

    fn name(&self) -> String {
        self.name.clone()
    }
    fn arn(&self) -> &str {
        &self.arn
    }
    fn region(&self) -> &str {
        &self.region
    }
    fn tags(&self) -> &Tags {
        &self.tags
    }
    fn protection(&self) -> &Protection {
        &self.protection
    }
    fn protection_mut(&mut self) -> &mut Protection {
        &mut self.protection
    }
    fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
    }

    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(bytes) = self.size_bytes {
            parts.push(human_bytes(bytes));
        }
        if let Some(count) = self.object_count {
            parts.push(format!("{} obj", thousands(count)));
        }
        if let Some(versioning) = &self.versioning {
            parts.push(match versioning.as_str() {
                "Enabled" => "versioning on".to_string(),
                "Suspended" => "versioning suspended".to_string(),
                _ => "versioning off".to_string(),
            });
        }
        parts.join(" · ")
    }

    fn findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();
        if let Some(block) = &self.public_access_block {
            if !block.fully_on() {
                findings.push(self.finding(
                    Severity::High,
                    "s3.public_access_block_off",
                    format!(
                        "bucket public access block not fully on (off: {})",
                        block.off().join(", ")
                    ),
                ));
            }
        }
        match self.versioning.as_deref() {
            None | Some("Enabled") => {}
            Some(status) => findings.push(self.finding(
                Severity::Medium,
                "s3.versioning_disabled",
                if status == "Suspended" {
                    "versioning is suspended"
                } else {
                    "versioning is not enabled"
                },
            )),
        }
        if self
            .encryption
            .as_ref()
            .is_some_and(|e| e.algorithm.is_none())
        {
            findings.push(self.finding(
                Severity::Medium,
                "s3.no_default_encryption",
                "no default encryption configured",
            ));
        }
        if self.logging.as_ref().is_some_and(|l| !l.enabled()) {
            findings.push(self.finding(
                Severity::Low,
                "s3.access_logging_off",
                "server access logging is off",
            ));
        }
        if self.object_lock.as_ref().is_some_and(|l| !l.enabled) {
            findings.push(self.finding(
                Severity::Info,
                "s3.object_lock_off",
                "object lock is not enabled",
            ));
        }
        findings
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket() -> S3Bucket {
        S3Bucket {
            name: "data".into(),
            arn: "arn:aws:s3:::data".into(),
            region: "ap-southeast-2".into(),
            created_at: None,
            versioning: Some("Enabled".into()),
            mfa_delete: Some(false),
            encryption: Some(S3Encryption {
                algorithm: Some("AES256".into()),
                ..S3Encryption::default()
            }),
            public_access_block: Some(PublicAccessBlock {
                block_public_acls: true,
                ignore_public_acls: true,
                block_public_policy: true,
                restrict_public_buckets: true,
            }),
            has_policy: Some(false),
            replication: Some(S3Replication::default()),
            lifecycle: Some(S3Lifecycle::default()),
            logging: Some(S3Logging {
                target_bucket: Some("logs".into()),
            }),
            object_lock: Some(S3ObjectLock {
                enabled: true,
                ..S3ObjectLock::default()
            }),
            notifications: Some(S3Notifications::default()),
            metrics_configurations: Some(0),
            inventory_configurations: Some(0),
            size_bytes: Some(1_288_490_189),
            object_count: Some(3_400),
            storage_classes: BTreeMap::new(),
            mraps: Vec::new(),
            tags: Tags::new(),
            protection: Protection::default(),
        }
    }

    fn codes(bucket: &S3Bucket) -> Vec<String> {
        bucket.findings().into_iter().map(|f| f.code).collect()
    }

    #[test]
    fn well_configured_bucket_has_no_findings() {
        assert!(codes(&bucket()).is_empty());
    }

    #[test]
    fn flags_each_misconfiguration() {
        let b = S3Bucket {
            versioning: Some("Suspended".into()),
            encryption: Some(S3Encryption::default()),
            public_access_block: Some(PublicAccessBlock {
                block_public_acls: true,
                ..PublicAccessBlock::default()
            }),
            logging: Some(S3Logging::default()),
            object_lock: Some(S3ObjectLock::default()),
            ..bucket()
        };
        let findings = b.findings();
        let got: Vec<_> = findings
            .iter()
            .map(|f| (f.severity, f.code.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (Severity::High, "s3.public_access_block_off"),
                (Severity::Medium, "s3.versioning_disabled"),
                (Severity::Medium, "s3.no_default_encryption"),
                (Severity::Low, "s3.access_logging_off"),
                (Severity::Info, "s3.object_lock_off"),
            ]
        );
        assert!(findings[0]
            .message
            .ends_with("(off: IgnorePublicAcls, BlockPublicPolicy, RestrictPublicBuckets)"));
        assert_eq!(findings[1].message, "versioning is suspended");
    }

    #[test]
    fn unread_settings_raise_no_findings() {
        let b = S3Bucket {
            versioning: None,
            encryption: None,
            public_access_block: None,
            logging: None,
            object_lock: None,
            ..bucket()
        };
        assert!(codes(&b).is_empty());
    }

    #[test]
    fn summary_is_short() {
        assert_eq!(bucket().summary(), "1.2 GiB · 3,400 obj · versioning on");
        let bare = S3Bucket {
            size_bytes: None,
            object_count: None,
            versioning: Some("Disabled".into()),
            ..bucket()
        };
        assert_eq!(bare.summary(), "versioning off");
    }

    #[test]
    fn formats_sizes_and_counts() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024u64.pow(4)), "5.0 TiB");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn describes_object_lock_retention() {
        let lock = |mode: Option<&str>, days, years| S3ObjectLock {
            enabled: true,
            mode: mode.map(str::to_string),
            days,
            years,
        };
        assert_eq!(
            lock(Some("COMPLIANCE"), Some(30), None).describe(),
            "COMPLIANCE 30d"
        );
        assert_eq!(
            lock(Some("GOVERNANCE"), None, Some(1)).describe(),
            "GOVERNANCE 1y"
        );
        assert_eq!(lock(None, None, None).describe(), "no default retention");
    }

    #[test]
    fn older_json_still_loads() {
        let b: S3Bucket = serde_json::from_value(serde_json::json!({
            "name": "data",
            "arn": "arn:aws:s3:::data",
            "region": "us-east-1",
            "created_at": null,
            "versioning": "Enabled",
            "tags": {}
        }))
        .unwrap();
        assert_eq!(b.versioning.as_deref(), Some("Enabled"));
        assert!(b.encryption.is_none() && b.mraps.is_empty());
    }
}
