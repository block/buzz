//! Declarative read-only HTTP endpoints — the `http_get` tool.
//!
//! An operator can hand the agent a curated, read-only slice of an internal
//! HTTP API without teaching it curl, without pasting credentials into
//! prompts, and without forking this crate per deployment: a JSON manifest
//! (path in `BUZZ_DEV_MCP_HTTP_MANIFEST`) names the endpoints, and this
//! module fetches them. GET only, allowlist only — an endpoint that is not
//! in the manifest does not exist as far as the tool is concerned.
//!
//! The manifest is deployment-specific and lives with the operator, not
//! here. Auth is a single header whose VALUE is read from an env var at
//! call time, so a rotated credential is picked up without a restart and
//! never appears in the manifest, the tool schema, or the conversation.
//!
//! Endpoint documentation is injected into the server *instructions*
//! (`get_info`) rather than the tool description: tool descriptions are
//! static macro literals, instructions are built at startup.

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Response-body budget for the LLM, matching the shell tool's ~8KB.
const BODY_BUDGET: usize = 8 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

pub const MANIFEST_ENV: &str = "BUZZ_DEV_MCP_HTTP_MANIFEST";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Human name for the instructions section, e.g. "Centro de Control".
    pub title: String,
    pub base_url: String,
    #[serde(default)]
    pub auth: Option<AuthSpec>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Deserialize)]
pub struct AuthSpec {
    /// Header to send, e.g. "Authorization".
    pub header: String,
    /// Env var holding the secret, read per call — never stored here.
    pub env: String,
    /// Optional value prefix, e.g. "Bearer ".
    #[serde(default)]
    pub prefix: String,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub name: String,
    /// Path with optional `{placeholder}` segments filled from `path_params`.
    pub path: String,
    pub description: String,
    /// Allowed query parameter names; anything else is rejected.
    #[serde(default)]
    pub query: Vec<String>,
    /// Placeholder names the path requires, e.g. ["token"] for `/api/hq/{token}`.
    #[serde(default)]
    pub path_params: Vec<String>,
}

/// Load the manifest named by `BUZZ_DEV_MCP_HTTP_MANIFEST`.
///
/// `Ok(None)` when the env var is unset (feature off). `Err` when it is set
/// but the file is missing or invalid — the caller surfaces that loudly in
/// the server instructions, because a manifest that silently fails to load
/// looks exactly like a manifest that was never configured.
pub fn load_from_env() -> Result<Option<Manifest>, String> {
    let Some(raw_path) = std::env::var_os(MANIFEST_ENV) else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(raw_path);
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: Manifest =
        serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    validate(&manifest)?;
    Ok(Some(manifest))
}

