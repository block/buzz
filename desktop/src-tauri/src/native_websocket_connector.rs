use std::{borrow::Cow, env, io, net::IpAddr};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_socks::tcp::Socks5Stream;
use tokio_tungstenite::{
    client_async_tls_with_config,
    tungstenite::{self, handshake::client::Response},
    MaybeTlsStream, WebSocketStream,
};
use url::Url;

const MAX_PROXY_RESPONSE_HEADER_BYTES: usize = 16 * 1024;

pub(crate) trait AsyncSocket: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> AsyncSocket for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

type BoxedSocket = Box<dyn AsyncSocket>;
pub(crate) type NativeWebSocket = WebSocketStream<MaybeTlsStream<BoxedSocket>>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RelayConnectError {
    #[error("relay connection failed during url-parse: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("relay connection failed during url-parse: unsupported scheme {0}")]
    UnsupportedRelayScheme(String),
    #[error("relay connection failed during url-parse: relay URL has no host")]
    MissingRelayHost,
    #[error("relay connection failed during proxy-config: {0}")]
    ProxyConfig(String),
    #[error("relay connection failed during {stage} ({endpoint}): {source}")]
    Io {
        stage: &'static str,
        endpoint: String,
        #[source]
        source: io::Error,
    },
    #[error("relay connection failed during proxy-handshake ({endpoint}): {detail}")]
    ProxyHandshake { endpoint: String, detail: String },
    #[error("relay connection failed during proxy-tunnel ({endpoint}): HTTP {status}")]
    ProxyTunnelStatus { endpoint: String, status: u16 },
    #[error("relay connection failed during {stage}: {source}")]
    WebSocket {
        stage: &'static str,
        #[source]
        source: tungstenite::Error,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelayScheme {
    Ws,
    Wss,
}

#[derive(Debug)]
struct RelayTarget {
    host: String,
    port: u16,
    scheme: RelayScheme,
}

impl RelayTarget {
    fn parse(value: &str) -> Result<Self, RelayConnectError> {
        let url = Url::parse(value)?;
        let scheme = match url.scheme() {
            "ws" => RelayScheme::Ws,
            "wss" => RelayScheme::Wss,
            other => return Err(RelayConnectError::UnsupportedRelayScheme(other.to_string())),
        };
        let host = url
            .host_str()
            .ok_or(RelayConnectError::MissingRelayHost)?
            .to_string();
        let port = url.port().unwrap_or(match scheme {
            RelayScheme::Ws => 80,
            RelayScheme::Wss => 443,
        });
        Ok(Self { host, port, scheme })
    }

    fn authority(&self) -> String {
        host_port(&self.host, self.port)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProxyKind {
    HttpConnect,
    Socks5,
}

struct ProxyEndpoint {
    kind: ProxyKind,
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
}

impl ProxyEndpoint {
    fn parse(value: &str) -> Result<Self, RelayConnectError> {
        let normalized = if value.contains("://") {
            Cow::Borrowed(value)
        } else {
            Cow::Owned(format!("http://{value}"))
        };
        let url = Url::parse(&normalized)
            .map_err(|error| RelayConnectError::ProxyConfig(error.to_string()))?;
        let kind = match url.scheme() {
            "http" => ProxyKind::HttpConnect,
            "socks5" | "socks5h" => ProxyKind::Socks5,
            scheme => {
                return Err(RelayConnectError::ProxyConfig(format!(
                    "unsupported proxy scheme {scheme}"
                )))
            }
        };
        let host = url
            .host_str()
            .ok_or_else(|| RelayConnectError::ProxyConfig("proxy URL has no host".into()))?
            .to_string();
        let port = url.port().unwrap_or(match kind {
            ProxyKind::HttpConnect => 8080,
            ProxyKind::Socks5 => 1080,
        });
        let username = (!url.username().is_empty()).then(|| url.username().to_string());
        let password = url.password().map(ToOwned::to_owned);
        Ok(Self {
            kind,
            host,
            port,
            username,
            password,
        })
    }

    fn system(kind: ProxyKind, host: String, port: u16) -> Self {
        Self {
            kind,
            host,
            port,
            username: None,
            password: None,
        }
    }

    fn address(&self) -> String {
        host_port(&self.host, self.port)
    }
}

pub(crate) async fn connect_websocket(
    url: &str,
) -> Result<(NativeWebSocket, Response), RelayConnectError> {
    let target = RelayTarget::parse(url)?;
    let proxy = resolve_proxy(&target)?;
    let stream = connect_transport(&target, proxy.as_ref()).await?;
    client_async_tls_with_config(url, stream, None, None)
        .await
        .map_err(|source| RelayConnectError::WebSocket {
            stage: if matches!(source, tungstenite::Error::Tls(_)) {
                "tls"
            } else {
                "websocket-handshake"
            },
            source,
        })
}

fn resolve_proxy(target: &RelayTarget) -> Result<Option<ProxyEndpoint>, RelayConnectError> {
    if should_bypass_proxy(&target.host) {
        return Ok(None);
    }

    if let Some(value) = environment_proxy(target.scheme) {
        return ProxyEndpoint::parse(&value).map(Some);
    }

    system_proxy(target)
}

fn environment_proxy(scheme: RelayScheme) -> Option<String> {
    let scheme_keys: &[&str] = match scheme {
        RelayScheme::Ws => &["HTTP_PROXY", "http_proxy"],
        RelayScheme::Wss => &["HTTPS_PROXY", "https_proxy"],
    };
    scheme_keys
        .iter()
        .chain(["ALL_PROXY", "all_proxy"].iter())
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
}

fn should_bypass_proxy(host: &str) -> bool {
    if is_local_host(host) {
        return true;
    }
    ["NO_PROXY", "no_proxy"]
        .iter()
        .filter_map(|key| env::var(key).ok())
        .any(|value| proxy_bypass_list_matches(host, &value))
}

fn is_local_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

fn proxy_bypass_list_matches(host: &str, list: &str) -> bool {
    list.split(',')
        .any(|entry| proxy_bypass_entry_matches(host, entry))
}

fn proxy_bypass_entry_matches(host: &str, raw_entry: &str) -> bool {
    let entry = raw_entry.trim();
    if entry == "*" {
        return true;
    }
    let domain = entry
        .split_once(':')
        .map_or(entry, |(without_port, _)| without_port)
        .trim_start_matches("*.")
        .trim_start_matches('.');
    !domain.is_empty()
        && (host.eq_ignore_ascii_case(domain)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", domain.to_ascii_lowercase())))
}

#[cfg(target_os = "macos")]
fn is_simple_hostname(host: &str) -> bool {
    !host.contains('.') && host.parse::<IpAddr>().is_err()
}

#[cfg(target_os = "macos")]
fn system_proxy(target: &RelayTarget) -> Result<Option<ProxyEndpoint>, RelayConnectError> {
    use std::io::Cursor;

    use plist::{Dictionary, Value};
    use system_configuration::{
        core_foundation::{
            base::{CFType, TCFType},
            dictionary::CFDictionary,
            number::CFNumber,
            propertylist::{create_data, kCFPropertyListBinaryFormat_v1_0},
            string::CFString,
        },
        dynamic_store::SCDynamicStoreBuilder,
    };

    fn number(proxies: &CFDictionary<CFString, CFType>, key: &str) -> Option<i64> {
        proxies
            .find(CFString::new(key))?
            .downcast::<CFNumber>()?
            .to_i64()
    }

    fn string(proxies: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
        proxies
            .find(CFString::new(key))?
            .downcast::<CFString>()
            .map(|value| value.to_string())
    }

    fn bypass_dictionary(
        proxies: &CFDictionary<CFString, CFType>,
    ) -> Result<Dictionary, RelayConnectError> {
        let data = create_data(
            proxies.as_CFTypeRef().cast(),
            kCFPropertyListBinaryFormat_v1_0,
        )
        .map_err(|error| RelayConnectError::ProxyConfig(error.to_string()))?;
        Value::from_reader(Cursor::new(data.bytes()))
            .map_err(|error| RelayConnectError::ProxyConfig(error.to_string()))?
            .into_dictionary()
            .ok_or_else(|| {
                RelayConnectError::ProxyConfig(
                    "macOS proxy settings are not a property-list dictionary".into(),
                )
            })
    }

    fn endpoint(
        proxies: &CFDictionary<CFString, CFType>,
        enabled_key: &str,
        host_key: &str,
        port_key: &str,
        kind: ProxyKind,
    ) -> Result<Option<ProxyEndpoint>, RelayConnectError> {
        if number(proxies, enabled_key) != Some(1) {
            return Ok(None);
        }
        let host = string(proxies, host_key).ok_or_else(|| {
            RelayConnectError::ProxyConfig(format!("{enabled_key} is set without {host_key}"))
        })?;
        let raw_port = number(proxies, port_key).ok_or_else(|| {
            RelayConnectError::ProxyConfig(format!("{enabled_key} is set without {port_key}"))
        })?;
        let port = u16::try_from(raw_port).map_err(|_| {
            RelayConnectError::ProxyConfig(format!("invalid {port_key} value {raw_port}"))
        })?;
        Ok(Some(ProxyEndpoint::system(kind, host, port)))
    }

    let Some(store) = SCDynamicStoreBuilder::new("buzz-relay-websocket").build() else {
        return Ok(None);
    };
    let Some(proxies) = store.get_proxies() else {
        return Ok(None);
    };
    if system_proxy_bypasses_host(&target.host, &bypass_dictionary(&proxies)?) {
        return Ok(None);
    }

    let http_proxy = match target.scheme {
        RelayScheme::Ws => endpoint(
            &proxies,
            "HTTPEnable",
            "HTTPProxy",
            "HTTPPort",
            ProxyKind::HttpConnect,
        )?,
        RelayScheme::Wss => endpoint(
            &proxies,
            "HTTPSEnable",
            "HTTPSProxy",
            "HTTPSPort",
            ProxyKind::HttpConnect,
        )?,
    };
    if http_proxy.is_some() {
        return Ok(http_proxy);
    }
    endpoint(
        &proxies,
        "SOCKSEnable",
        "SOCKSProxy",
        "SOCKSPort",
        ProxyKind::Socks5,
    )
}

#[cfg(target_os = "macos")]
fn system_proxy_bypasses_host(host: &str, proxies: &plist::Dictionary) -> bool {
    let exclude_simple_hostnames = proxies.get("ExcludeSimpleHostnames").is_some_and(|value| {
        value.as_boolean() == Some(true)
            || value.as_signed_integer() == Some(1)
            || value.as_unsigned_integer() == Some(1)
    });
    if exclude_simple_hostnames && is_simple_hostname(host) {
        return true;
    }

    proxies
        .get("ExceptionsList")
        .and_then(plist::Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .as_string()
                    .is_some_and(|pattern| proxy_bypass_entry_matches(host, pattern))
            })
        })
}

#[cfg(not(target_os = "macos"))]
fn system_proxy(_target: &RelayTarget) -> Result<Option<ProxyEndpoint>, RelayConnectError> {
    Ok(None)
}

async fn connect_transport(
    target: &RelayTarget,
    proxy: Option<&ProxyEndpoint>,
) -> Result<BoxedSocket, RelayConnectError> {
    match proxy {
        None => TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map(|stream| Box::new(stream) as BoxedSocket)
            .map_err(|source| RelayConnectError::Io {
                stage: "tcp-connect",
                endpoint: target.authority(),
                source,
            }),
        Some(proxy) if proxy.kind == ProxyKind::HttpConnect => {
            open_http_connect_tunnel(proxy, target)
                .await
                .map(|stream| Box::new(stream) as BoxedSocket)
        }
        Some(proxy) => open_socks5_tunnel(proxy, target).await,
    }
}

async fn open_http_connect_tunnel(
    proxy: &ProxyEndpoint,
    target: &RelayTarget,
) -> Result<TcpStream, RelayConnectError> {
    let proxy_address = proxy.address();
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|source| RelayConnectError::Io {
            stage: "proxy-connect",
            endpoint: proxy_address.clone(),
            source,
        })?;
    let authority = target.authority();
    let mut request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if let Some(username) = &proxy.username {
        let credentials = format!("{username}:{}", proxy.password.as_deref().unwrap_or(""));
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            BASE64_STANDARD.encode(credentials)
        ));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|source| RelayConnectError::Io {
            stage: "proxy-tunnel-write",
            endpoint: proxy_address.clone(),
            source,
        })?;

    let response = read_proxy_response_header(&mut stream, &proxy_address).await?;
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| RelayConnectError::ProxyHandshake {
            endpoint: proxy_address.clone(),
            detail: "empty HTTP CONNECT response".into(),
        })?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| RelayConnectError::ProxyHandshake {
            endpoint: proxy_address.clone(),
            detail: format!("invalid HTTP CONNECT status line {status_line:?}"),
        })?;
    if status != 200 {
        return Err(RelayConnectError::ProxyTunnelStatus {
            endpoint: proxy_address,
            status,
        });
    }
    Ok(stream)
}

