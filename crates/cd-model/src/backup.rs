use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::AwsBackupCoverage;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedResource {
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub resource_type: Option<String>,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub coverage: AwsBackupCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_backup: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub points_by_status: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unencrypted_points: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_plans: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<RecoveryPoint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPoint {
    pub arn: String,
    pub vault: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_after_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cold_storage_after_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_restore: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupVault {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub recovery_points: Option<i64>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_key_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_retention_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retention_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_point_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub points_by_status: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_recovery_point: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_backup_json_still_loads() {
        let resource: ProtectedResource =
            serde_json::from_str(r#"{"arn":"arn:aws:s3:::b","region":"us-east-1"}"#).unwrap();
        assert_eq!(resource.unencrypted_points, 0);
        assert!(resource.latest.is_none());
        let vault: BackupVault =
            serde_json::from_str(r#"{"name":"Default","arn":"a","region":"us-east-1"}"#).unwrap();
        assert!(vault.points_by_status.is_empty());
    }

    #[test]
    fn empty_extras_are_left_out_of_json() {
        let json = serde_json::to_value(ProtectedResource::default()).unwrap();
        for key in [
            "oldest_backup",
            "points_by_status",
            "unencrypted_points",
            "backup_plans",
            "latest",
        ] {
            assert!(json.get(key).is_none(), "{key} serialized");
        }
    }
}
