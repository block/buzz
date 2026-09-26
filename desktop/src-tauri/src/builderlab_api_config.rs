#[derive(Clone, Copy)]
pub(crate) enum ApiBaseUrlKind {
    Builderlab,
    EnterpriseAuthAdapter,
}

const DEFAULT_BUILDERLAB_API_BASE_URL: &str = "https://app.builderlab.xyz/api/goose";

fn env_name(kind: ApiBaseUrlKind) -> &'static str {
    match kind {
        ApiBaseUrlKind::Builderlab => "BUZZ_BUILD_BUILDERLAB_API_BASE_URL",
        ApiBaseUrlKind::EnterpriseAuthAdapter => "BUZZ_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL",
    }
}

pub(crate) fn validate_api_base_url(raw: &str, kind: ApiBaseUrlKind) -> Result<String, String> {
    let env = env_name(kind);
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(format!("{env} must not be empty when set"));
    }

    let url =
        url::Url::parse(trimmed).map_err(|error| format!("{env} is not a valid URL: {error}"))?;
    match url.scheme() {
        "https" => {}
        "http" => {
            if matches!(kind, ApiBaseUrlKind::EnterpriseAuthAdapter)
                && !url.host().is_some_and(is_loopback_host)
            {
                return Err(format!(
                    "{env} must use https:// unless it points to localhost, 127.0.0.0/8, or ::1 for local development"
                ));
            }
        }
        _ => {
            return Err(format!("{env} must use http:// or https://"));
        }
    }
    if url.host_str().is_none() {
        return Err(format!("{env} must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{env} must not include userinfo"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{env} must not include a query or fragment"));
    }
    Ok(trimmed.to_owned())
}

fn is_loopback_host(host: url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(addr) => addr.is_loopback(),
        url::Host::Ipv6(addr) => addr.is_loopback(),
    }
}

pub(crate) fn validate_builderlab_api_base_url(raw: &str) -> Result<String, String> {
    validate_api_base_url(raw, ApiBaseUrlKind::Builderlab)
}

pub(crate) fn resolve_builderlab_api_base_url(
    configured: Option<&str>,
    _enterprise_relays: Option<&str>,
) -> Result<String, String> {
    if let Some(configured) = configured {
        return validate_builderlab_api_base_url(configured);
    }

    Ok(DEFAULT_BUILDERLAB_API_BASE_URL.to_owned())
}

pub(crate) fn resolve_enterprise_auth_adapter_base_url(
    configured: Option<&str>,
    enterprise_relays: Option<&str>,
) -> Result<Option<String>, String> {
    let configured = configured.map(str::trim).filter(|value| !value.is_empty());
    if let Some(configured) = configured {
        return validate_api_base_url(configured, ApiBaseUrlKind::EnterpriseAuthAdapter).map(Some);
    }

    if enterprise_relays
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Err(
            "BUZZ_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL must be set when BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS is set"
                .to_owned(),
        );
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_builderlab_api_base_url_normalizes_trailing_slash() {
        assert_eq!(
            resolve_builderlab_api_base_url(Some(" https://login.example/api/goose/ "), None)
                .unwrap(),
            "https://login.example/api/goose",
        );
    }

    #[test]
    fn hosted_builderlab_api_keeps_default_when_enterprise_auth_uses_separate_adapter() {
        assert_eq!(
            resolve_builderlab_api_base_url(None, Some("wss://buzz.block.example")).unwrap(),
            DEFAULT_BUILDERLAB_API_BASE_URL,
        );
    }

    #[test]
    fn enterprise_auth_requires_configured_adapter_base_url() {
        let error =
            resolve_enterprise_auth_adapter_base_url(None, Some("wss://buzz.block.example"))
                .expect_err("enterprise builds must configure the browser-login adapter base");
        assert!(error.contains("BUZZ_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL must be set"));
    }

    #[test]
    fn configured_enterprise_adapter_base_url_normalizes_trailing_slash() {
        assert_eq!(
            resolve_enterprise_auth_adapter_base_url(
                Some(" https://identity.example/buzz-auth/ "),
                Some("wss://buzz.block.example"),
            )
            .unwrap(),
            Some("https://identity.example/buzz-auth".to_owned()),
        );
    }

    #[test]
    fn enterprise_adapter_base_url_rejects_remote_plaintext_http() {
        for raw in [
            "http://identity.example/buzz-auth",
            "http://192.0.2.10/buzz-auth",
            "http://[2001:db8::1]/buzz-auth",
        ] {
            let error = resolve_enterprise_auth_adapter_base_url(
                Some(raw),
                Some("wss://buzz.block.example"),
            )
            .expect_err("remote plaintext adapter URLs must fail closed");

            assert!(error.contains("must use https://"), "{raw:?}: {error}");
        }
    }

    #[test]
    fn enterprise_adapter_base_url_allows_plaintext_loopback_for_development() {
        for (raw, expected) in [
            (
                "http://localhost:8787/buzz-auth/",
                "http://localhost:8787/buzz-auth",
            ),
            (
                "http://LOCALHOST:8787/buzz-auth/",
                "http://LOCALHOST:8787/buzz-auth",
            ),
            (
                "http://127.0.0.1:8787/buzz-auth/",
                "http://127.0.0.1:8787/buzz-auth",
            ),
            (
                "http://127.42.0.9:8787/buzz-auth/",
                "http://127.42.0.9:8787/buzz-auth",
            ),
            (
                "http://[::1]:8787/buzz-auth/",
                "http://[::1]:8787/buzz-auth",
            ),
        ] {
            assert_eq!(
                resolve_enterprise_auth_adapter_base_url(
                    Some(raw),
                    Some("wss://buzz.block.example"),
                )
                .unwrap(),
                Some(expected.to_owned()),
                "{raw:?} should be accepted as loopback development HTTP",
            );
        }
    }

    #[test]
    fn builderlab_api_base_url_still_allows_remote_plaintext_http() {
        assert_eq!(
            validate_builderlab_api_base_url("http://builderlab.example/api/goose/").unwrap(),
            "http://builderlab.example/api/goose",
        );
    }

    #[test]
    fn builderlab_api_base_url_rejects_ambiguous_urls() {
        for raw in [
            "",
            "ftp://login.example/api/goose",
            "https://user@login.example/api/goose",
            "https://login.example/api/goose?env=prod",
            "https://login.example/api/goose#prod",
        ] {
            assert!(
                validate_builderlab_api_base_url(raw).is_err(),
                "{raw:?} must fail closed",
            );
        }
    }
}