fn validate(m: &Manifest) -> Result<(), String> {
    if !(m.base_url.starts_with("http://") || m.base_url.starts_with("https://")) {
        return Err(format!("base_url must be http(s), got {:?}", m.base_url));
    }
    if m.endpoints.is_empty() {
        return Err("manifest has no endpoints".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for ep in &m.endpoints {
        if !seen.insert(ep.name.as_str()) {
            return Err(format!("duplicate endpoint name {:?}", ep.name));
        }
        // Every {placeholder} in the path must be declared, and vice versa —
        // catching the mismatch at load time, not on the first 3am call.
        let mut in_path = std::collections::BTreeSet::new();
        let mut rest = ep.path.as_str();
        while let Some(start) = rest.find('{') {
            let Some(len) = rest[start..].find('}') else {
                return Err(format!("endpoint {:?}: unclosed '{{' in path", ep.name));
            };
            in_path.insert(&rest[start + 1..start + len]);
            rest = &rest[start + len + 1..];
        }
        let declared: std::collections::BTreeSet<&str> =
            ep.path_params.iter().map(String::as_str).collect();
        if in_path != declared {
            return Err(format!(
                "endpoint {:?}: path placeholders {in_path:?} != declared path_params {declared:?}",
                ep.name
            ));
        }
    }
    Ok(())
}

/// The instructions section advertising the configured endpoints.
pub fn instructions_section(m: &Manifest) -> String {
    let mut out = format!(
        "\n\n## http_get endpoints — {}\nRead-only GETs against {}. \
         Call the `http_get` tool with `endpoint` (name below), plus \
         `path_params` / `query` where listed.\n",
        m.title, m.base_url
    );
    for ep in &m.endpoints {
        out.push_str(&format!("- {}: {}", ep.name, ep.description));
        if !ep.path_params.is_empty() {
            out.push_str(&format!(" (path_params: {})", ep.path_params.join(", ")));
        }
        if !ep.query.is_empty() {
            out.push_str(&format!(" (query: {})", ep.query.join(", ")));
        }
        out.push('\n');
    }
    out
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HttpGetParams {
    /// Endpoint name — the available names are listed in the server
    /// instructions under "http_get endpoints".
    pub endpoint: String,
    /// Values for the endpoint's path placeholders (e.g. {"token": "HQ-90"}).
    #[serde(default)]
    pub path_params: Option<BTreeMap<String, String>>,
    /// Query parameters; only names the endpoint allows are accepted.
    #[serde(default)]
    pub query: Option<BTreeMap<String, String>>,
}

/// Pure request-shape resolution, separated for testability: endpoint lookup,
/// placeholder substitution, and query allowlisting — everything that can be
/// wrong before the network is involved.
fn resolve(m: &Manifest, p: &HttpGetParams) -> Result<(String, Vec<(String, String)>), String> {
    let Some(ep) = m.endpoints.iter().find(|e| e.name == p.endpoint) else {
        let names: Vec<&str> = m.endpoints.iter().map(|e| e.name.as_str()).collect();
        return Err(format!(
            "unknown endpoint {:?}; configured: {}",
            p.endpoint,
            names.join(", ")
        ));
    };

    let empty = BTreeMap::new();
    let supplied = p.path_params.as_ref().unwrap_or(&empty);
    for key in supplied.keys() {
        if !ep.path_params.contains(key) {
            return Err(format!(
                "endpoint {:?} takes no path param {:?}",
                ep.name, key
            ));
        }
    }
    let mut path = ep.path.clone();
    for name in &ep.path_params {
        let Some(value) = supplied.get(name) else {
            return Err(format!("missing path param {:?}", name));
        };
        // Conservative charset: enough for ids like HQ-90 or UUIDs, and no
        // way to smuggle a '/', '?', '#' or '..' into the path.
        let ok = !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
            && !value.contains("..");
        if !ok {
            return Err(format!(
                "path param {:?} may only contain [A-Za-z0-9._:-] (got {:?})",
                name, value
            ));
        }
        path = path.replace(&format!("{{{name}}}"), value);
    }

    let empty_q = BTreeMap::new();
    let query = p.query.as_ref().unwrap_or(&empty_q);
    for key in query.keys() {
        if !ep.query.contains(key) {
            return Err(format!(
                "endpoint {:?} does not allow query param {:?} (allowed: {})",
                ep.name,
                key,
                if ep.query.is_empty() {
                    "none".to_string()
                } else {
                    ep.query.join(", ")
                }
            ));
        }
    }

    let url = format!("{}{}", m.base_url.trim_end_matches('/'), path);
    let pairs = query.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    Ok((url, pairs))
}

/// RFC 3986 unreserved-set percent encoding. Hand-rolled because the
/// workspace reqwest is built without its query-builder feature, and one
/// loop beats a new dependency.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn append_query(url: String, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return url;
    }
    let qs: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect();
    format!("{url}?{}", qs.join("&"))
}

fn tool_error(msg: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(msg)]))
}

