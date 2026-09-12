//! Fixed-port native PKCE callback listener. Bind before launching the browser.
use std::{net::Ipv4Addr, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use crate::enterprise_oauth::{EnterpriseLoginAttempt, EnterpriseLoginConfig};

/// One bounded callback receiver for the exact release-registered redirect URI.
/// Dropping it closes the port. It never selects an ephemeral/fallback port.
pub struct EnterpriseCallback {
    listener: TcpListener,
    redirect_uri: String,
    authority: String,
}

impl EnterpriseCallback {
    /// Bind the configured port on 127.0.0.1, failing closed when it is occupied.
    pub async fn bind(config: &EnterpriseLoginConfig) -> Result<Self, String> {
        config.validate()?;
        let port = config.loopback_port()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| "Configured corporate login callback port is unavailable")?;
        Ok(Self {
            listener,
            redirect_uri: config.redirect_uri.clone(),
            authority: format!("127.0.0.1:{port}"),
        })
    }

    /// Receive a state-checked callback, with a five-minute overall deadline,
    /// at most 32 connections, 8 KiB headers, and three-second per-peer IO limits.
    /// No code/verifier/token is reflected in the browser response.
    pub async fn receive(self, attempt: &EnterpriseLoginAttempt) -> Result<url::Url, String> {
        timeout(Duration::from_secs(300), async {
            for _ in 0..32 {
                let (mut stream, _) = self.listener.accept().await.map_err(|_| "Login callback failed")?;
                let request = timeout(Duration::from_secs(3), read_headers(&mut stream)).await;
                let callback = match request {
                    Ok(Ok(request)) => self.parse_request(&request, attempt),
                    _ => Err("Invalid login callback".into()),
                };
                let (status, body) = if callback.is_ok() {
                    ("200 OK", "Return to Buzz to finish signing in.")
                } else {
                    ("400 Bad Request", "Invalid login callback.")
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
                // A disconnected browser must not discard an already valid callback.
                let _ = timeout(Duration::from_secs(3), async {
                    stream.write_all(response.as_bytes()).await?;
                    stream.shutdown().await
                }).await;
                if let Ok(callback) = callback { return Ok(callback); }
            }
            Err("Too many invalid login callbacks".into())
        }).await.map_err(|_| "Corporate login timed out")?
    }

    fn parse_request(
        &self,
        request: &str,
        attempt: &EnterpriseLoginAttempt,
    ) -> Result<url::Url, String> {
        let mut lines = request.split("\r\n");
        let target = lines
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|line| line.strip_suffix(" HTTP/1.1"))
            .ok_or("Invalid login callback method")?;
        // Validate BEFORE URL parsing can normalize dot segments or backslashes.
        if !(target == "/enterprise-callback" || target.starts_with("/enterprise-callback?"))
            || target.contains(['\\', '#'])
            || !target.is_ascii()
        {
            return Err("Invalid login callback path".into());
        }
        let hosts: Vec<_> = lines
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim())
            .collect();
        if hosts != [self.authority.as_str()] {
            return Err("Invalid login callback authority".into());
        }
        let query = target
            .strip_prefix("/enterprise-callback")
            .ok_or("Invalid callback")?;
        let callback = url::Url::parse(&format!("{}{query}", self.redirect_uri))
            .map_err(|_| "Invalid callback URL")?;
        attempt.callback_code(&callback)?;
        Ok(callback)
    }
}

async fn read_headers(stream: &mut TcpStream) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "Callback read failed")?;
        if count == 0 || bytes.len() + count > 8192 {
            return Err("Invalid callback size".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(bytes).map_err(|_| "Invalid callback encoding".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(port: u16) -> EnterpriseLoginConfig {
        EnterpriseLoginConfig {
            signer_url: "https://signer.example/cash-app/goose/".into(),
            issuer: "https://login.example/".into(),
            client_id: "native".into(),
            audience: "signer".into(),
            organization: "org".into(),
            connection: "corporate".into(),
            redirect_uri: format!("http://127.0.0.1:{port}/enterprise-callback"),
        }
    }

    #[tokio::test]
    async fn occupied_configured_port_fails_without_fallback() {
        let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        assert!(
            EnterpriseCallback::bind(&config(occupied.local_addr().unwrap().port()))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn actual_listener_ignores_bad_callbacks_then_closes_successful_response() {
        let provisional = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = provisional.local_addr().unwrap().port();
        drop(provisional);
        let config = config(port);
        let callback = EnterpriseCallback::bind(&config).await.unwrap();
        assert_eq!(callback.listener.local_addr().unwrap().port(), port);
        let attempt = EnterpriseLoginAttempt::new([1; 32], [2; 32], config.redirect_uri.clone());
        let authorize = attempt.authorization_url(&config).unwrap();
        let state = authorize
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let task = tokio::spawn(async move { callback.receive(&attempt).await });
        for (path, host, expected) in [
            (
                "/favicon.ico".to_string(),
                format!("127.0.0.1:{port}"),
                "400",
            ),
            (
                format!("/enterprise-callback?state={state}&code=secret"),
                "attacker.example".into(),
                "400",
            ),
            (
                "/enterprise-callback?state=wrong&code=secret".into(),
                format!("127.0.0.1:{port}"),
                "400",
            ),
            (
                format!("/enterprise-callback?state={state}&code=secret"),
                format!("127.0.0.1:{port}"),
                "200",
            ),
        ] {
            let mut peer = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .unwrap();
            peer.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut response = String::new();
            timeout(Duration::from_secs(2), peer.read_to_string(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.starts_with(&format!("HTTP/1.1 {expected}")));
            assert!(!response.contains("secret"));
        }
        assert_eq!(
            task.await
                .unwrap()
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "code")
                .unwrap()
                .1,
            "secret"
        );
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn request_parser_rejects_authority_path_and_normalization_confusion() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = config(port);
        let callback = EnterpriseCallback {
            listener,
            redirect_uri: config.redirect_uri.clone(),
            authority: format!("127.0.0.1:{port}"),
        };
        let attempt = EnterpriseLoginAttempt::new([1; 32], [2; 32], config.redirect_uri.clone());
        let auth = attempt.authorization_url(&config).unwrap();
        let state = auth
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned();
        let good = format!("GET /enterprise-callback?code=x&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(callback.parse_request(&good, &attempt).is_ok());
        for bad in [
            good.replace("GET ", "POST "),
            good.replace("/enterprise-callback?", "/a/../enterprise-callback?"),
            good.replace("/enterprise-callback?", "//enterprise-callback?"),
            good.replace("/enterprise-callback?", "/%65nterprise-callback?"),
            good.replace("/enterprise-callback?", "/enterprise-callback\\?"),
            good.replace("Host:", "Not-Host:"),
            good.replace("Host:", &format!("Host: 127.0.0.1:{port}\r\nHost:")),
            good.replace("127.0.0.1:", "user@127.0.0.1:"),
            good.replace(" HTTP/1.1", "#fragment HTTP/1.1"),
            good.replace("code=x", "code=x&code=y"),
        ] {
            assert!(
                callback.parse_request(&bad, &attempt).is_err(),
                "accepted {bad}"
            );
        }
    }
}
