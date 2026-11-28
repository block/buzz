//! Generic, bounded HTTP transport. Credentials and server bodies never enter errors.

use std::time::Duration;

use anyhow::{ensure, Context, Result};
use reqwest::{Client, Url};
use serde_json::Value;

const MAX_BYTES: usize = 64 * 1024;

pub(in crate::user) struct IdentityGateConfig {
    block_url: Url,
    login_url: Url,
    account_url: Url,

    client: Client,
}

impl IdentityGateConfig {
    pub(in crate::user) fn new(
        block_url: &str,
        login_url: &str,
        account_url: &str,

        timeout: Duration,
    ) -> Result<Self> {
        ensure!(
            timeout >= Duration::from_secs(1) && timeout <= Duration::from_secs(120),
            "identity HTTP timeout must be between 1 and 120 seconds"
        );
        let block_url = endpoint(block_url, "/v1/identity/block-user-auth-login")?;
        let login_url = endpoint(login_url, "/v1/identity/delete-user-auth-login")?;
        let account_url = endpoint(account_url, "/v1/identity/delete-builderlab-account")?;
        let directory = block_url.join(".")?;
        ensure!(
            login_url.join(".")? == directory && account_url.join(".")? == directory,
            "identity endpoints must use the same origin and route prefix"
        );
        let builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout);
        Ok(Self {
            block_url,
            login_url,
            account_url,
            client: builder
                .build()
                .map_err(|_| anyhow::anyhow!("could not build identity HTTP client"))?,
        })
    }

    pub(in crate::user) fn from_env() -> Result<Option<Self>> {
        let read = |name| -> Result<Option<String>> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => anyhow::bail!("invalid identity gate configuration encoding"),
            }
        };
        let block = read("BUZZ_USER_DELETION_IDENTITY_BLOCK_URL")?;
        let login = read("BUZZ_USER_DELETION_IDENTITY_DELETE_LOGIN_URL")?;
        let account = read("BUZZ_USER_DELETION_IDENTITY_DELETE_ACCOUNT_URL")?;
        ensure!(
            std::env::var_os("BUZZ_USER_DELETION_IDENTITY_TOKEN_FILE").is_none(),
            "remove the legacy identity token file setting after configuring workload authentication"
        );
        let timeout = read("BUZZ_USER_DELETION_IDENTITY_TIMEOUT_SECS")?;
        if block.is_none() && login.is_none() && account.is_none() && timeout.is_none() {
            return Ok(None);
        }
        let timeout = timeout
            .as_deref()
            .unwrap_or("30")
            .parse::<u64>()
            .map_err(|_| anyhow::anyhow!("invalid identity HTTP timeout"))?;
        Ok(Some(Self::new(
            &block.context("missing identity block URL")?,
            &login.context("missing identity delete login URL")?,
            &account.context("missing identity delete account URL")?,
            Duration::from_secs(timeout),
        )?))
    }

    pub(super) fn prepare_block(&self, body: &Value) -> Result<reqwest::Request> {
        self.prepare(&self.block_url, body)
    }

    pub(in crate::user) fn prepare_delete_login(&self, body: &Value) -> Result<reqwest::Request> {
        self.prepare(&self.login_url, body)
    }

    pub(in crate::user) fn prepare_delete_account(&self, body: &Value) -> Result<reqwest::Request> {
        self.prepare(&self.account_url, body)
    }

    fn prepare(&self, endpoint: &Url, body: &Value) -> Result<reqwest::Request> {
        let body = serde_json::to_vec(body)?;
        ensure!(
            body.len() <= MAX_BYTES,
            "identity request exceeds size limit"
        );
        self.client
            .post(endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .build()
            .context("could not prepare identity request")
    }

    pub(in crate::user) async fn send(&self, request: reqwest::Request) -> Result<Value> {
        let mut response = self
            .client
            .execute(request)
            .await
            .map_err(|_| anyhow::anyhow!("identity request failed; outcome unconfirmed"))?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "identity service returned HTTP {}",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("identity response interrupted; outcome unconfirmed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= MAX_BYTES,
                "identity response exceeds size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid identity JSON response"))
    }
}

fn endpoint(value: &str, path: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow::anyhow!("invalid identity endpoint URL"))?;
    // The mesh intercepts a direct connection to the Kubernetes Service and
    // supplies mutual TLS. Numeric loopback also supports a local sidecar/test.
    let internal_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            let service = host
                .strip_suffix(".svc.cluster.local")
                .and_then(|name| name.split_once('.'));
            service.is_some_and(|(service, namespace)| {
                !service.is_empty() && !namespace.is_empty() && !namespace.contains('.')
            }) || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
    ensure!(
        (url.scheme() == "https" || internal_http)
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "identity endpoint requires HTTPS or HTTP to a cluster Service or numeric loopback address without credentials, query or fragment"
    );
    ensure!(
        url.path().ends_with(path),
        "identity endpoint requires route suffix {path}"
    );
    Ok(url)
}
