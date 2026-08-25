use std::{collections::HashSet, net::SocketAddr, time::Duration};
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    pub authentik_issuer: String,
    pub workbench_client_id: String,
    pub business_client_id: String,
    pub allowed_workbench_origins: HashSet<String>,
    pub business_origin: String,
    pub challenge_ttl: Duration,
    pub embed_ttl: Duration,
    pub business_ttl: Duration,
    pub rate_limit: i64,
    pub cleanup_interval: Duration,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub deployment_id: String,
    pub global_logout_redirect_uri: String,
    pub business_agent_read_enabled: bool,
    pub business_read_mcp_audience: String,
    pub agent_delegation_ttl: Duration,
    pub agent_delegation_max_calls: i32,
    pub business_agent_rate_limit_per_minute: i64,
    pub business_read_service_credential: Option<String>,
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}
fn seconds(name: &str, default: u64, min: u64, max: u64) -> Result<Duration, String> {
    let value = std::env::var(name)
        .ok()
        .map(|v| {
            v.parse::<u64>()
                .map_err(|_| format!("{name} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(Duration::from_secs(value))
}
fn boolean(name: &str, default: bool) -> Result<bool, String> {
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
fn origin(value: &str, name: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| format!("{name} must be a URL"))?;
    if url.scheme() != "https" && !(cfg!(debug_assertions) && url.scheme() == "http") {
        return Err(format!("{name} must use HTTPS"));
    }
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(format!("{name} must be an origin"));
    }
    Ok(url.origin().ascii_serialization())
}
fn workbench_origin(value: &str) -> Result<String, String> {
    let url =
        Url::parse(value).map_err(|_| "ALLOWED_WORKBENCH_ORIGINS must contain URLs".to_string())?;
    if url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.path().is_empty() {
        return Ok("tauri://localhost".into());
    }
    if url.scheme() == "http" && url.host_str() == Some("tauri.localhost") && url.path() == "/" {
        return Ok("http://tauri.localhost".into());
    }
    origin(value, "ALLOWED_WORKBENCH_ORIGINS")
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let authentik_issuer = required("AUTHENTIK_ISSUER")?;
        let issuer = Url::parse(&authentik_issuer)
            .map_err(|_| "AUTHENTIK_ISSUER must be a URL".to_string())?;
        if issuer.scheme() != "https" && !(cfg!(debug_assertions) && issuer.scheme() == "http") {
            return Err("AUTHENTIK_ISSUER must use HTTPS".into());
        }
        let origins = required("ALLOWED_WORKBENCH_ORIGINS")?
            .split(',')
            .map(|v| workbench_origin(v.trim()))
            .collect::<Result<HashSet<_>, _>>()?;
        let cookie_secure: bool = std::env::var("BUSINESS_SESSION_COOKIE_SECURE")
            .unwrap_or_else(|_| "true".into())
            .parse()
            .map_err(|_| "BUSINESS_SESSION_COOKIE_SECURE must be true or false")?;
        if !cookie_secure && !cfg!(debug_assertions) {
            return Err("production cookies must be Secure".into());
        }
        if std::env::var("BUSINESS_SESSION_COOKIE_SAMESITE").unwrap_or_else(|_| "None".into())
            != "None"
        {
            return Err("BUSINESS_SESSION_COOKIE_SAMESITE must be None".into());
        }
        let cookie_name = std::env::var("BUSINESS_SESSION_COOKIE_NAME")
            .unwrap_or_else(|_| "__Host-bizfin_business".into());
        if !cookie_name.starts_with("__Host-") {
            return Err("BUSINESS_SESSION_COOKIE_NAME must use __Host-".into());
        }
        let business_agent_read_enabled = boolean("BUSINESS_AGENT_READ_ENABLED", false)?;
        let business_read_service_credential = std::env::var("BUSINESS_READ_SERVICE_CREDENTIAL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        if business_agent_read_enabled
            && business_read_service_credential
                .as_ref()
                .is_none_or(|value| value.len() < 32)
        {
            return Err(
                "BUSINESS_READ_SERVICE_CREDENTIAL (at least 32 bytes) is required when BUSINESS_AGENT_READ_ENABLED=true".into(),
            );
        }
        let audience = std::env::var("BUSINESS_READ_MCP_AUDIENCE")
            .unwrap_or_else(|_| "business-read-mcp".into());
        if audience != "business-read-mcp" {
            return Err("BUSINESS_READ_MCP_AUDIENCE must be business-read-mcp".into());
        }
        let max_calls = std::env::var("AGENT_DELEGATION_MAX_CALLS")
            .unwrap_or_else(|_| "20".into())
            .parse::<i32>()
            .map_err(|_| "AGENT_DELEGATION_MAX_CALLS must be an integer")?;
        if !(1..=100).contains(&max_calls) {
            return Err("AGENT_DELEGATION_MAX_CALLS must be between 1 and 100".into());
        }
        let agent_rate_limit = std::env::var("BUSINESS_AGENT_RATE_LIMIT_PER_MINUTE")
            .unwrap_or_else(|_| "10".into())
            .parse::<i64>()
            .map_err(|_| "BUSINESS_AGENT_RATE_LIMIT_PER_MINUTE must be an integer")?;
        if !(1..=600).contains(&agent_rate_limit) {
            return Err("BUSINESS_AGENT_RATE_LIMIT_PER_MINUTE must be between 1 and 600".into());
        }
        Ok(Self {
            database_url: required("DATABASE_URL")?,
            bind_addr: std::env::var("BIND_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:3100".into())
                .parse()
                .map_err(|_| "BIND_ADDR is invalid")?,
            // OIDC issuer matching is exact, including a provider's trailing
            // slash. Preserve the configured value instead of silently
            // changing the trust boundary.
            authentik_issuer: issuer.as_str().to_string(),
            workbench_client_id: required("WORKBENCH_CLIENT_ID")?,
            business_client_id: required("BUSINESS_CLIENT_ID")?,
            allowed_workbench_origins: origins,
            business_origin: origin(&required("BUSINESS_APP_ORIGIN")?, "BUSINESS_APP_ORIGIN")?,
            challenge_ttl: seconds("IDENTITY_BINDING_CHALLENGE_TTL_SECONDS", 90, 60, 120)?,
            embed_ttl: seconds("EMBED_SESSION_TTL_SECONDS", 30, 5, 30)?,
            business_ttl: seconds("BUSINESS_SESSION_TTL_SECONDS", 86400, 60, 86400)?,
            rate_limit: std::env::var("EMBED_SESSION_RATE_LIMIT_PER_MINUTE")
                .unwrap_or_else(|_| "10".into())
                .parse()
                .map_err(|_| "EMBED_SESSION_RATE_LIMIT_PER_MINUTE must be an integer")?,
            cleanup_interval: seconds("SESSION_CLEANUP_INTERVAL_SECONDS", 60, 10, 3600)?,
            cookie_name,
            cookie_secure,
            deployment_id: required("DEPLOYMENT_ID")?,
            global_logout_redirect_uri: required("GLOBAL_LOGOUT_REDIRECT_URI")?,
            business_agent_read_enabled,
            business_read_mcp_audience: audience,
            agent_delegation_ttl: seconds("AGENT_DELEGATION_TTL_SECONDS", 300, 30, 900)?,
            agent_delegation_max_calls: max_calls,
            business_agent_rate_limit_per_minute: agent_rate_limit,
            business_read_service_credential,
        })
    }
}
