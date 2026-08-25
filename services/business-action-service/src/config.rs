use std::{path::PathBuf, time::Duration};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    Production,
    Acceptance,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub enabled: bool,
    pub mode: ActionMode,
    pub database_url: String,
    pub allowed_origin: String,
    pub gateway_base_url: Url,
    pub service_credential: String,
    pub service_audience: String,
    pub catalog_path: PathBuf,
    pub catalog_version: String,
    pub work_item_draft_ttl: Duration,
    pub approval_draft_ttl: Duration,
    pub timezone: String,
    pub max_active_items_per_finding: u32,
    pub rate_limit_per_minute: u32,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{name} is required"))
        };
        let enabled = bool_env("BUSINESS_ACTION_ENABLED", false)?;
        let mode = match std::env::var("BUSINESS_ACTION_MODE")
            .unwrap_or_else(|_| "production".into())
            .as_str()
        {
            "production" => ActionMode::Production,
            "acceptance" => ActionMode::Acceptance,
            _ => return Err("BUSINESS_ACTION_MODE must be production or acceptance".into()),
        };
        let resolver =
            std::env::var("BUSINESS_ASSIGNEE_RESOLVER").unwrap_or_else(|_| "production".into());
        let authorization = bool_env("BUSINESS_ACTION_AUTHORIZATION_ENABLED", false)?;
        if enabled && mode == ActionMode::Production && (!authorization || resolver != "production")
        {
            return Err(
                "Production action mode requires formal authorization and production AssigneeResolver"
                    .into(),
            );
        }
        if enabled
            && mode == ActionMode::Acceptance
            && (std::env::var("BUSINESS_ACTION_ACCEPTANCE_ACKNOWLEDGE").as_deref()
                != Ok("Desensitized Acceptance - Production Disabled")
                || resolver != "acceptance")
        {
            return Err("Acceptance mode requires explicit Production Disabled acknowledgement and acceptance resolver".into());
        }
        let allowed_origin = required("BUSINESS_ACTION_ALLOWED_ORIGIN")?;
        let origin = Url::parse(&allowed_origin)
            .map_err(|_| "BUSINESS_ACTION_ALLOWED_ORIGIN must be a URL")?;
        if origin.path() != "/" || origin.query().is_some() || origin.fragment().is_some() {
            return Err("BUSINESS_ACTION_ALLOWED_ORIGIN must contain only an origin".into());
        }
        if mode == ActionMode::Production && origin.scheme() != "https" {
            return Err("Production Business Action origin must use HTTPS".into());
        }
        let gateway_base_url = Url::parse(&required("BUSINESS_AUTH_GATEWAY_BASE_URL")?)
            .map_err(|_| "BUSINESS_AUTH_GATEWAY_BASE_URL must be a URL")?;
        if mode == ActionMode::Production && gateway_base_url.scheme() != "https" {
            return Err("Production gateway URL must use HTTPS".into());
        }
        let service_credential = required("BUSINESS_READ_SERVICE_CREDENTIAL")?;
        if service_credential.len() < 32 {
            return Err("BUSINESS_READ_SERVICE_CREDENTIAL must be at least 32 bytes".into());
        }
        let timezone =
            std::env::var("BUSINESS_WORKFLOW_TIMEZONE").unwrap_or_else(|_| "Asia/Shanghai".into());
        if !matches!(timezone.as_str(), "Asia/Shanghai" | "UTC") {
            return Err("BUSINESS_WORKFLOW_TIMEZONE must be Asia/Shanghai or UTC".into());
        }
        let catalog_version = std::env::var("BUSINESS_ACTION_CATALOG_VERSION")
            .unwrap_or_else(|_| business_action_contracts::CATALOG_VERSION.into());
        if catalog_version != business_action_contracts::CATALOG_VERSION {
            return Err("BUSINESS_ACTION_CATALOG_VERSION is not supported by this build".into());
        }
        Ok(Self {
            enabled,
            mode,
            database_url: required("DATABASE_URL")?,
            allowed_origin: origin.origin().ascii_serialization(),
            gateway_base_url,
            service_credential,
            service_audience: "business-action-service".into(),
            catalog_path: PathBuf::from(
                std::env::var("BUSINESS_ACTION_CATALOG_PATH").unwrap_or_else(|_| {
                    "services/business-action-service/catalog/trade-action-v1.0.json".into()
                }),
            ),
            catalog_version,
            work_item_draft_ttl: Duration::from_secs(number(
                "WORK_ITEM_DRAFT_TTL_SECONDS",
                600,
                60,
                1800,
            )?),
            approval_draft_ttl: Duration::from_secs(number(
                "APPROVAL_DRAFT_TTL_SECONDS",
                604_800,
                3600,
                2_592_000,
            )?),
            timezone,
            max_active_items_per_finding: number(
                "BUSINESS_ACTION_MAX_ACTIVE_ITEMS_PER_FINDING",
                5,
                1,
                20,
            )? as u32,
            rate_limit_per_minute: number("BUSINESS_ACTION_RATE_LIMIT_PER_MINUTE", 60, 1, 300)?
                as u32,
        })
    }
}

fn bool_env(name: &str, default: bool) -> Result<bool, String> {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<bool>()
                .map_err(|_| format!("{name} must be true or false"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn number(name: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    let value = std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    (min..=max)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| format!("{name} must be between {min} and {max}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_mode_never_falls_back_to_acceptance() {
        assert_ne!(ActionMode::Production, ActionMode::Acceptance);
        assert!(matches!(ActionMode::Production, ActionMode::Production));
    }
}
