//! Dedicated no-redirect HTTP client for admin API requests.
//!
//! A separate client (not the app-wide `http_client`) ensures that:
//! - 3xx responses are surfaced as errors rather than followed — preventing
//!   redirect-hop SSRF where a relay-issued redirect could forward the NIP-98
//!   `Authorization` header to an off-origin host.
//! - Timeouts are tuned for synchronous UI feedback rather than media downloads.

use std::sync::OnceLock;

use super::dns::LocalhostDnsResolver;

/// Request timeout for admin API calls.
pub(crate) const ADMIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The module-level singleton admin HTTP client.
///
/// Built once via `OnceLock` — panics on build failure so there is no
/// silent fallback to a redirect-following client.
pub static ADMIN_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Initialise the admin client singleton. Must be called from `setup()` before
/// any admin command can be invoked. Subsequent calls are no-ops.
pub fn init_admin_client() {
    ADMIN_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            // Pin bare `localhost` to loopback (exact-hostname override).
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            // Pin `.localhost` subdomain names (e.g. `admin.localhost`) to loopback.
            // RFC 6761 §6.3 requires this but system getaddrinfo is unreliable on
            // Linux/Windows CI runners; the custom resolver makes it deterministic
            // across all supported platforms without changing non-localhost resolution.
            .dns_resolver(LocalhostDnsResolver)
            .pool_idle_timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(ADMIN_TIMEOUT)
            .build()
            .expect(
                "admin HTTP client must build with redirect::Policy::none(); \
                 a redirect-following fallback would forward the NIP-98 \
                 Authorization header across origins (redirect-hop SSRF)",
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The admin client must be buildable and must refuse to follow redirects.
    /// This mirrors the `build_media_fetch_client_succeeds_with_no_redirect_policy`
    /// test in `media_download.rs`.
    #[test]
    fn admin_client_builds_with_no_redirect_policy() {
        init_admin_client();
        assert!(ADMIN_CLIENT.get().is_some());
    }

    /// A live test that the client does not follow a 302.
    ///
    /// Mirrors `media_fetch_client_does_not_follow_redirects` in
    /// `media_download.rs`. Serves a 302 pointing at the metadata endpoint
    /// and asserts exactly one connection was accepted.
    #[tokio::test]
    async fn admin_client_does_not_follow_redirects() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        init_admin_client();
        let client = ADMIN_CLIENT.get().expect("client initialised");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));

        let server_connections = Arc::clone(&connections);
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                server_connections.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = "HTTP/1.1 302 Found\r\n\
                     Location: http://169.254.169.254/latest/meta-data/\r\n\
                     Content-Length: 0\r\n\
                     Connection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let resp = client
            .get(format!("http://{addr}/api/admin/v1/reports"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .expect("request should complete without following the redirect");

        assert_eq!(resp.status().as_u16(), 302);
        server.join().unwrap();
        assert_eq!(
            connections.load(Ordering::SeqCst),
            1,
            "exactly one request must be issued — redirect must not be followed",
        );
    }
}
