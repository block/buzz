//! Canonical relay identities shared by runtime components.

use thiserror::Error;
use url::{Host, Url};

/// Append an HTTP endpoint to a relay base path without changing its query bytes.
///
/// `endpoint` is a relay-relative path, optionally with a query string. Endpoint
/// query parameters follow the configured relay parameters, without decoding or
/// re-encoding either query. A base fragment is omitted from the request URL.
pub fn relay_http_endpoint(base: &str, endpoint: &str) -> Result<Url, url::ParseError> {
    let mut url = Url::parse(base)?;
    let (path, query) = endpoint
        .split_once('?')
        .map_or((endpoint, None), |(path, query)| (path, Some(query)));
    url.set_path(&format!(
        "{}/{}",
        url.path().trim_end_matches('/'),
        path.trim_start_matches('/')
    ));
    if let Some(query) = query {
        let combined = match url.query() {
            Some(base_query) if !base_query.is_empty() && !query.is_empty() => {
                format!("{base_query}&{query}")
            }
            Some(base_query) if query.is_empty() => base_query.to_owned(),
            _ => query.to_owned(),
        };
        url.set_query(Some(&combined));
    }
    url.set_fragment(None);
    Ok(url)
}

/// Errors returned while canonicalizing a relay URL for runtime identity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NormalizeRelayUrlError {
    /// The input is not a valid URL.
    #[error("invalid relay URL: {0}")]
    InvalidUrl(String),
    /// Relay sockets must use WebSocket schemes.
    #[error("relay URL scheme must be ws or wss")]
    InvalidScheme,
    /// Relay identity never includes user credentials.
    #[error("relay URL must not contain credentials")]
    Credentials,
    /// Relay identity never includes a fragment.
    #[error("relay URL must not contain a fragment")]
    Fragment,
    /// A relay URL requires a host.
    #[error("relay URL must contain a host")]
    MissingHost,
}

/// Canonicalize a WebSocket relay URL for use as a runtime identity key.
///
/// This is the sole normalizer for `(agent, relay)` process identity. It keeps
/// the WebSocket scheme, lowercases DNS hosts, folds all loopback spellings to
/// `127.0.0.1`, removes default ports and a root slash, and preserves non-root
/// paths and queries. It deliberately is **not** the NIP-42 AUTH comparison
/// helper in `buzz-auth`: AUTH validation is a security boundary with narrower
/// equivalence rules and must not be widened by runtime-key canonicalization.
///
/// Connection code may retain the configured URL; this canonical form is for
/// identity, receipts, status and deduplication.
pub fn normalize_relay_url(raw: &str) -> Result<String, NormalizeRelayUrlError> {
    let mut url = Url::parse(raw.trim())
        .map_err(|error| NormalizeRelayUrlError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "ws" | "wss") {
        return Err(NormalizeRelayUrlError::InvalidScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NormalizeRelayUrlError::Credentials);
    }
    if url.fragment().is_some() {
        return Err(NormalizeRelayUrlError::Fragment);
    }

    let host = url.host().ok_or(NormalizeRelayUrlError::MissingHost)?;
    let loopback = match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    if loopback {
        url.set_host(Some("127.0.0.1"))
            .map_err(|_| NormalizeRelayUrlError::MissingHost)?;
    } else if let Host::Domain(domain) = host {
        let lowercase = domain.to_ascii_lowercase();
        url.set_host(Some(&lowercase))
            .map_err(|_| NormalizeRelayUrlError::MissingHost)?;
    }

    let default_port = match url.scheme() {
        "ws" => Some(80),
        "wss" => Some(443),
        _ => None,
    };
    if url.port() == default_port {
        url.set_port(None)
            .map_err(|_| NormalizeRelayUrlError::InvalidScheme)?;
    }
    let had_root_path = url.path() == "/";
    if had_root_path {
        url.set_path("");
    }
    let mut serialized = url.to_string();
    if had_root_path {
        remove_root_path_separator(&mut serialized);
    }
    Ok(serialized)
}

