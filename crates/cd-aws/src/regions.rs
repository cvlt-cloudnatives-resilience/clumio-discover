use aws_sdk_account::types::RegionOptStatus;
use aws_sdk_account::Client;

use cd_model::DiscoveryError;

use crate::{failure, Ctx};

pub(crate) const ACTIONS: &[&str] = &["account:ListRegions"];

pub(crate) const CLUMIO_REGIONS: &[&str] = &[
    "ap-east-1",
    "ap-south-1",
    "ap-south-2",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-southeast-1",
    "ap-southeast-2",
    "ca-central-1",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "il-central-1",
    "me-central-1",
    "me-south-1",
    "sa-east-1",
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
];

pub async fn resolve(ctx: &Ctx) -> (Vec<String>, Vec<DiscoveryError>) {
    let requested = ctx.config.requested_regions();
    if !requested.is_empty() {
        return validate_requested(requested);
    }
    match enabled_regions(ctx).await {
        Ok(enabled) => (supported(enabled), Vec::new()),
        Err(mut err) => {
            let home = ctx.home().region().map(|r| r.to_string());
            err.message = format!(
                "{}; scanning only the home region {}. Set `regions` in the config to choose regions",
                err.message,
                home.as_deref().unwrap_or("(none)"),
            );
            (home.into_iter().collect(), vec![err])
        }
    }
}

async fn enabled_regions(ctx: &Ctx) -> Result<Vec<String>, DiscoveryError> {
    let client = Client::new(ctx.home());
    let mut pages = client
        .list_regions()
        .region_opt_status_contains(RegionOptStatus::Enabled)
        .region_opt_status_contains(RegionOptStatus::EnabledByDefault)
        .into_paginator()
        .send();
    let mut regions = Vec::new();
    while let Some(page) = ctx.call(pages.next()).await {
        let page = page.map_err(|e| failure("account", "ListRegions", None, e))?;
        regions.extend(
            page.regions()
                .iter()
                .filter_map(|r| r.region_name())
                .map(str::to_string),
        );
    }
    Ok(regions)
}

fn is_supported(region: &str) -> bool {
    CLUMIO_REGIONS.contains(&region)
}

fn supported(enabled: Vec<String>) -> Vec<String> {
    let mut regions: Vec<String> = enabled.into_iter().filter(|r| is_supported(r)).collect();
    regions.sort();
    regions.dedup();
    regions
}

fn validate_requested(requested: Vec<String>) -> (Vec<String>, Vec<DiscoveryError>) {
    let (kept, dropped): (Vec<String>, Vec<String>) =
        requested.into_iter().partition(|r| is_supported(r));
    let errors = dropped
        .into_iter()
        .map(|region| DiscoveryError {
            service: "config".to_string(),
            operation: "regions".to_string(),
            message: format!("{region} is not a Clumio-supported region; skipped"),
            region: Some(region),
        })
        .collect();
    (supported(kept), errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn clumio_regions_are_the_25_unique_template_regions() {
        let mut regions = CLUMIO_REGIONS.to_vec();
        regions.sort_unstable();
        regions.dedup();
        assert_eq!(regions.len(), 25);
    }

    #[test]
    fn enabled_regions_are_narrowed_to_clumio_regions_and_sorted() {
        let enabled = strings(&[
            "us-west-2",
            "af-south-1",
            "ap-southeast-2",
            "us-east-1",
            "ap-southeast-4",
            "us-east-1",
        ]);
        assert_eq!(
            supported(enabled),
            strings(&["ap-southeast-2", "us-east-1", "us-west-2"])
        );
    }

    #[test]
    fn unsupported_requested_regions_are_dropped_with_an_error() {
        let (regions, errors) =
            validate_requested(strings(&["us-east-1", "mars-north-1", "ap-southeast-2"]));
        assert_eq!(regions, strings(&["ap-southeast-2", "us-east-1"]));
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].region.as_deref(), Some("mars-north-1"));
        assert!(errors[0].message.contains("not a Clumio-supported region"));
    }

    #[test]
    fn supported_requested_regions_pass_without_errors() {
        let (regions, errors) = validate_requested(strings(&["eu-west-1"]));
        assert_eq!(regions, strings(&["eu-west-1"]));
        assert!(errors.is_empty());
    }
}
