//! HTTP clients with separate decompression contracts for relay JSON and media.

pub fn build() -> reqwest::Result<reqwest::Client> {
    base().build()
}

/// Build the media client without redirects or transparent decoding.
///
/// Media callers forward upstream Content-Length/Content-Range. Decoding would
/// change the body while stripping or invalidating those video-seeking headers.
pub fn build_media() -> reqwest::Result<reqwest::Client> {
    base()
        .no_gzip()
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn base() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
}

#[cfg(test)]
mod tests {
    use axum::{http::HeaderMap, response::Response, routing::get, Router};

    const GZIP_BODY: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 43, 72, 76, 207, 204, 75, 44, 201, 204, 207, 83, 40, 72,
        172, 204, 201, 79, 76, 1, 0, 132, 249, 73, 160, 18, 0, 0, 0,
    ];

    async fn serve(app: Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (address, server)
    }

    #[tokio::test]
    async fn app_client_transparently_decodes_gzip() {
        let app = Router::new().route(
            "/",
            get(|headers: HeaderMap| async move {
                assert!(headers
                    .get("accept-encoding")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.split(',').any(|coding| coding.trim() == "gzip")));
                Response::builder()
                    .header("content-encoding", "gzip")
                    .header("content-length", GZIP_BODY.len())
                    .body(axum::body::Body::from(GZIP_BODY))
                    .unwrap()
            }),
        );
        let (address, server) = serve(app).await;
        let response = super::build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "pagination payload");
        server.abort();
    }

    #[tokio::test]
    async fn media_client_does_not_advertise_gzip() {
        let app = Router::new().route(
            "/",
            get(|headers: HeaderMap| async move {
                assert!(headers.get("accept-encoding").is_none());
                "range bytes"
            }),
        );
        let (address, server) = serve(app).await;
        let response = super::build_media()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "range bytes");
        server.abort();
    }
}
