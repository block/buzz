//! Proxy-aware WebSocket connection establishment.

use std::error::Error as StdError;
use std::io;

use hyper_util::client::legacy::connect::proxy::{SocksV4, SocksV5, Tunnel};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::error::UrlError;
use tokio_tungstenite::tungstenite::handshake::client::{Request, Response};
use tokio_tungstenite::tungstenite::http::{uri::Authority, Uri};
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::{client_async_tls, MaybeTlsStream, WebSocketStream};
use tower_service::Service;

/// A WebSocket stream whose TCP connection may run through a configured proxy.
pub type ProxyWebSocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Connect to a WebSocket using environment and platform proxy settings.
///
/// `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY` are honored on all
/// platforms. Static system proxy settings are also discovered on macOS and
/// Windows. Loopback destinations always connect directly so a system proxy
/// cannot break Buzz's default local relay.
///
/// HTTP proxies use CONNECT tunneling. SOCKS4, SOCKS4a, SOCKS5, and SOCKS5h
/// proxy URLs are supported.
pub async fn connect_websocket<R>(request: R) -> Result<(ProxyWebSocketStream, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    let request = request.into_client_request()?;
    let matcher = Matcher::from_system();
    connect_websocket_with(&matcher, request).await
}

async fn connect_websocket_with(
    matcher: &Matcher,
    request: Request,
) -> Result<(ProxyWebSocketStream, Response), Error> {
    let target = proxy_target_uri(request.uri())?;
    let stream = connect_tcp(matcher, &target).await?;

    client_async_tls(request, stream).await
}

async fn connect_tcp(matcher: &Matcher, target: &Uri) -> Result<TcpStream, Error> {
    if target.host().is_some_and(is_loopback_host) {
        return connect_direct(target).await;
    }

    match matcher.intercept(target) {
        Some(proxy) => connect_via_proxy(target, proxy).await,
        None => connect_direct(target).await,
    }
}

async fn connect_direct(target: &Uri) -> Result<TcpStream, Error> {
    let mut connector = tcp_connector();
    connector
        .call(target.clone())
        .await
        .map(|stream| stream.into_inner())
        .map_err(|error| transport_error("direct TCP connection failed", error))
}

async fn connect_via_proxy(target: &Uri, proxy: Intercept) -> Result<TcpStream, Error> {
    let scheme = proxy.uri().scheme_str().unwrap_or("http");

    match scheme {
        "http" => connect_http_proxy(target, &proxy).await,
        "socks4" | "socks4a" => connect_socks4_proxy(target, &proxy, scheme).await,
        "socks5" | "socks5h" => connect_socks5_proxy(target, &proxy, scheme).await,
        unsupported => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unsupported WebSocket proxy scheme: {unsupported}"),
        )
        .into()),
    }
}

async fn connect_http_proxy(target: &Uri, proxy: &Intercept) -> Result<TcpStream, Error> {
    let proxy_uri = proxy_tcp_uri(proxy.uri(), 80)?;
    let mut connector = Tunnel::new(proxy_uri, tcp_connector());
    if let Some(auth) = proxy.basic_auth() {
        connector = connector.with_auth(auth.clone());
    }

    connector
        .call(target.clone())
        .await
        .map(|stream| stream.into_inner())
        .map_err(|error| transport_error("HTTP proxy tunnel failed", error))
}

async fn connect_socks4_proxy(
    target: &Uri,
    proxy: &Intercept,
    scheme: &str,
) -> Result<TcpStream, Error> {
    if proxy.raw_auth().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SOCKS4 proxy authentication is not supported",
        )
        .into());
    }

    let proxy_uri = proxy_tcp_uri(proxy.uri(), 1080)?;
    let mut connector = SocksV4::new(proxy_uri, tcp_connector()).local_dns(scheme == "socks4");

    connector
        .call(target.clone())
        .await
        .map(|stream| stream.into_inner())
        .map_err(|error| transport_error("SOCKS4 proxy tunnel failed", error))
}

async fn connect_socks5_proxy(
    target: &Uri,
    proxy: &Intercept,
    scheme: &str,
) -> Result<TcpStream, Error> {
    let proxy_uri = proxy_tcp_uri(proxy.uri(), 1080)?;
    let mut connector = SocksV5::new(proxy_uri, tcp_connector()).local_dns(scheme == "socks5");
    if let Some((username, password)) = proxy.raw_auth() {
        connector = connector.with_auth(username.to_string(), password.to_string());
    }

    connector
        .call(target.clone())
        .await
        .map(|stream| stream.into_inner())
        .map_err(|error| transport_error("SOCKS5 proxy tunnel failed", error))
}

fn tcp_connector() -> HttpConnector {
    let mut connector = HttpConnector::new();
    connector.enforce_http(false);
    connector
}

fn proxy_target_uri(websocket_uri: &Uri) -> Result<Uri, Error> {
    let (scheme, default_port) = match websocket_uri.scheme_str() {
        Some("ws") => ("http", 80),
        Some("wss") => ("https", 443),
        _ => return Err(UrlError::UnsupportedUrlScheme.into()),
    };
    let authority = authority_with_port(websocket_uri, default_port)?;
    let path_and_query = websocket_uri
        .path_and_query()
        .cloned()
        .ok_or(UrlError::NoPathOrQuery)?;

    Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(Error::from)
}

fn proxy_tcp_uri(proxy_uri: &Uri, default_port: u16) -> Result<Uri, Error> {
    let authority = authority_with_port(proxy_uri, default_port)?;
    Uri::builder()
        .scheme("http")
        .authority(authority)
        .path_and_query("/")
        .build()
        .map_err(Error::from)
}