fn remove_root_path_separator(serialized: &mut String) {
    let delimiter = serialized.find(['?', '#']);
    match delimiter {
        Some(index) if serialized.as_bytes().get(index.wrapping_sub(1)) == Some(&b'/') => {
            serialized.remove(index - 1);
        }
        None if serialized.ends_with('/') => {
            serialized.pop();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_endpoints_preserve_base_paths_and_raw_queries() {
        for (base, endpoint, expected) in [
            ("https://relay.example", "/query", "https://relay.example/query"),
            ("https://relay.example/nostr/", "/query", "https://relay.example/nostr/query"),
            ("https://relay.example?token=abc", "/query", "https://relay.example/query?token=abc"),
            ("https://relay.example/nostr/?token=abc", "/query", "https://relay.example/nostr/query?token=abc"),
            ("https://relay.example/n%2Fostr?next=/", "/info", "https://relay.example/n%2Fostr/info?next=/"),
            ("https://relay.example?token=a%2fb+%20&token=2&url=wss://other/", "/reports?status=open", "https://relay.example/reports?token=a%2fb+%20&token=2&url=wss://other/&status=open"),
            ("https://relay.example/nostr/?token=abc", "/", "https://relay.example/nostr/?token=abc"),
            ("http://relay.example:8080?", "/query", "http://relay.example:8080/query?"),
            ("https://relay.example?", "/reports?status=open", "https://relay.example/reports?status=open"),
            ("https://relay.example?token=abc", "/query?", "https://relay.example/query?token=abc"),
            ("https://relay.example", "/reports?status=open", "https://relay.example/reports?status=open"),
            ("https://relay.example?token=abc#fragment", "/query", "https://relay.example/query?token=abc"),
        ] {
            assert_eq!(relay_http_endpoint(base, endpoint).unwrap().as_str(), expected);
        }
        assert!(relay_http_endpoint("not a URL", "/query").is_err());
    }

    #[test]
    fn loopback_spellings_have_one_identity() {
        let ipv6 = normalize_relay_url("wss://[::1]/").unwrap();
        let ipv4 = normalize_relay_url("wss://127.0.0.1/").unwrap();
        let localhost = normalize_relay_url("wss://localhost/").unwrap();
        assert_eq!(ipv6, ipv4);
        assert_eq!(ipv4, localhost);
        assert_eq!(localhost, "wss://127.0.0.1");
    }

    #[test]
    fn canonicalizes_only_identity_equivalences() {
        assert_eq!(
            normalize_relay_url(" WSS://Relay.Example:443/ ").unwrap(),
            "wss://relay.example"
        );
        assert_eq!(
            normalize_relay_url("ws://relay.example:8080/community/?x=1").unwrap(),
            "ws://relay.example:8080/community/?x=1"
        );
    }

    #[test]
    fn preserves_queries_when_removing_root_path() {
        for (input, expected) in [
            ("wss://relay.example/?next=/", "wss://relay.example?next=/"),
            (
                "wss://relay.example/?next=/foo/",
                "wss://relay.example?next=/foo/",
            ),
            (
                "wss://relay.example/?url=wss://other.example/",
                "wss://relay.example?url=wss://other.example/",
            ),
        ] {
            assert_eq!(normalize_relay_url(input).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_non_relay_and_ambiguous_urls() {
        assert_eq!(
            normalize_relay_url("https://relay.example").unwrap_err(),
            NormalizeRelayUrlError::InvalidScheme
        );
        assert_eq!(
            normalize_relay_url("wss://user@relay.example").unwrap_err(),
            NormalizeRelayUrlError::Credentials
        );
        assert_eq!(
            normalize_relay_url("wss://relay.example/#x").unwrap_err(),
            NormalizeRelayUrlError::Fragment
        );
    }
}
