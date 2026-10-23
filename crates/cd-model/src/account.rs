use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    #[serde(default)]
    pub partition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_management_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_feature_set: Option<String>,
}

impl AccountInfo {
    pub fn is_org_management_account(&self) -> bool {
        self.org_management_account.as_deref() == Some(self.id.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trail {
    pub name: String,
    pub arn: String,
    pub region: String,
    #[serde(default)]
    pub home_region: Option<String>,
    #[serde(default)]
    pub multi_region: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_trail: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_key_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file_validation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kms_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_global_service_events: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloudwatch_log_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_event_selectors: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_account_is_the_org_owner() {
        let mut account = AccountInfo {
            id: "111111111111".into(),
            ..AccountInfo::default()
        };
        assert!(!account.is_org_management_account());
        account.org_management_account = Some("222222222222".into());
        assert!(!account.is_org_management_account());
        account.org_management_account = Some("111111111111".into());
        assert!(account.is_org_management_account());
    }

    #[test]
    fn older_account_and_trail_json_still_loads() {
        let account: AccountInfo = serde_json::from_str(r#"{"id":"1"}"#).unwrap();
        assert_eq!(account.org_feature_set, None);
        let trail: Trail =
            serde_json::from_str(r#"{"name":"t","arn":"a","region":"us-east-1"}"#).unwrap();
        assert_eq!(trail.s3_bucket, None);
    }
}
