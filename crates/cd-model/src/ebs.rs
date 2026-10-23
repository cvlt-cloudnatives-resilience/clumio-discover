use serde::{Deserialize, Serialize};

use crate::{Asset, Finding, Protection, Service, Severity, Tags};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EbsVolume {
    pub id: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub volume_type: Option<String>,
    #[serde(default)]
    pub size_gib: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iops: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_attach: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_managed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_state: Option<String>,
    #[serde(default)]
    pub attached_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshots: Option<SnapshotSummary>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub rbin_scanned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_enable_io: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub product_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_snapshots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_snapshot_accounts: Vec<String>,
    #[serde(default)]
    pub tags: Tags,
    #[serde(default)]
    pub protection: Protection,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    #[serde(default)]
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_full_bytes: Option<u64>,
    #[serde(default)]
    pub locked: u64,
    #[serde(default)]
    pub fast_restore: u64,
}

impl Asset for EbsVolume {
    const SERVICE: Service = Service::Ebs;

    fn name(&self) -> String {
        self.tags
            .get("Name")
            .cloned()
            .unwrap_or_else(|| self.id.clone())
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
    fn protection_scanned(&self) -> bool {
        self.snapshots.is_some()
    }
    fn size_bytes(&self) -> Option<u64> {
        self.size_gib.map(|gib| gib as u64 * 1024 * 1024 * 1024)
    }

    fn summary(&self) -> String {
        let shape = [
            self.volume_type.clone(),
            self.size_gib.map(|gib| format!("{gib} GiB")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        let snaps = self.snapshots.as_ref().map(|s| match s.count {
            1 => "1 snap".to_string(),
            n => format!("{n} snaps"),
        });
        [
            Some(shape).filter(|s| !s.is_empty()),
            self.state.clone(),
            snaps,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }

    fn findings(&self) -> Vec<Finding> {
        let mut findings = Vec::new();
        if self.encrypted == Some(false) {
            findings.push(self.finding(
                Severity::Medium,
                "ebs.unencrypted",
                "volume is not encrypted",
            ));
        }
        if self.state.as_deref() == Some("available") {
            findings.push(self.finding(
                Severity::Low,
                "ebs.unattached",
                "volume is not attached to any instance",
            ));
        }
        if self.kms_key_managed_by.as_deref() == Some("AWS") {
            findings.push(self.finding(
                Severity::Info,
                "ebs.aws_managed_key",
                "encrypted with the AWS managed key aws/ebs; its snapshots cannot be shared or copied to another account",
            ));
        }
        if let Some(state) = self.kms_key_state.as_deref().filter(|s| *s != "Enabled") {
            findings.push(self.finding(
                Severity::Medium,
                "ebs.kms_key_unusable",
                format!("KMS key is {state}; restores from this volume's snapshots will fail"),
            ));
        }
        if !self.public_snapshots.is_empty() {
            findings.push(self.finding(
                Severity::High,
                "ebs.snapshot_public",
                format!(
                    "{} snapshot(s) anyone can restore: {}",
                    self.public_snapshots.len(),
                    self.public_snapshots.join(", ")
                ),
            ));
        }
        if !self.shared_snapshot_accounts.is_empty() {
            findings.push(self.finding(
                Severity::Info,
                "ebs.snapshot_shared",
                format!(
                    "snapshots shared with account(s) {}",
                    self.shared_snapshot_accounts.join(", ")
                ),
            ));
        }
        let has_snapshots = self.snapshots.as_ref().is_some_and(|s| s.count > 0);
        if self.rbin_scanned && has_snapshots && self.protection.rbin_rules.is_empty() {
            findings.push(self.finding(
                Severity::Info,
                "ebs.no_recycle_bin",
                "no Recycle Bin rule retains this volume's snapshots if they are deleted",
            ));
        }
        findings
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecycleBinRule {
    pub id: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlock_delay_days: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_end_time: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_tags: Vec<RuleTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_tags: Vec<RuleTag>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleTag {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl RuleTag {
    fn matches(&self, tags: &Tags) -> bool {
        match (tags.get(&self.key), &self.value) {
            (Some(_), None) => true,
            (Some(have), Some(want)) => have == want,
            (None, _) => false,
        }
    }
}

impl RecycleBinRule {
    pub const EBS_SNAPSHOT: &'static str = "EBS_SNAPSHOT";
    pub const EC2_IMAGE: &'static str = "EC2_IMAGE";

    pub fn retains(&self, resource_type: &str, tags: &Tags) -> bool {
        if self.resource_type != resource_type || self.status.as_deref() != Some("available") {
            return false;
        }
        if self.resource_tags.is_empty() {
            !self.exclude_tags.iter().any(|t| t.matches(tags))
        } else {
            self.resource_tags.iter().any(|t| t.matches(tags))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> EbsVolume {
        EbsVolume {
            id: "vol-1".into(),
            arn: "arn:aws:ec2:ap-southeast-2:1:volume/vol-1".into(),
            region: "ap-southeast-2".into(),
            state: Some("in-use".into()),
            volume_type: Some("gp3".into()),
            size_gib: Some(100),
            encrypted: Some(true),
            kms_key_managed_by: Some("CUSTOMER".into()),
            kms_key_state: Some("Enabled".into()),
            snapshots: Some(SnapshotSummary {
                count: 14,
                ..SnapshotSummary::default()
            }),
            rbin_scanned: true,
            protection: Protection {
                rbin_rules: vec!["rule-1".into()],
                ..Protection::default()
            },
            ..EbsVolume::default()
        }
    }

    fn codes(volume: &EbsVolume) -> Vec<String> {
        volume.findings().into_iter().map(|f| f.code).collect()
    }

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn rule(resource_tags: Vec<RuleTag>, exclude_tags: Vec<RuleTag>) -> RecycleBinRule {
        RecycleBinRule {
            id: "rule-1".into(),
            resource_type: RecycleBinRule::EBS_SNAPSHOT.into(),
            status: Some("available".into()),
            resource_tags,
            exclude_tags,
            ..RecycleBinRule::default()
        }
    }

    fn tag(key: &str, value: Option<&str>) -> RuleTag {
        RuleTag {
            key: key.into(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn summary_reads_type_size_state_and_snapshots() {
        assert_eq!(volume().summary(), "gp3 100 GiB · in-use · 14 snaps");
        let bare = EbsVolume {
            size_gib: Some(8),
            snapshots: Some(SnapshotSummary {
                count: 1,
                ..SnapshotSummary::default()
            }),
            ..EbsVolume::default()
        };
        assert_eq!(bare.summary(), "8 GiB · 1 snap");
        assert_eq!(EbsVolume::default().summary(), "");
    }

    #[test]
    fn healthy_volume_has_no_findings() {
        assert!(codes(&volume()).is_empty());
    }

    #[test]
    fn flags_encryption_attachment_and_key_problems() {
        let v = EbsVolume {
            state: Some("available".into()),
            encrypted: Some(false),
            ..volume()
        };
        assert_eq!(codes(&v), vec!["ebs.unencrypted", "ebs.unattached"]);

        let v = EbsVolume {
            kms_key_managed_by: Some("AWS".into()),
            kms_key_state: Some("PendingDeletion".into()),
            ..volume()
        };
        assert_eq!(
            codes(&v),
            vec!["ebs.aws_managed_key", "ebs.kms_key_unusable"]
        );
    }

    #[test]
    fn flags_public_and_shared_snapshots() {
        let v = EbsVolume {
            public_snapshots: vec!["snap-1".into()],
            shared_snapshot_accounts: vec!["222233334444".into()],
            ..volume()
        };
        let findings = v.findings();
        assert_eq!(findings[0].code, "ebs.snapshot_public");
        assert_eq!(findings[0].severity, Severity::High);
        assert!(findings[0].message.contains("snap-1"));
        assert_eq!(findings[1].code, "ebs.snapshot_shared");
    }

    #[test]
    fn no_recycle_bin_needs_snapshots_and_a_completed_rule_scan() {
        let uncovered = EbsVolume {
            protection: Protection::default(),
            ..volume()
        };
        assert_eq!(codes(&uncovered), vec!["ebs.no_recycle_bin"]);

        let unscanned = EbsVolume {
            rbin_scanned: false,
            ..uncovered.clone()
        };
        assert!(codes(&unscanned).is_empty());

        let no_snapshots = EbsVolume {
            snapshots: Some(SnapshotSummary::default()),
            ..uncovered
        };
        assert!(codes(&no_snapshots).is_empty());
    }

    #[test]
    fn region_wide_rule_retains_all_but_excluded() {
        let r = rule(Vec::new(), vec![tag("skip", Some("yes"))]);
        assert!(r.retains(RecycleBinRule::EBS_SNAPSHOT, &Tags::new()));
        assert!(r.retains(RecycleBinRule::EBS_SNAPSHOT, &tags(&[("skip", "no")])));
        assert!(!r.retains(RecycleBinRule::EBS_SNAPSHOT, &tags(&[("skip", "yes")])));
        assert!(!r.retains(RecycleBinRule::EC2_IMAGE, &Tags::new()));
    }

    #[test]
    fn tag_scoped_rule_retains_any_matching_tag() {
        let r = rule(
            vec![tag("backup", Some("daily")), tag("keep", None)],
            Vec::new(),
        );
        assert!(r.retains(RecycleBinRule::EBS_SNAPSHOT, &tags(&[("backup", "daily")])));
        assert!(r.retains(RecycleBinRule::EBS_SNAPSHOT, &tags(&[("keep", "anything")])));
        assert!(!r.retains(RecycleBinRule::EBS_SNAPSHOT, &tags(&[("backup", "weekly")])));
        assert!(!r.retains(RecycleBinRule::EBS_SNAPSHOT, &Tags::new()));
    }

    #[test]
    fn only_available_rules_retain() {
        let pending = RecycleBinRule {
            status: Some("pending".into()),
            ..rule(Vec::new(), Vec::new())
        };
        assert!(!pending.retains(RecycleBinRule::EBS_SNAPSHOT, &Tags::new()));
        let unknown = RecycleBinRule {
            status: None,
            ..rule(Vec::new(), Vec::new())
        };
        assert!(!unknown.retains(RecycleBinRule::EBS_SNAPSHOT, &Tags::new()));
    }

    #[test]
    fn older_json_still_loads() {
        let v: EbsVolume = serde_json::from_str(
            r#"{"id":"vol-1","arn":"arn:aws:ec2:r:1:volume/vol-1","region":"r","size_gib":8}"#,
        )
        .unwrap();
        assert_eq!(v.size_gib, Some(8));
        assert!(v.snapshots.is_none() && !v.rbin_scanned);

        let r: RecycleBinRule =
            serde_json::from_str(r#"{"id":"r1","arn":"a","region":"r"}"#).unwrap();
        assert!(r.resource_tags.is_empty());
    }

    #[test]
    fn empty_extras_are_omitted_from_json() {
        let json = serde_json::to_value(EbsVolume::default()).unwrap();
        assert!(json.get("rbin_scanned").is_none());
        assert!(json.get("public_snapshots").is_none());
        assert!(json.get("snapshots").is_none());
    }
}
