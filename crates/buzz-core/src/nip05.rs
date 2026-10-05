//! NIP-05 handle canonicalization shared by the relay's kind-0 projection,
//! its NIP-05 lookup endpoint, and the `buzz-admin` profile backfill.
//!
//! The relay and the backfill must agree on which `nip05` values from a kind-0
//! profile become `users.nip05_handle`; keeping the rule here means there is
//! exactly one definition for both writers.

/// Validate and canonicalize a NIP-05 handle: must be `local@domain` where domain
/// matches the bound tenant host. Returns the lowercased canonical form, or an
/// error message. `expected_host_or_url` may be either a bare Host authority or
/// a relay URL; only its host/domain component is compared.
pub fn canonicalize_nip05(raw: &str, expected_host_or_url: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty".into());
    }
    let (local, domain) = trimmed
        .split_once('@')
        .ok_or_else(|| "nip05_handle must be in user@domain format".to_string())?;
    if local.is_empty() || domain.is_empty() {
        return Err("nip05_handle must be in user@domain format".to_string());
    }
    let expected_domain = extract_domain(expected_host_or_url);
    let canonical_domain = domain.to_lowercase();
    if canonical_domain != expected_domain {
        return Err(format!(
            "nip05_handle domain must match this relay ({})",
            expected_domain
        ));
    }
    Ok(format!("{}@{}", local.to_lowercase(), canonical_domain))
}

/// Extract the domain (host) from a URL string.
/// e.g. "ws://localhost:3000" → "localhost", "wss://buzz.block.xyz" → "buzz.block.xyz"
pub fn extract_domain(url: &str) -> String {
    url.trim_start_matches("wss://")
        .trim_start_matches("ws://")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .split('/')
        .next()
        .unwrap_or("localhost")
        .to_lowercase()
}