pub async fn run(m: &Manifest, p: HttpGetParams) -> Result<CallToolResult, ErrorData> {
    let (url, query) = match resolve(m, &p) {
        Ok(r) => r,
        Err(e) => return tool_error(e),
    };

    let timeout = std::time::Duration::from_millis(m.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => return tool_error(format!("http client: {e}")),
    };
    let url = append_query(url, &query);
    let mut req = client.get(&url);
    if let Some(auth) = &m.auth {
        // Read per call: a credential rotated on disk/env is live immediately,
        // and its value never lands in the manifest or the transcript.
        let Ok(secret) = std::env::var(&auth.env) else {
            return tool_error(format!(
                "endpoint auth env {} is unset — the operator must provide it",
                auth.env
            ));
        };
        req = req.header(&auth.header, format!("{}{}", auth.prefix, secret));
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return tool_error(format!("GET {}: {e}", p.endpoint)),
    };
    let status = resp.status();
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => return tool_error(format!("GET {}: reading body: {e}", p.endpoint)),
    };
    let total = body.len();
    let mut shown: String = body.chars().take(BODY_BUDGET).collect();
    if total > shown.len() {
        shown.push_str(&format!("\n… truncated ({total} bytes total)"));
    }

    if status.is_success() {
        Ok(CallToolResult::success(vec![Content::text(format!(
            "HTTP {status} {}\n{shown}",
            p.endpoint
        ))]))
    } else {
        // A non-2xx is a real answer, not a transport failure — surface the
        // status AND the body: the upstream's 503 {ok:false,error} is exactly
        // what distinguishes "backend down" from "nothing to report".
        tool_error(format!("HTTP {status} {}\n{shown}", p.endpoint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        serde_json::from_str(
            r#"{
              "title": "t", "base_url": "http://example.test",
              "endpoints": [
                {"name": "hq", "path": "/api/hq", "description": "board"},
                {"name": "item", "path": "/api/hq/{token}", "description": "one",
                 "path_params": ["token"]},
                {"name": "health", "path": "/api/health", "description": "h",
                 "query": ["fresh"]}
              ]
            }"#,
        )
        .and_then(|m| {
            validate(&m).map_err(serde::de::Error::custom)?;
            Ok(m)
        })
        .unwrap_or_else(|e| panic!("fixture: {e}"))
    }

    fn params(endpoint: &str) -> HttpGetParams {
        HttpGetParams {
            endpoint: endpoint.into(),
            path_params: None,
            query: None,
        }
    }

    #[test]
    fn resolves_plain_endpoint() {
        let (url, q) = resolve(&manifest(), &params("hq")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(url, "http://example.test/api/hq");
        assert!(q.is_empty());
    }

    #[test]
    fn substitutes_path_params_and_rejects_traversal() {
        let mut p = params("item");
        p.path_params = Some([("token".to_string(), "HQ-90".to_string())].into());
        let (url, _) = resolve(&manifest(), &p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(url, "http://example.test/api/hq/HQ-90");

        let mut evil = params("item");
        evil.path_params = Some([("token".to_string(), "../users".to_string())].into());
        assert!(resolve(&manifest(), &evil).is_err());
    }

    #[test]
    fn missing_and_unknown_params_are_named() {
        let err = resolve(&manifest(), &params("item")).unwrap_err();
        assert!(err.contains("missing path param"), "{err}");

        let mut p = params("hq");
        p.query = Some([("fresh".to_string(), "1".to_string())].into());
        let err = resolve(&manifest(), &p).unwrap_err();
        assert!(err.contains("does not allow query param"), "{err}");
    }

    #[test]
    fn allowed_query_passes_through() {
        let mut p = params("health");
        p.query = Some([("fresh".to_string(), "1".to_string())].into());
        let (_, q) = resolve(&manifest(), &p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(q, vec![("fresh".to_string(), "1".to_string())]);
    }

    #[test]
    fn query_string_is_percent_encoded() {
        let url = append_query(
            "http://x/api".to_string(),
            &[("q".to_string(), "a b&c".to_string())],
        );
        assert_eq!(url, "http://x/api?q=a%20b%26c");
        assert_eq!(append_query("http://x".to_string(), &[]), "http://x");
    }

    #[test]
    fn unknown_endpoint_lists_configured_names() {
        let err = resolve(&manifest(), &params("nope")).unwrap_err();
        assert!(err.contains("configured: hq, item, health"), "{err}");
    }

    #[test]
    fn validate_catches_placeholder_mismatch() {
        let bad: Manifest = serde_json::from_str(
            r#"{"title":"t","base_url":"http://x",
                "endpoints":[{"name":"a","path":"/x/{id}","description":"d"}]}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn instructions_list_every_endpoint() {
        let text = instructions_section(&manifest());
        for name in ["hq", "item", "health", "fresh", "token"] {
            assert!(text.contains(name), "missing {name} in:\n{text}");
        }
    }
}
