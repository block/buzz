//! Native build-time signing configuration, shared with build.rs.

use url::Url;

/// Signing backend selected by the native build, not webview/runtime environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignerConfig {
    /// Default OSS key custody and signing behavior.
    Local,
    /// Remote signing with a complete API base (including any deployment prefix).
    Remote {
        /// Complete API URL; the transport appends only `/v1/buzz/identity/sign`.
        api_base: Url,
    },
}

impl SignerConfig {
    /// Validate build inputs. Remote builds must explicitly supply their API base.
    pub fn parse(mode: Option<&str>, api_base: Option<&str>) -> Result<Self, &'static str> {
        match mode.unwrap_or("local") {
            "local" => {
                if api_base.is_some() {
                    return Err("signer API base requires remote signer mode");
                }
                Ok(Self::Local)
            }
            "remote" => Ok(Self::Remote {
                api_base: parse_api_base(api_base.ok_or("remote signer requires an API base")?)?,
            }),
            _ => Err("signer mode must be local or remote"),
        }
    }
}

/// Parse a complete API base without credentials, query, or fragment.
/// HTTPS is required except for literal loopback IPs used in local development.
pub fn parse_api_base(base: &str) -> Result<Url, &'static str> {
    if base.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("invalid signer API base");
    }
    let url = Url::parse(base).map_err(|_| "invalid signer API base")?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.host().is_none()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("signer API base requires HTTPS, a host, and no credentials/query/fragment");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_mode_and_complete_base_validation() {
        assert_eq!(SignerConfig::parse(None, None), Ok(SignerConfig::Local));
        assert_eq!(
            SignerConfig::parse(Some("local"), None),
            Ok(SignerConfig::Local)
        );
        for (mode, base) in [
            (Some(""), None),
            (Some("other"), None),
            (Some("remote"), None),
            (None, Some("https://example.com/api/goose")),
        ] {
            assert!(SignerConfig::parse(mode, base).is_err());
        }
        for base in [
            "",
            "relative/api/goose",
            "http://example.com/api/goose",
            "https://user:secret@example.com",
            "https://example.com/?token=x",
            "https://example.com/#x",
            "https://example.com/\npath",
            "file:///tmp/api",
        ] {
            assert!(
                SignerConfig::parse(Some("remote"), Some(base)).is_err(),
                "{base:?}"
            );
        }
        for base in [
            "https://example.com/cash-app/goose",
            "https://example.com/api/goose/",
            "http://127.0.0.1:1234/api/goose",
            "http://[::1]:1234",
        ] {
            let SignerConfig::Remote { api_base } =
                SignerConfig::parse(Some("remote"), Some(base)).unwrap()
            else {
                panic!("remote")
            };
            assert_eq!(api_base, Url::parse(base).unwrap());
        }
    }
}