async fn read_proxy_response_header(
    stream: &mut TcpStream,
    endpoint: &str,
) -> Result<String, RelayConnectError> {
    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|source| RelayConnectError::Io {
                stage: "proxy-tunnel-read",
                endpoint: endpoint.to_string(),
                source,
            })?;
        if read == 0 {
            return Err(RelayConnectError::ProxyHandshake {
                endpoint: endpoint.to_string(),
                detail: "proxy closed before completing HTTP CONNECT".into(),
            });
        }
        response.extend_from_slice(&chunk[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(response).map_err(|error| {
                RelayConnectError::ProxyHandshake {
                    endpoint: endpoint.to_string(),
                    detail: format!("HTTP CONNECT response was not UTF-8: {error}"),
                }
            });
        }
        if response.len() > MAX_PROXY_RESPONSE_HEADER_BYTES {
            return Err(RelayConnectError::ProxyHandshake {
                endpoint: endpoint.to_string(),
                detail: "HTTP CONNECT response headers exceeded 16 KiB".into(),
            });
        }
    }
}

async fn open_socks5_tunnel(
    proxy: &ProxyEndpoint,
    target: &RelayTarget,
) -> Result<BoxedSocket, RelayConnectError> {
    let proxy_address = proxy.address();
    let result = if let Some(username) = &proxy.username {
        Socks5Stream::connect_with_password(
            (proxy.host.as_str(), proxy.port),
            (target.host.as_str(), target.port),
            username,
            proxy.password.as_deref().unwrap_or(""),
        )
        .await
    } else {
        Socks5Stream::connect(
            (proxy.host.as_str(), proxy.port),
            (target.host.as_str(), target.port),
        )
        .await
    };
    result
        .map(|stream| Box::new(stream) as BoxedSocket)
        .map_err(|error| RelayConnectError::ProxyHandshake {
            endpoint: proxy_address,
            detail: error.to_string(),
        })
}

fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn no_proxy_domain_suffix_bypasses_subdomains() {
        assert!(proxy_bypass_list_matches(
            "relay.example.com",
            "localhost,.example.com"
        ));
    }

    #[test]
    fn proxy_bypass_domain_does_not_match_a_longer_suffix() {
        assert!(!proxy_bypass_entry_matches("notexample.com", "example.com"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_proxy_exceptions_bypass_matching_domains() {
        let mut proxies = plist::Dictionary::new();
        proxies.insert(
            "ExceptionsList".into(),
            plist::Value::Array(vec![plist::Value::String("*.example.com".into())]),
        );

        assert!(system_proxy_bypasses_host("relay.example.com", &proxies));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_proxy_excludes_simple_hostnames_without_excluding_ip_addresses() {
        let mut proxies = plist::Dictionary::new();
        proxies.insert(
            "ExcludeSimpleHostnames".into(),
            plist::Value::Integer(1.into()),
        );

        assert!(system_proxy_bypasses_host("relay", &proxies));
        assert!(!system_proxy_bypasses_host("192.0.2.1", &proxies));
    }

    #[test]
    fn proxy_debug_path_never_needs_credentials() {
        let proxy = ProxyEndpoint::parse("http://user:secret@127.0.0.1:6152").unwrap();

        assert_eq!(proxy.address(), "127.0.0.1:6152");
    }

    #[tokio::test]
    async fn http_connect_tunnel_sends_relay_authority() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let read = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(request[..read].to_vec()).unwrap()
        });
        let proxy = ProxyEndpoint::system(
            ProxyKind::HttpConnect,
            address.ip().to_string(),
            address.port(),
        );
        let target = RelayTarget::parse("wss://relay.example.com/socket").unwrap();

        let _stream = open_http_connect_tunnel(&proxy, &target).await.unwrap();
        let request = server.await.unwrap();

        assert!(request.starts_with("CONNECT relay.example.com:443 HTTP/1.1\r\n"));
    }
}
