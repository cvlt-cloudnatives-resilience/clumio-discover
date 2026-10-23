use aws_sdk_rbin::operation::get_rule::GetRuleOutput;
use aws_sdk_rbin::types::{ResourceTag, ResourceType, RuleSummary};
use aws_sdk_rbin::Client;
use futures::future::join_all;

use cd_model::{DiscoveryError, RecycleBinRule, RuleTag};

use crate::{failure, timestamp, Scope, Section};

const SERVICE: &str = "rbin";

pub(crate) const ACTIONS: &[&str] = &["rbin:GetRule", "rbin:ListRules"];

pub async fn rules(scope: &Scope<'_>) -> Section<RecycleBinRule> {
    let client = &Client::new(scope.sdk());
    let listed = join_all(
        [ResourceType::EbsSnapshot, ResourceType::Ec2Image].map(|t| list(scope, client, t)),
    )
    .await;
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for result in listed {
        match result {
            Ok(listed) => rules.extend(listed),
            Err(err) => errors.push(err),
        }
    }
    if !errors.is_empty() {
        return Section {
            items: None,
            errors,
        };
    }

    let details = join_all(
        rules
            .iter()
            .map(|rule| scope.call(client.get_rule().identifier(rule.id.clone()).send())),
    )
    .await;
    for (rule, detail) in rules.iter_mut().zip(details) {
        match detail {
            Ok(output) => apply_detail(rule, &output),
            Err(err) => errors.push(failure(SERVICE, "GetRule", Some(&rule.id), err)),
        }
    }
    Section::ok(rules, errors)
}

async fn list(
    scope: &Scope<'_>,
    client: &Client,
    resource_type: ResourceType,
) -> Result<Vec<RecycleBinRule>, DiscoveryError> {
    let mut pages = client
        .list_rules()
        .resource_type(resource_type.clone())
        .max_results(1000)
        .into_paginator()
        .send();
    let mut rules = Vec::new();
    while let Some(page) = scope.call(pages.next()).await {
        let page =
            page.map_err(|e| failure(SERVICE, "ListRules", Some(resource_type.as_str()), e))?;
        for summary in page.rules() {
            let id = summary.identifier().unwrap_or_default();
            let arn = scope.arn(SERVICE, &format!("rule/{id}"));
            rules.push(from_summary(summary, &resource_type, &scope.region, arn));
        }
    }
    Ok(rules)
}

fn from_summary(
    summary: &RuleSummary,
    resource_type: &ResourceType,
    region: &str,
    fallback_arn: String,
) -> RecycleBinRule {
    RecycleBinRule {
        id: summary.identifier().unwrap_or_default().to_string(),
        arn: summary
            .rule_arn()
            .map(str::to_string)
            .unwrap_or(fallback_arn),
        region: region.to_string(),
        resource_type: resource_type.as_str().to_string(),
        description: non_empty(summary.description()),
        retention_days: summary.retention_period().and_then(|p| {
            days(
                p.retention_period_value(),
                p.retention_period_unit().as_str(),
            )
        }),
        lock_state: summary.lock_state().map(|s| s.as_str().to_string()),
        ..RecycleBinRule::default()
    }
}

fn apply_detail(rule: &mut RecycleBinRule, output: &GetRuleOutput) {
    if let Some(arn) = output.rule_arn() {
        rule.arn = arn.to_string();
    }
    if let Some(kind) = output.resource_type() {
        rule.resource_type = kind.as_str().to_string();
    }
    if let Some(description) = non_empty(output.description()) {
        rule.description = Some(description);
    }
    if let Some(period) = output.retention_period() {
        rule.retention_days = days(
            period.retention_period_value(),
            period.retention_period_unit().as_str(),
        );
    }
    rule.status = output.status().map(|s| s.as_str().to_string());
    if let Some(state) = output.lock_state() {
        rule.lock_state = Some(state.as_str().to_string());
    }
    rule.unlock_delay_days = output
        .lock_configuration()
        .and_then(|c| c.unlock_delay())
        .and_then(|d| days(d.unlock_delay_value(), d.unlock_delay_unit().as_str()));
    rule.lock_end_time = output.lock_end_time().and_then(timestamp);
    rule.resource_tags = output.resource_tags().iter().map(rule_tag).collect();
    rule.exclude_tags = output
        .exclude_resource_tags()
        .iter()
        .map(rule_tag)
        .collect();
}

