use cd_model::{AccountInfo, DiscoveryError};

use crate::{failure, Ctx};

pub(crate) const ACTIONS: &[&str] = &[
    "iam:ListAccountAliases",
    "organizations:DescribeOrganization",
];

pub async fn discover(ctx: &Ctx) -> (AccountInfo, Vec<DiscoveryError>) {
    let mut info = AccountInfo {
        id: ctx.account.clone(),
        partition: ctx.partition.clone(),
        ..AccountInfo::default()
    };
    let mut errors = Vec::new();

    let (alias, org) = tokio::join!(alias(ctx), organization(ctx));
    match alias {
        Ok(alias) => info.alias = alias,
        Err(err) => errors.push(err),
    }
    match org {
        Ok(Some(org)) => {
            info.org_id = org.id().map(str::to_string);
            info.org_management_account = org.master_account_id().map(str::to_string);
            info.org_feature_set = org.feature_set().map(|f| f.as_str().to_string());
        }
        Ok(None) => {}
        Err(err) => errors.push(err),
    }
    (info, errors)
}

async fn alias(ctx: &Ctx) -> Result<Option<String>, DiscoveryError> {
    let client = aws_sdk_iam::Client::new(ctx.home());
    let output = ctx
        .call(client.list_account_aliases().send())
        .await
        .map_err(|e| failure("iam", "ListAccountAliases", None, e))?;
    Ok(output.account_aliases().first().cloned())
}

async fn organization(
    ctx: &Ctx,
) -> Result<Option<aws_sdk_organizations::types::Organization>, DiscoveryError> {
    let client = aws_sdk_organizations::Client::new(ctx.home());
    match ctx.call(client.describe_organization().send()).await {
        Ok(output) => Ok(output.organization().cloned()),
        Err(err)
            if err
                .as_service_error()
                .is_some_and(|e| e.is_aws_organizations_not_in_use_exception()) =>
        {
            Ok(None)
        }
        Err(err) => Err(failure("organizations", "DescribeOrganization", None, err)),
    }
}
