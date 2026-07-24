//! Minimal SOCKS5 CONNECT client (RFC 1928) with optional username/password
//! authentication (RFC 1929).
//!
//! Hand-rolled on purpose: the client side of CONNECT is a few dozen bytes
//! of protocol, and owning it keeps the transport dependency-light. Domain
//! targets are passed to the proxy verbatim (`socks5h` behavior), so name
//! resolution happens inside the private network — required for overlays
//! like Tor where the client cannot resolve the destination itself.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::TransportError;

const SOCKS_VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_USER_PASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const AUTH_SUBNEGOTIATION_VERSION: u8 = 0x01;

/// A parsed `socks5://[user:pass@]host:port` proxy address.
///
/// Credentials are taken verbatim from the URL (no percent-decoding) and
/// must each fit RFC 1929's 255-byte limit.
#[derive(Debug, Clone)]
pub(crate) struct SocksProxy {
    host: url::Host<String>,
    port: u16,
    auth: Option<(String, String)>,
}

impl SocksProxy {
    /// Parse a `socks5://` (or `socks5h://` — both resolve names at the
    /// proxy) URL into a proxy address.
    pub(crate) fn parse(raw: &str) -> Result<Self, TransportError> {
        let url = url::Url::parse(raw)
            .map_err(|e| TransportError::Connection(format!("invalid SOCKS5 proxy URL: {e}")))?;
        match url.scheme() {
            "socks5" | "socks5h" => {}
            other => {
                return Err(TransportError::Connection(format!(
                    "unsupported proxy scheme {other:?} (expected socks5:// or socks5h://)"
                )));
            }
        }
        // `socks5` is not a "special" scheme, so the url crate leaves IP
        // literals as opaque domains — normalize them back to IP hosts.
        let host = match url
            .host()
            .ok_or_else(|| TransportError::Connection("SOCKS5 proxy URL has no host".into()))?
            .to_owned()
        {
            url::Host::Domain(domain) => match domain.parse::<std::net::IpAddr>() {
                Ok(std::net::IpAddr::V4(ip)) => url::Host::Ipv4(ip),
                Ok(std::net::IpAddr::V6(ip)) => url::Host::Ipv6(ip),
                Err(_) => url::Host::Domain(domain),
            },
            other => other,
        };
        let port = url.port().unwrap_or(1080);

        let username = url.username();
        let auth = match (username.is_empty(), url.password()) {
            (true, None) => None,
            (_, password) => {
                let password = password.unwrap_or_default();
                if username.len() > 255 || password.len() > 255 {
                    return Err(TransportError::Connection(
                        "SOCKS5 credentials exceed the 255-byte RFC 1929 limit".into(),
                    ));
                }
                Some((username.to_string(), password.to_string()))
            }
        };

        Ok(Self { host, port, auth })
    }

    /// The proxy's host, for security-policy decisions by the caller.
    pub(crate) fn host(&self) -> &url::Host<String> {
        &self.host
    }

    /// Open a TCP connection to `target_host:target_port` through the proxy:
    /// dial the proxy, negotiate authentication, and issue a CONNECT.
    pub(crate) async fn connect(
        &self,
        target_host: &url::Host<&str>,
        target_port: u16,
    ) -> Result<TcpStream, TransportError> {
        let mut stream = match &self.host {
            url::Host::Domain(domain) => TcpStream::connect((domain.as_str(), self.port)).await,
            url::Host::Ipv4(ip) => TcpStream::connect((std::net::IpAddr::V4(*ip), self.port)).await,
            url::Host::Ipv6(ip) => TcpStream::connect((std::net::IpAddr::V6(*ip), self.port)).await,
        }
        .map_err(|e| TransportError::Connection(format!("SOCKS5 proxy connect failed: {e}")))?;

        self.negotiate_auth(&mut stream).await?;
        send_connect(&mut stream, target_host, target_port).await?;
        Ok(stream)
    }

    /// Method negotiation plus (if selected) RFC 1929 username/password
    /// subnegotiation.
    async fn negotiate_auth(&self, stream: &mut TcpStream) -> Result<(), TransportError> {
        let methods: &[u8] = if self.auth.is_some() {
            &[METHOD_NO_AUTH, METHOD_USER_PASS]
        } else {
            &[METHOD_NO_AUTH]
        };
        let mut greeting = vec![SOCKS_VERSION, methods.len() as u8];
        greeting.extend_from_slice(methods);
        stream.write_all(&greeting).await.map_err(socks_io_error)?;

        let mut choice = [0u8; 2];
        stream
            .read_exact(&mut choice)
            .await
            .map_err(socks_io_error)?;
        if choice[0] != SOCKS_VERSION {
            return Err(TransportError::Connection(format!(
                "proxy is not SOCKS5 (version byte {:#04x})",
                choice[0]
            )));
        }
        match choice[1] {
            METHOD_NO_AUTH => Ok(()),
            METHOD_USER_PASS => {
                let Some((user, pass)) = &self.auth else {
                    return Err(TransportError::Connection(
                        "proxy requires username/password but none were configured".into(),
                    ));
                };
                let mut request = vec![AUTH_SUBNEGOTIATION_VERSION, user.len() as u8];
                request.extend_from_slice(user.as_bytes());
                request.push(pass.len() as u8);
                request.extend_from_slice(pass.as_bytes());
                stream.write_all(&request).await.map_err(socks_io_error)?;

                let mut status = [0u8; 2];
                stream
                    .read_exact(&mut status)
                    .await
                    .map_err(socks_io_error)?;
                if status[0] != AUTH_SUBNEGOTIATION_VERSION {
                    return Err(TransportError::Connection(format!(
                        "proxy sent unsupported username/password subnegotiation \
                         version {:#04x}",
                        status[0]
                    )));
                }
                if status[1] != 0 {
                    return Err(TransportError::Connection(
                        "proxy rejected the SOCKS5 username/password".into(),
                    ));
                }
                Ok(())
            }
            METHOD_NONE_ACCEPTABLE => Err(TransportError::Connection(
                "proxy accepted none of the offered SOCKS5 authentication methods".into(),
            )),
            other => Err(TransportError::Connection(format!(
                "proxy selected unsupported SOCKS5 method {other:#04x}"
            ))),
        }
    }
}

