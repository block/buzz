use super::*;
use std::collections::HashSet;
use url::Host;

fn resource_meta(content: &Value, listing: Option<&McpAppResource>) -> Value {
    content
        .get("_meta")
        .or_else(|| content.get("meta"))
        .cloned()
        .or_else(|| listing.map(|resource| resource.meta.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(super) fn parse_ui_resource(
    response: &Value,
    requested_uri: &str,
    listing: Option<&McpAppResource>,
) -> Result<(String, McpAppResourceCsp, McpAppResourcePermissions), String> {
    let contents = response
        .pointer("/result/contents")
        .and_then(Value::as_array)
        .ok_or_else(|| "MCP resources/read response is missing result.contents".to_string())?;
    if contents.len() != 1 {
        return Err("MCP App resource must contain exactly one document".to_string());
    }
    let content = &contents[0];
    if text(content.get("uri")).as_deref() != Some(requested_uri) {
        return Err("MCP App resource URI does not match the request".to_string());
    }
    if text(content.get("mimeType")).as_deref() != Some(MCP_APP_MIME_TYPE) {
        return Err(format!("MCP App resource must use {MCP_APP_MIME_TYPE}"));
    }
    let html = if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.to_string()
    } else if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob)
            .map_err(|_| "MCP App resource blob is not valid base64".to_string())?;
        String::from_utf8(bytes)
            .map_err(|_| "MCP App resource blob is not valid UTF-8".to_string())?
    } else {
        return Err("MCP App resource has no text or blob content".to_string());
    };
    if html.len() > MAX_MCP_APP_HTML_BYTES {
        return Err("MCP App HTML exceeds the 4 MiB limit".to_string());
    }
    let meta = resource_meta(content, listing);
    let ui = meta.get("ui").cloned().unwrap_or_else(|| json!({}));
    let csp = sanitize_csp(
        serde_json::from_value(ui.get("csp").cloned().unwrap_or_else(|| json!({})))
            .map_err(|error| format!("MCP App CSP metadata is invalid: {error}"))?,
    );
    let permissions =
        serde_json::from_value(ui.get("permissions").cloned().unwrap_or_else(|| json!({})))
            .map_err(|error| format!("MCP App permission metadata is invalid: {error}"))?;
    Ok((html, csp, permissions))
}

pub(super) fn csp_origin(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let wildcard_suffix = raw
        .strip_prefix("https://*.")
        .or_else(|| raw.strip_prefix("wss://*."));
    if let Some(suffix) = wildcard_suffix {
        return valid_domain_name(suffix).then(|| raw.to_string());
    }
    let url = Url::parse(raw).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return None;
    }
    let host = url.host()?;
    if matches!(host, Host::Domain(domain) if !valid_domain_name(domain)) {
        return None;
    }
    let loopback = match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    if !(matches!(url.scheme(), "https" | "wss")
        || matches!(url.scheme(), "http" | "ws") && loopback)
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn valid_domain_name(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

fn sanitize_csp(csp: McpAppResourceCsp) -> McpAppResourceCsp {
    fn sanitize(values: Vec<String>) -> Vec<String> {
        values
            .into_iter()
            .filter_map(|value| csp_origin(&value))
            .collect()
    }
    McpAppResourceCsp {
        connect_domains: sanitize(csp.connect_domains),
        resource_domains: sanitize(csp.resource_domains),
        frame_domains: sanitize(csp.frame_domains),
        base_uri_domains: sanitize(csp.base_uri_domains),
    }
}

fn csp_is_subset(requested: &McpAppResourceCsp, approved: &McpAppResourceCsp) -> bool {
    fn values_are_subset(requested: &[String], approved: &[String]) -> bool {
        let approved = approved.iter().collect::<HashSet<_>>();
        requested.iter().all(|value| approved.contains(value))
    }
    values_are_subset(&requested.connect_domains, &approved.connect_domains)
        && values_are_subset(&requested.resource_domains, &approved.resource_domains)
        && values_are_subset(&requested.frame_domains, &approved.frame_domains)
        && values_are_subset(&requested.base_uri_domains, &approved.base_uri_domains)
}

fn permissions_are_subset(
    requested: &McpAppResourcePermissions,
    approved: &McpAppResourcePermissions,
) -> bool {
    (requested.camera.is_none() || approved.camera.is_some())
        && (requested.microphone.is_none() || approved.microphone.is_some())
        && (requested.geolocation.is_none() || approved.geolocation.is_some())
        && (requested.clipboard_write.is_none() || approved.clipboard_write.is_some())
}

pub(super) fn policy_is_subset(
    requested: &McpAppResourcePolicy,
    approved: &McpAppResourcePolicy,
) -> bool {
    csp_is_subset(&requested.csp, &approved.csp)
        && permissions_are_subset(
            &requested.requested_permissions,
            &approved.requested_permissions,
        )
}

fn sources(values: &[String], fallback: &str) -> String {
    let collected = values
        .iter()
        .filter_map(|value| csp_origin(value))
        .collect::<Vec<_>>();
    if collected.is_empty() {
        fallback.to_string()
    } else {
        collected.join(" ")
    }
}

pub(super) fn sandbox_csp(csp: &McpAppResourceCsp) -> String {
    let resources = sources(&csp.resource_domains, "");
    let connects = sources(&csp.connect_domains, "'none'");
    let frames = sources(&csp.frame_domains, "");
    let bases = sources(&csp.base_uri_domains, "'self'");
    format!(
        "default-src 'none'; script-src 'self' 'unsafe-inline' {resources}; \
         style-src 'self' 'unsafe-inline' {resources}; img-src 'self' data: blob: {resources}; \
         font-src 'self' data: {resources}; media-src 'self' data: blob: {resources}; \
         connect-src {connects}; frame-src 'self' {frames}; base-uri {bases}; \
         object-src 'none'; form-action 'none'"
    )
}
