use aws_sdk_cloudtrail::Client;

use cd_model::Trail;

use crate::{failure, Scope, Section};

pub(crate) const ACTIONS: &[&str] = &["cloudtrail:DescribeTrails"];

pub async fn discover(scope: &Scope<'_>) -> Section<Trail> {
    let client = Client::new(scope.sdk());
    match scope
        .call(client.describe_trails().include_shadow_trails(false).send())
        .await
    {
        Ok(output) => Section::ok(
            output
                .trail_list()
                .iter()
                .map(|t| trail(t, &scope.region))
                .collect(),
            Vec::new(),
        ),
        Err(err) => Section::failed(failure("cloudtrail", "DescribeTrails", None, err)),
    }
}

fn trail(t: &aws_sdk_cloudtrail::types::Trail, region: &str) -> Trail {
    let owned = |s: Option<&str>| s.map(str::to_string);
    Trail {
        name: t.name().unwrap_or_default().to_string(),
        arn: t.trail_arn().unwrap_or_default().to_string(),
        region: region.to_string(),
        home_region: owned(t.home_region()),
        multi_region: t.is_multi_region_trail(),
        organization_trail: t.is_organization_trail(),
        s3_bucket: owned(t.s3_bucket_name()),
        s3_key_prefix: owned(t.s3_key_prefix()),
        log_file_validation: t.log_file_validation_enabled(),
        kms_key_id: owned(t.kms_key_id()),
        include_global_service_events: t.include_global_service_events(),
        cloudwatch_log_group: owned(t.cloud_watch_logs_log_group_arn()),
        custom_event_selectors: t.has_custom_event_selectors(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_described_trail() {
        let described = aws_sdk_cloudtrail::types::Trail::builder()
            .name("org-trail")
            .trail_arn("arn:aws:cloudtrail:us-east-1:111111111111:trail/org-trail")
            .home_region("us-east-1")
            .is_multi_region_trail(true)
            .is_organization_trail(true)
            .s3_bucket_name("trail-logs")
            .log_file_validation_enabled(false)
            .build();
        let trail = trail(&described, "us-east-1");
        assert_eq!(trail.name, "org-trail");
        assert_eq!(trail.multi_region, Some(true));
        assert_eq!(trail.organization_trail, Some(true));
        assert_eq!(trail.s3_bucket.as_deref(), Some("trail-logs"));
        assert_eq!(trail.log_file_validation, Some(false));
        assert_eq!(trail.kms_key_id, None);
    }
}