/// Issue the CONNECT request and consume the proxy's reply, leaving the
/// stream positioned at the start of the tunneled byte stream.
async fn send_connect(
    stream: &mut TcpStream,
    target_host: &url::Host<&str>,
    target_port: u16,
) -> Result<(), TransportError> {
    let mut request = vec![SOCKS_VERSION, CMD_CONNECT, 0x00];
    match target_host {
        url::Host::Domain(domain) => {
            let bytes = domain.as_bytes();
            let len = u8::try_from(bytes.len()).map_err(|_| {
                TransportError::Connection("target hostname exceeds 255 bytes".into())
            })?;
            request.push(ATYP_DOMAIN);
            request.push(len);
            request.extend_from_slice(bytes);
        }
        url::Host::Ipv4(ip) => {
            request.push(ATYP_IPV4);
            request.extend_from_slice(&ip.octets());
        }
        url::Host::Ipv6(ip) => {
            request.push(ATYP_IPV6);
            request.extend_from_slice(&ip.octets());
        }
    }
    request.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&request).await.map_err(socks_io_error)?;

    // Reply: VER REP RSV ATYP BND.ADDR BND.PORT. The bind address must be
    // fully consumed so the tunnel starts aligned.
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(socks_io_error)?;
    if head[0] != SOCKS_VERSION {
        return Err(TransportError::Connection(format!(
            "proxy sent a non-SOCKS5 reply (version byte {:#04x})",
            head[0]
        )));
    }
    if head[1] != 0 {
        return Err(TransportError::Connection(format!(
            "proxy refused the connection: {}",
            reply_reason(head[1])
        )));
    }
    let addr_len = match head[3] {
        ATYP_IPV4 => 4,
        ATYP_IPV6 => 16,
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(socks_io_error)?;
            len[0] as usize
        }
        other => {
            return Err(TransportError::Connection(format!(
                "proxy reply has unknown address type {other:#04x}"
            )));
        }
    };
    let mut bind = vec![0u8; addr_len + 2];
    stream.read_exact(&mut bind).await.map_err(socks_io_error)?;
    Ok(())
}

fn socks_io_error(e: std::io::Error) -> TransportError {
    TransportError::Connection(format!("SOCKS5 handshake failed: {e}"))
}

/// Human-readable RFC 1928 §6 reply codes.
fn reply_reason(code: u8) -> &'static str {
    match code {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown reply code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_port_and_default_port() {
        let proxy = SocksProxy::parse("socks5://127.0.0.1:9050").unwrap();
        assert_eq!(proxy.host, url::Host::parse("127.0.0.1").unwrap());
        assert_eq!(proxy.port, 9050);
        assert!(proxy.auth.is_none());

        let defaulted = SocksProxy::parse("socks5://proxy.internal").unwrap();
        assert_eq!(defaulted.port, 1080);

        let hostname_resolving = SocksProxy::parse("socks5h://proxy.internal:1081").unwrap();
        assert_eq!(hostname_resolving.port, 1081);
    }

    #[test]
    fn parses_credentials() {
        let proxy = SocksProxy::parse("socks5://agent:hunter2@10.0.0.1:1080").unwrap();
        assert_eq!(
            proxy.auth,
            Some(("agent".to_string(), "hunter2".to_string()))
        );

        let user_only = SocksProxy::parse("socks5://agent@10.0.0.1:1080").unwrap();
        assert_eq!(user_only.auth, Some(("agent".to_string(), String::new())));
    }

    #[test]
    fn rejects_non_socks_schemes_and_oversized_credentials() {
        assert!(matches!(
            SocksProxy::parse("http://proxy.internal:8080"),
            Err(TransportError::Connection(_))
        ));
        assert!(matches!(
            SocksProxy::parse("not a url"),
            Err(TransportError::Connection(_))
        ));
        let oversized = format!("socks5://{}:pw@10.0.0.1:1080", "u".repeat(256));
        assert!(matches!(
            SocksProxy::parse(&oversized),
            Err(TransportError::Connection(_))
        ));
    }
}
