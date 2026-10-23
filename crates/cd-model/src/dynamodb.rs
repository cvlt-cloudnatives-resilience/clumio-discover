use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, Service, Severity, Tags};

pub const LEGACY_GLOBAL_TABLE_VERSION: &str = "2017.11.29";

const KEY_LOST_STATUSES: [&str; 3] = [
    "INACCESSIBLE_ENCRYPTION_CREDENTIALS",
    "ARCHIVING",
    "ARCHIVED",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoTable {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub billing_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub item_count: Option<i64>,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_capacity_units: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_capacity_units: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gsi_count: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lsi_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_protection: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_manager: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<DynamoStream>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_table_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replicas: Vec<DynamoReplica>,
    #[serde(default)]
    pub pitr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitr_earliest_restorable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitr_latest_restorable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitr_recovery_period_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backups: Option<DynamoBackups>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_attribute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributor_insights: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_policy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoscaling: Option<DynamoAutoscaling>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoStream {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoReplica {
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_capacity_units: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_capacity_units: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_autoscaling: Option<DynamoCapacityRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_autoscaling: Option<DynamoCapacityRange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoBackups {
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub user: u64,
    #[serde(default)]
    pub system: u64,
    #[serde(default)]
    pub aws_backup: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoAutoscaling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<DynamoCapacityRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<DynamoCapacityRange>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indexes: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub policies: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamoCapacityRange {
    pub min: i64,
    pub max: i64,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl DynamoTable {
    pub fn pitr_enabled(&self) -> bool {
        self.pitr.as_deref() == Some("ENABLED")
    }

    pub fn is_legacy_global_table(&self) -> bool {
        self.global_table_version.as_deref() == Some(LEGACY_GLOBAL_TABLE_VERSION)
    }
}

impl Asset for DynamoTable {
    const SERVICE: Service = Service::Dynamodb;

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
        self.size_bytes.map(|b| b.max(0) as u64)
    }

    fn summary(&self) -> String {
        let mut parts = vec![match self.billing_mode.as_str() {
            "PAY_PER_REQUEST" => "on-demand".to_string(),
            "" => "unknown billing".to_string(),
            other => other.to_lowercase(),
        }];
        if let Some(bytes) = self.size_bytes() {
            parts.push(human_bytes(bytes));
        }
        if self.pitr.is_some() {
            parts.push(
                if self.pitr_enabled() {
                    "PITR on"
                } else {
                    "PITR off"
                }
                .to_string(),
            );
        }
        if let Some(backups) = &self.backups {
            parts.push(match backups.count {
                1 => "1 backup".to_string(),
                n => format!("{n} backups"),
            });
        }
        if !self.replicas.is_empty() {
            parts.push(format!("{} replica(s)", self.replicas.len()));
        }
        parts.join(" · ")
    }

    fn findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();
        if self.pitr.is_some() && !self.pitr_enabled() {
            findings.push(self.finding(
                Severity::Medium,
                "dynamodb.pitr_disabled",
                "point-in-time recovery is off",
            ));
        }
        if self.deletion_protection == Some(false) {
            findings.push(self.finding(
                Severity::Low,
                "dynamodb.deletion_protection_disabled",
                "deletion protection is off",
            ));
        }
        if let Some(status) = self
            .status
            .as_deref()
            .filter(|s| KEY_LOST_STATUSES.contains(s))
        {
            findings.push(self.finding(
                Severity::High,
                "dynamodb.kms_key_inaccessible",
                format!("table is {status}: its KMS key is unreachable"),
            ));
        }
        if self.is_legacy_global_table() {
            findings.push(self.finding(
                Severity::Info,
                "dynamodb.legacy_global_table",
                "global table version 2017.11.29 (legacy); AWS recommends upgrading to 2019.11.21",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> DynamoTable {
        serde_json::from_value(serde_json::json!({
            "name": "orders",
            "arn": "arn:aws:dynamodb:ap-southeast-2:1:table/orders",
            "region": "ap-southeast-2",
        }))
        .unwrap()
    }

    fn codes(table: &DynamoTable) -> Vec<String> {
        table.findings().into_iter().map(|f| f.code).collect()
    }

    #[test]
    fn minimal_json_still_loads() {
        let t: DynamoTable = serde_json::from_value(serde_json::json!({
            "name": "orders",
            "arn": "arn:aws:dynamodb:ap-southeast-2:1:table/orders",
            "region": "ap-southeast-2",
            "status": "ACTIVE",
            "billing_mode": "PROVISIONED",
            "pitr": "ENABLED",
            "item_count": 3,
            "size_bytes": 10,
            "tags": {},
            "protection": {"status": "unprotected"}
        }))
        .unwrap();
        assert!(t.pitr_enabled());
        assert!(t.replicas.is_empty());
        assert_eq!(t.backups, None);
    }

    #[test]
    fn summary_reads_like_the_console() {
        let mut t = table();
        t.billing_mode = "PAY_PER_REQUEST".into();
        t.size_bytes = Some(1_288_490_189);
        t.pitr = Some("ENABLED".into());
        t.backups = Some(DynamoBackups {
            count: 3,
            ..DynamoBackups::default()
        });
        assert_eq!(t.summary(), "on-demand · 1.2 GiB · PITR on · 3 backups");

        t.billing_mode = "PROVISIONED".into();
        t.size_bytes = None;
        t.pitr = Some("DISABLED".into());
        t.backups = Some(DynamoBackups {
            count: 1,
            ..DynamoBackups::default()
        });
        t.replicas = vec![DynamoReplica {
            region: "us-east-1".into(),
            ..DynamoReplica::default()
        }];
        assert_eq!(
            t.summary(),
            "provisioned · PITR off · 1 backup · 1 replica(s)"
        );
    }

    #[test]
    fn pitr_and_deletion_protection_findings() {
        let mut t = table();
        assert!(codes(&t).is_empty(), "unknown state is not a finding");

        t.pitr = Some("DISABLED".into());
        t.deletion_protection = Some(false);
        assert_eq!(
            codes(&t),
            vec![
                "dynamodb.pitr_disabled",
                "dynamodb.deletion_protection_disabled"
            ]
        );
        let pitr = &t.findings()[0];
        assert_eq!(pitr.severity, Severity::Medium);
        assert_eq!(pitr.name, "orders");

        t.pitr = Some("ENABLED".into());
        t.deletion_protection = Some(true);
        assert!(codes(&t).is_empty());
    }

    #[test]
    fn lost_kms_key_and_legacy_global_table_findings() {
        let mut t = table();
        t.status = Some("INACCESSIBLE_ENCRYPTION_CREDENTIALS".into());
        t.global_table_version = Some(LEGACY_GLOBAL_TABLE_VERSION.into());
        let findings = t.findings();
        assert_eq!(findings[0].code, "dynamodb.kms_key_inaccessible");
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[1].code, "dynamodb.legacy_global_table");
        assert_eq!(findings[1].severity, Severity::Info);

        t.status = Some("ACTIVE".into());
        t.global_table_version = Some("2019.11.21".into());
        assert!(codes(&t).is_empty());
    }

    #[test]
    fn human_bytes_uses_binary_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024 * 1024), "5.0 TiB");
    }
}