fn authority_with_port(uri: &Uri, default_port: u16) -> Result<Authority, Error> {
    let host = uri.host().ok_or(UrlError::NoHostName)?;
    if host.is_empty() {
        return Err(UrlError::EmptyHostName.into());
    }

    if let Some(port) = uri.port_u16() {
        return format_authority(host, port);
    }

    format_authority(host, default_port)
}

fn format_authority(host: &str, port: u16) -> Result<Authority, Error> {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("{host}:{port}")
        .parse::<Authority>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error).into())
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    let host = host.strip_suffix('.').unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host.rsplit_once('.').is_some_and(|(prefix, suffix)| {
            !prefix.is_empty() && suffix.eq_ignore_ascii_case("localhost")
        })
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn transport_error(
    operation: &'static str,
    source: impl StdError + Send + Sync + 'static,
) -> Error {
    io::Error::other(TransportError {
        operation,
        source: Box::new(source),
    })
    .into()
}

#[derive(Debug)]
struct TransportError {
    operation: &'static str,
    source: Box<dyn StdError + Send + Sync>,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.source)
    }
}

impl StdError for TransportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn loopback_destinations_bypass_proxy() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("relay.localhost"));
        assert!(is_loopback_host("relay.LOCALHOST"));
        assert!(is_loopback_host("localhost."));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("relay.example.com"));
        assert!(!is_loopback_host("127.0.0.2.example.com"));
    }

    #[tokio::test]
    async fn loopback_destination_connects_directly() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target_listener.local_addr().unwrap();
        let target_task = tokio::spawn(async move {
            let (stream, _) = target_listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });

        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let matcher = Matcher::builder()
            .all(format!("http://{proxy_address}"))
            .build();
        let request = format!("ws://{target_address}")
            .into_client_request()
            .unwrap();

        let (_websocket, response) = connect_websocket_with(&matcher, request).await.unwrap();

        assert_eq!(response.status(), 101);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                proxy_listener.accept()
            )
            .await
            .is_err(),
            "loopback connection must not reach the proxy"
        );
        target_task.await.unwrap();
    }

    #[tokio::test]
    async fn connects_through_authenticated_http_proxy_without_target_dns() {
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();

        let proxy_task = tokio::spawn(async move {
            let (mut stream, _) = proxy_listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }

            let request = String::from_utf8(request).unwrap();
            assert!(request
                .starts_with("CONNECT relay.invalid:443 HTTP/1.1\r\nHost: relay.invalid:443\r\n"));
            assert!(request.contains("\r\nProxy-Authorization: Basic dXNlcjpwYXNzd29yZA==\r\n"));

            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();

            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            websocket
                .send(Message::Text("proxied".into()))
                .await
                .unwrap();
        });

        let matcher = Matcher::builder()
            .all(format!("http://user:password@{proxy_address}"))
            .build();
        let request = "ws://relay.invalid:443".into_client_request().unwrap();
        let (mut websocket, response) = connect_websocket_with(&matcher, request).await.unwrap();

        assert_eq!(response.status(), 101);
        assert_eq!(
            websocket.next().await.unwrap().unwrap(),
            Message::Text("proxied".into())
        );
        proxy_task.await.unwrap();
    }

    #[tokio::test]
    async fn proxy_connection_errors_do_not_expose_credentials() {
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let proxy_task = tokio::spawn(async move {
            let (_stream, _) = proxy_listener.accept().await.unwrap();
        });

        let matcher = Matcher::builder()
            .all(format!(
                "http://sensitive-user:sensitive-password@{proxy_address}"
            ))
            .build();
        let request = "ws://relay.invalid:443".into_client_request().unwrap();
        let error = connect_websocket_with(&matcher, request)
            .await
            .unwrap_err()
            .to_string();

        assert!(!error.contains("sensitive-user"));
        assert!(!error.contains("sensitive-password"));
        proxy_task.await.unwrap();
    }

    #[tokio::test]
    async fn connects_through_socks5h_proxy_without_target_dns() {
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();

        let proxy_task = tokio::spawn(async move {
            let (mut stream, _) = proxy_listener.accept().await.unwrap();

            let mut negotiation = [0_u8; 3];
            stream.read_exact(&mut negotiation).await.unwrap();
            assert_eq!(negotiation, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();

            let mut request_prefix = [0_u8; 5];
            stream.read_exact(&mut request_prefix).await.unwrap();
            assert_eq!(&request_prefix[..4], &[5, 1, 0, 3]);

            let mut target = vec![0_u8; usize::from(request_prefix[4]) + 2];
            stream.read_exact(&mut target).await.unwrap();
            let (host, port) = target.split_at(target.len() - 2);
            assert_eq!(host, b"relay.invalid");
            assert_eq!(u16::from_be_bytes([port[0], port[1]]), 443);

            stream
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();

            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            websocket
                .send(Message::Text("proxied".into()))
                .await
                .unwrap();
        });

        let matcher = Matcher::builder()
            .all(format!("socks5h://{proxy_address}"))
            .build();
        let request = "ws://relay.invalid:443".into_client_request().unwrap();
        let (mut websocket, response) = connect_websocket_with(&matcher, request).await.unwrap();

        assert_eq!(response.status(), 101);
        assert_eq!(
            websocket.next().await.unwrap().unwrap(),
            Message::Text("proxied".into())
        );
        proxy_task.await.unwrap();
    }
}
