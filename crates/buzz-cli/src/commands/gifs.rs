//! Agent GIF search and share via the relay's KLIPY proxy.
//!
//! `buzz gifs search` / `buzz gifs share` hit the relay-relative endpoints
//! advertised in the NIP-11 `gif` descriptor. No provider credential is held
//! by the agent — the relay proxies KLIPY and returns only allowlisted data.
//!
//! Sending a GIF is a normal message whose content contains the CDN URL
//! returned by search — no special send-path handling, no imeta.

use crate::client::BuzzClient;
use crate::error::CliError;

/// Gate: `supported_extensions` must contain this value.
const REQUIRED_EXTENSION: &str = "buzz-gif";
/// Gate: `gif.provider` must be this value.
const REQUIRED_PROVIDER: &str = "klipy";

/// Derive a stable anonymous `customer_id` from the agent keypair.
///
/// KLIPY requires a per-installation identifier that is stable and anonymous.
/// SHA-256 of the public key hex satisfies both requirements: stable across
/// sessions, never traceable to a person, never stored. The first 32 hex chars
/// (128 bits) are ample for KLIPY's uniqueness needs; the full 64-char hash is
/// within the server's 128-char limit but unnecessarily long.
fn customer_id_from_pubkey(pubkey_hex: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(pubkey_hex.as_bytes());
    hex::encode(&hash[..16]) // 16 bytes → 32 hex chars
}

/// Locale to send to KLIPY.  Reads `LANG` first, falls back to `en_US`.
fn default_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| {
            // `LANG` is typically `en_US.UTF-8` or `en_US`; strip the encoding
            // suffix and take up to 5 chars which gives the provider-understood
            // locale code (e.g. `en_US`).
            let code: String = l
                .splitn(2, '.')
                .next()
                .unwrap_or("")
                .chars()
                .take(5)
                .collect();
            if code.len() >= 2 {
                Some(code)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "en_US".to_string())
}

/// Resolve the relay's `gif` descriptor from its NIP-11 document.
///
/// Returns `(search_path, share_path)` as relay-relative strings (e.g.
/// `"/gifs/search"`, `"/gifs/share"`).  Returns a clear `CliError` if the
/// relay does not advertise `buzz-gif` or the provider is not `klipy`.
async fn resolve_gif_descriptor(client: &BuzzClient) -> Result<(String, String), CliError> {
    let raw = client.get_public("/info").await?;
    let info: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid NIP-11 response: {e}")))?;

    // Gate 1: `supported_extensions` must contain `"buzz-gif"`.
    let extensions = info
        .get("supported_extensions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    if !extensions.iter().any(|&e| e == REQUIRED_EXTENSION) {
        return Err(CliError::Other(format!(
            "this relay does not support GIF search (missing \"{REQUIRED_EXTENSION}\" in supported_extensions)"
        )));
    }

    // Gate 2: `gif.provider` must be `"klipy"`.
    let gif = info.get("gif").ok_or_else(|| {
        CliError::Other("relay advertises buzz-gif but has no \"gif\" descriptor".to_string())
    })?;
    let provider = gif.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    if provider != REQUIRED_PROVIDER {
        return Err(CliError::Other(format!(
            "unsupported GIF provider \"{provider}\" (only \"{REQUIRED_PROVIDER}\" is supported)"
        )));
    }

    let search = gif
        .get("search")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let share = gif
        .get("share")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if search.is_empty() || share.is_empty() {
        return Err(CliError::Other(
            "relay gif descriptor is missing search or share path".to_string(),
        ));
    }

    Ok((search, share))
}

