use business_analytics::RuleConfig;
use url::Url;

/// Runtime configuration for the reference API. It always requires final
/// delegation verification and never provides a production bypass.
pub struct Config {
    pub bind: std::net::SocketAddr,
    pub service_credential: String,
    pub gateway_base_url: Url,
    pub service_audience: String,
    pub rule_config: RuleConfig,
    pub max_findings: usize,
    pub max_payload_bytes: usize,
    pub core_base_url: Url,
    pub core_credential: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let required = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{name} is required"))
        };
        if std::env::var("BUSINESS_AUTHORIZATION_ENABLED").as_deref() != Ok("true") {
            return Err("BUSINESS_AUTHORIZATION_ENABLED=true is required".into());
        }
        let credential = required("BUSINESS_READ_SERVICE_CREDENTIAL")?;
        if credential.len() < 32 {
            return Err("BUSINESS_READ_SERVICE_CREDENTIAL must be at least 32 bytes".into());
        }
        if std::env::var("BUSINESS_READ_SERVICE_AUTH_MODE")
            .unwrap_or_else(|_| "shared_secret".into())
            != "shared_secret"
        {
            return Err("BUSINESS_READ_SERVICE_AUTH_MODE must be shared_secret".into());
        }
        let service_audience = std::env::var("BUSINESS_READ_SERVICE_AUDIENCE")
            .unwrap_or_else(|_| "business-read-api".into());
        if service_audience != "business-read-api" {
            return Err("BUSINESS_READ_SERVICE_AUDIENCE must be business-read-api".into());
        }
        if std::env::var("BUSINESS_ANOMALY_ENABLED").unwrap_or_else(|_| "true".into()) != "true" {
            return Err("BUSINESS_ANOMALY_ENABLED=true is required".into());
        }
        let mut rule_config: RuleConfig = match std::env::var("BUSINESS_ANOMALY_RULESET_PATH") {
            Ok(ruleset_path) => serde_json::from_str(
                &std::fs::read_to_string(&ruleset_path)
                    .map_err(|_| "BUSINESS_ANOMALY_RULESET_PATH could not be read")?,
            )
            .map_err(|_| "BUSINESS_ANOMALY_RULESET_PATH is not valid rule JSON")?,
            Err(_) => RuleConfig::bundled().map_err(|error| error.to_string())?,
        };
        let configured_version = std::env::var("BUSINESS_ANOMALY_DEFAULT_RULESET_VERSION")
            .unwrap_or_else(|_| business_anomaly_contracts::RULE_SET_VERSION.into());
        if rule_config.version != configured_version {
            return Err(
                "BUSINESS_ANOMALY_DEFAULT_RULESET_VERSION does not match the rule file".into(),
            );
        }
        rule_config.stale_after_minutes = parse_env_usize(
            "BUSINESS_DATA_STALE_AFTER_MINUTES",
            rule_config.stale_after_minutes as usize,
            1,
            525_600,
        )? as i64;
        rule_config.validate().map_err(|error| error.to_string())?;
        let max_findings = parse_env_usize("BUSINESS_ANOMALY_MAX_FINDINGS", 100, 1, 100)?;
        let max_payload_bytes = parse_env_usize(
            "BUSINESS_ANOMALY_MAX_PAYLOAD_BYTES",
            128 * 1024,
            4096,
            1024 * 1024,
        )?;
        let tool_timeout = parse_env_usize("BUSINESS_ANOMALY_TOOL_TIMEOUT_SECONDS", 10, 1, 30)?;
        let run_timeout = parse_env_usize("BUSINESS_ANOMALY_RUN_TIMEOUT_SECONDS", 30, 1, 300)?;
        if run_timeout < tool_timeout {
            return Err(
                "BUSINESS_ANOMALY_RUN_TIMEOUT_SECONDS must not be below tool timeout".into(),
            );
        }
        if std::env::var("BUSINESS_ANOMALY_SCHEDULE_ENABLED").unwrap_or_else(|_| "false".into())
            != "false"
        {
            return Err("scheduled anomaly runs are not implemented; keep BUSINESS_ANOMALY_SCHEDULE_ENABLED=false".into());
        }
        let gateway = Url::parse(&required("BUSINESS_AUTH_GATEWAY_BASE_URL")?)
            .map_err(|_| "BUSINESS_AUTH_GATEWAY_BASE_URL must be a URL")?;
        if gateway.scheme() != "https" && !(cfg!(debug_assertions) && gateway.scheme() == "http") {
            return Err("BUSINESS_AUTH_GATEWAY_BASE_URL must use HTTPS".into());
        }
        let core_base_url = Url::parse(&required("BUSINESS_CORE_BASE_URL")?)
            .map_err(|_| "BUSINESS_CORE_BASE_URL must be a URL")?;
        if core_base_url.scheme() != "https"
            && !(cfg!(debug_assertions) && core_base_url.scheme() == "http")
        {
            return Err("BUSINESS_CORE_BASE_URL must use HTTPS".into());
        }
        let core_credential = required("BUSINESS_CORE_SERVICE_CREDENTIAL")?;
        if core_credential.len() < 32 {
            return Err("BUSINESS_CORE_SERVICE_CREDENTIAL must be at least 32 bytes".into());
        }
        Ok(Self {
            bind: required("BUSINESS_READ_API_BIND")?
                .parse()
                .map_err(|_| "BUSINESS_READ_API_BIND must be a socket address")?,
            service_credential: credential,
            gateway_base_url: gateway,
            service_audience,
            rule_config,
            max_findings,
            max_payload_bytes,
            core_base_url,
            core_credential,
        })
    }
}

fn parse_env_usize(name: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
    let value = std::env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}
