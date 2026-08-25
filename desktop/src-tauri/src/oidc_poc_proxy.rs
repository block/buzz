use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcPocProxyRequest {
    path: String,
    method: String,
    body: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcPocProxyResponse {
    status: u16,
    body: String,
    headers: BTreeMap<String, String>,
}

/// Local POC transport only. The allowlist prevents this command from becoming
/// a generic native-network proxy; authorization still runs in the system
/// browser and PKCE/state validation remains in oidc-client-ts.
#[tauri::command]
pub async fn oidc_poc_proxy(request: OidcPocProxyRequest) -> Result<OidcPocProxyResponse, String> {
    const ALLOWED_PATHS: [&str; 3] = [
        "/_pacioli_oidc/token/",
        "/_pacioli_oidc/userinfo/",
        "/_pacioli_oidc/workbench/jwks/",
    ];
    if !ALLOWED_PATHS.contains(&request.path.as_str()) {
        return Err("OIDC POC proxy path rejected".to_string());
    }
    if request.method != "GET" && request.method != "POST" {
        return Err("OIDC POC proxy method rejected".to_string());
    }

    // Caddy intentionally exposes these routes only on the `localhost`
    // virtual host. Using 127.0.0.1 here selects Caddy's automatic HTTPS
    // redirect instead of the loopback-only POC handler.
    let url = format!("http://localhost{}", request.path);
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| "OIDC POC proxy method rejected".to_string())?;
    let client = reqwest::Client::new();
    let mut outbound = client.request(method, url);
    for name in ["accept", "authorization", "content-type"] {
        if let Some(value) = request.headers.get(name) {
            outbound = outbound.header(name, value);
        }
    }
    if let Some(body) = request.body {
        outbound = outbound.body(body);
    }
    let response = outbound
        .send()
        .await
        .map_err(|_| "OIDC POC proxy request failed".to_string())?;
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for name in ["cache-control", "content-type"] {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            headers.insert(name.to_string(), value.to_string());
        }
    }
    let body = response
        .text()
        .await
        .map_err(|_| "OIDC POC proxy response failed".to_string())?;
    Ok(OidcPocProxyResponse {
        status,
        body,
        headers,
    })
}
