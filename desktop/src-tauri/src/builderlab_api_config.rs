const DEFAULT_BUILDERLAB_API_BASE_URL: &str = "https://app.builderlab.xyz/api/goose";

pub(crate) fn validate_builderlab_api_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("BUZZ_BUILD_BUILDERLAB_API_BASE_URL must not be empty when set".to_owned());
    }

    let url = url::Url::parse(trimmed).map_err(|error| {
        format!("BUZZ_BUILD_BUILDERLAB_API_BASE_URL is not a valid URL: {error}")
    })?;
    match url.scheme() {
        "https" | "http" => {}
        _ => {
            return Err(
                "BUZZ_BUILD_BUILDERLAB_API_BASE_URL must use http:// or https://".to_owned(),
            );
        }
    }
    if url.host_str().is_none() {
        return Err("BUZZ_BUILD_BUILDERLAB_API_BASE_URL must include a host".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("BUZZ_BUILD_BUILDERLAB_API_BASE_URL must not include userinfo".to_owned());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(
            "BUZZ_BUILD_BUILDERLAB_API_BASE_URL must not include a query or fragment".to_owned(),
        );
    }
    Ok(trimmed.to_owned())
}

pub(crate) fn resolve_builderlab_api_base_url(
    configured: Option<&str>,
    enterprise_relays: Option<&str>,
) -> Result<String, String> {
    if let Some(configured) = configured {
        return validate_builderlab_api_base_url(configured);
    }

    if enterprise_relays
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Err(
            "BUZZ_BUILD_BUILDERLAB_API_BASE_URL must be set when BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS is set"
                .to_owned(),
        );
    }

    Ok(DEFAULT_BUILDERLAB_API_BASE_URL.to_owned())
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
    fn enterprise_auth_requires_configured_builderlab_api_base_url() {
        let error = resolve_builderlab_api_base_url(None, Some("wss://buzz.block.example"))
            .expect_err("enterprise builds must configure the browser-login API base");
        assert!(error.contains("BUZZ_BUILD_BUILDERLAB_API_BASE_URL must be set"));
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
