/// Build the shared redirect-following HTTP client used by relay API calls and
/// other app services. Keeping this in one function lets relay recovery create
/// a fresh connection pool after a proxy or captive-portal interception.
pub(crate) fn build_app_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