fn rule_tag(tag: &ResourceTag) -> RuleTag {
    RuleTag {
        key: tag.resource_tag_key().to_string(),
        value: tag.resource_tag_value().map(str::to_string),
    }
}

fn days(value: i32, unit: &str) -> Option<i32> {
    (unit == "DAYS").then_some(value)
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use aws_sdk_rbin::types::{
        LockConfiguration, LockState, RetentionPeriod, RetentionPeriodUnit, RuleStatus,
        UnlockDelay, UnlockDelayUnit,
    };
    use aws_smithy_types::DateTime;

    use super::*;

    fn retention(days: i32) -> RetentionPeriod {
        RetentionPeriod::builder()
            .retention_period_value(days)
            .retention_period_unit(RetentionPeriodUnit::Days)
            .build()
            .unwrap()
    }

    fn resource_tag(key: &str, value: &str) -> ResourceTag {
        ResourceTag::builder()
            .resource_tag_key(key)
            .resource_tag_value(value)
            .build()
            .unwrap()
    }

    #[test]
    fn maps_a_rule_summary() {
        let summary = RuleSummary::builder()
            .identifier("abc123")
            .description("")
            .retention_period(retention(7))
            .lock_state(LockState::Unlocked)
            .build();
        let rule = from_summary(
            &summary,
            &ResourceType::EbsSnapshot,
            "ap-southeast-2",
            "arn:aws:rbin:ap-southeast-2:1:rule/abc123".into(),
        );
        assert_eq!(rule.id, "abc123");
        assert_eq!(rule.arn, "arn:aws:rbin:ap-southeast-2:1:rule/abc123");
        assert_eq!(rule.resource_type, RecycleBinRule::EBS_SNAPSHOT);
        assert_eq!(rule.description, None);
        assert_eq!(rule.retention_days, Some(7));
        assert_eq!(rule.lock_state.as_deref(), Some("unlocked"));
        assert_eq!(rule.status, None);
    }

    #[test]
    fn applies_get_rule_detail() {
        let mut rule = RecycleBinRule {
            id: "abc123".into(),
            resource_type: RecycleBinRule::EBS_SNAPSHOT.into(),
            ..RecycleBinRule::default()
        };
        let output = GetRuleOutput::builder()
            .identifier("abc123")
            .rule_arn("arn:aws:rbin:ap-southeast-2:1:rule/abc123")
            .description("keep prod snapshots")
            .resource_type(ResourceType::EbsSnapshot)
            .retention_period(retention(30))
            .status(RuleStatus::Available)
            .lock_state(LockState::Locked)
            .lock_configuration(
                LockConfiguration::builder()
                    .unlock_delay(
                        UnlockDelay::builder()
                            .unlock_delay_value(10)
                            .unlock_delay_unit(UnlockDelayUnit::Days)
                            .build()
                            .unwrap(),
                    )
                    .build(),
            )
            .lock_end_time(DateTime::from_secs(1_792_713_600))
            .resource_tags(resource_tag("env", "prod"))
            .build();
        apply_detail(&mut rule, &output);

        assert_eq!(rule.arn, "arn:aws:rbin:ap-southeast-2:1:rule/abc123");
        assert_eq!(rule.description.as_deref(), Some("keep prod snapshots"));
        assert_eq!(rule.retention_days, Some(30));
        assert_eq!(rule.status.as_deref(), Some("available"));
        assert_eq!(rule.lock_state.as_deref(), Some("locked"));
        assert_eq!(rule.unlock_delay_days, Some(10));
        assert_eq!(rule.lock_end_time.as_deref(), Some("2026-10-23T00:00:00Z"));
        assert_eq!(
            rule.resource_tags,
            vec![RuleTag {
                key: "env".into(),
                value: Some("prod".into())
            }]
        );
        assert!(rule.exclude_tags.is_empty());

        let prod = [("env".to_string(), "prod".to_string())].into();
        assert!(rule.retains(RecycleBinRule::EBS_SNAPSHOT, &prod));
    }
}