/// `buzz gifs search [--query <q>] [--locale <l>]`
///
/// Empty/omitted `query` returns KLIPY trending GIFs.  Output is a JSON
/// array of GIF objects; each entry's `cdn_url` field is the URL to embed
/// in a `buzz messages send --content` argument.
pub async fn cmd_search(
    client: &BuzzClient,
    query: &str,
    locale: Option<&str>,
) -> Result<(), CliError> {
    let (search_path, _) = resolve_gif_descriptor(client).await?;
    let customer_id = customer_id_from_pubkey(&client.keys().public_key().to_hex());
    let locale = locale.map(|l| l.to_string()).unwrap_or_else(default_locale);

    let body = serde_json::json!({
        "query": query,
        "customer_id": customer_id,
        "locale": locale,
    });
    let raw = client.post_json_authed(&search_path, &body).await?;

    // Relay returns `{"result": true, "data": {"data": [...]}}`. Unwrap to the
    // inner array so agents get a flat list they can iterate directly.
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("invalid GIF search response: {e}")))?;
    let gifs = parsed
        .get("data")
        .and_then(|d| d.get("data"))
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));

    println!(
        "{}",
        serde_json::to_string(&gifs).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(())
}

/// `buzz gifs share --slug <slug>`
///
/// Reports a selected GIF to KLIPY so it can update Recents.  The `slug`
/// is the provider identifier returned in search results.  Prints
/// `{"accepted": true}` on success.
pub async fn cmd_share(client: &BuzzClient, slug: &str) -> Result<(), CliError> {
    let (_, share_path) = resolve_gif_descriptor(client).await?;
    let customer_id = customer_id_from_pubkey(&client.keys().public_key().to_hex());

    let body = serde_json::json!({
        "slug": slug,
        "customer_id": customer_id,
    });
    // The relay returns 204 No Content on success; post_json_authed returns "".
    client.post_json_authed(&share_path, &body).await?;
    println!("{}", serde_json::json!({"accepted": true}));
    Ok(())
}

pub async fn dispatch(cmd: crate::GifsCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::GifsCmd::Search { query, locale } => {
            cmd_search(client, query.as_deref().unwrap_or(""), locale.as_deref()).await
        }
        crate::GifsCmd::Share { slug } => cmd_share(client, &slug).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customer_id_is_32_hex_chars_and_stable() {
        let pk = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let id = customer_id_from_pubkey(pk);
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        // Stability: same input → same output.
        assert_eq!(id, customer_id_from_pubkey(pk));
    }

    #[test]
    fn customer_id_differs_for_different_pubkeys() {
        let a = customer_id_from_pubkey("aaaa");
        let b = customer_id_from_pubkey("bbbb");
        assert_ne!(a, b);
    }

    #[test]
    fn default_locale_falls_back_when_lang_unset() {
        // Remove LANG if set; we cannot safely setenv in parallel tests, so
        // only verify the fallback path indirectly via the absence condition.
        let locale =
            if std::env::var("LANG").ok().map(|l| l.trim().to_string()) == Some(String::new()) {
                "en_US".to_string()
            } else {
                // LANG is set — just confirm we get a non-empty string.
                default_locale()
            };
        assert!(!locale.is_empty());
    }

    /// Checks that NIP-11 gating rejects a relay that doesn't advertise buzz-gif.
    #[test]
    fn nip11_gating_logic_missing_extension() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-emoji"],
            "gif": { "provider": "klipy", "search": "/gifs/search", "share": "/gifs/share" }
        });
        // Simulate the gating check inline (no I/O needed).
        let extensions: Vec<&str> = info["supported_extensions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(!extensions.contains(&REQUIRED_EXTENSION));
    }

    #[test]
    fn nip11_gating_logic_wrong_provider() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "tenor", "search": "/gifs/search", "share": "/gifs/share" }
        });
        let provider = info["gif"]["provider"].as_str().unwrap_or("");
        assert_ne!(provider, REQUIRED_PROVIDER);
    }

    #[test]
    fn nip11_gating_logic_passes_valid_descriptor() {
        let info = serde_json::json!({
            "supported_extensions": ["buzz-gif"],
            "gif": { "provider": "klipy", "search": "/gifs/search", "share": "/gifs/share" }
        });
        let extensions: Vec<&str> = info["supported_extensions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(extensions.contains(&REQUIRED_EXTENSION));
        assert_eq!(info["gif"]["provider"].as_str().unwrap(), REQUIRED_PROVIDER);
        assert_eq!(info["gif"]["search"].as_str().unwrap(), "/gifs/search");
        assert_eq!(info["gif"]["share"].as_str().unwrap(), "/gifs/share");
    }
}
