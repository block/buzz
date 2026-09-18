use url::Url;

pub(crate) fn parse_enterprise_relay_allowlist(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS must not be empty when set".to_owned());
    }

    trimmed
        .split(',')
        .enumerate()
        .map(|(index, relay)| {
            let relay = relay.trim();
            if relay.is_empty() {
                return Err(format!(
                    "BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS entry {} must not be empty",
                    index + 1
                ));
            }
            canonical_enterprise_relay_url(relay)
        })
        .collect()
}

pub(crate) fn canonical_enterprise_relay_url(raw: &str) -> Result<String, String> {
    let mut url = Url::parse(raw.trim()).map_err(|error| format!("invalid relay URL: {error}"))?;
    let scheme = match url.scheme() {
        "ws" | "http" => "ws",
        "wss" | "https" => "wss",
        other => return Err(format!("unsupported relay URL scheme: {other}")),
    };
    url.set_scheme(scheme)
        .map_err(|_| "could not normalize relay URL scheme".to_owned())?;

    if url.host_str().is_none() {
        return Err("relay URL must include a host".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("relay URL must not include userinfo".to_owned());
    }
    if url.query().is_some() {
        return Err("relay URL must not include a query string".to_owned());
    }
    if url.fragment().is_some() {
        return Err("relay URL must not include a fragment".to_owned());
    }

    if matches!((scheme, url.port()), ("ws", Some(80)) | ("wss", Some(443))) {
        url.set_port(None)
            .map_err(|_| "could not normalize relay URL port".to_owned())?;
    }

    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string())
}
