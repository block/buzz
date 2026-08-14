use std::sync::RwLock;

struct VersionedClient {
    client: reqwest::Client,
    generation: u64,
}

/// Replaceable redirect-following client shared by relay API calls and other
/// app services.
pub(crate) struct AppHttpClient {
    current: RwLock<VersionedClient>,
}

impl AppHttpClient {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self {
            current: RwLock::new(VersionedClient {
                client,
                generation: 0,
            }),
        }
    }

    pub(crate) fn current(&self) -> reqwest::Client {
        match self.current.read() {
            Ok(current) => current.client.clone(),
            Err(poisoned) => poisoned.into_inner().client.clone(),
        }
    }

    pub(crate) fn versioned(&self) -> (reqwest::Client, u64) {
        match self.current.read() {
            Ok(current) => (current.client.clone(), current.generation),
            Err(poisoned) => {
                let current = poisoned.into_inner();
                (current.client.clone(), current.generation)
            }
        }
    }

    /// Install `replacement` only if another request has not already healed
    /// the client generation this caller observed.
    pub(crate) fn replace_if_generation(
        &self,
        expected_generation: u64,
        replacement: reqwest::Client,
    ) {
        let mut current = match self.current.write() {
            Ok(current) => current,
            Err(poisoned) => poisoned.into_inner(),
        };
        if current.generation == expected_generation {
            current.client = replacement;
            current.generation = current.generation.saturating_add(1);
        }
    }

    pub(crate) fn get<U: reqwest::IntoUrl>(&self, url: U) -> reqwest::RequestBuilder {
        self.current().get(url)
    }

    pub(crate) fn post<U: reqwest::IntoUrl>(&self, url: U) -> reqwest::RequestBuilder {
        self.current().post(url)
    }

    pub(crate) fn put<U: reqwest::IntoUrl>(&self, url: U) -> reqwest::RequestBuilder {
        self.current().put(url)
    }
}

/// Build the redirect-following HTTP client used by [`AppHttpClient`]. Keeping
/// this in one function lets relay recovery create a fresh connection pool
/// after a proxy or captive-portal interception.
pub(crate) fn build_app_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
