//! Generic, bounded HTTP transport. Credentials and server bodies never enter errors.

use std::{path::PathBuf, time::Duration};

use anyhow::{ensure, Context, Result};
use reqwest::{header::HeaderValue, Client, Url};
use serde_json::Value;
use tokio::io::AsyncReadExt;

const MAX_BYTES: usize = 64 * 1024;
const MAX_TOKEN_BYTES: u64 = 16 * 1024;

pub(in crate::user) struct IdentityGateConfig {
    block_url: Url,
    login_url: Url,
    account_url: Url,
    token_file: PathBuf,
    client: Client,
}

impl IdentityGateConfig {
    pub(in crate::user) fn new(
        block_url: &str,
        login_url: &str,
        account_url: &str,
        token_file: PathBuf,
        timeout: Duration,
    ) -> Result<Self> {
        ensure!(
            timeout >= Duration::from_secs(1) && timeout <= Duration::from_secs(120),
            "identity HTTP timeout must be between 1 and 120 seconds"
        );
        ensure!(
            !token_file.as_os_str().is_empty(),
            "identity token file is empty"
        );
        let block_url = endpoint(block_url, "/api/goose/v1/identity/block-user-auth-login")?;
        let login_url = endpoint(login_url, "/api/goose/v1/identity/delete-user-auth-login")?;
        let account_url = endpoint(
            account_url,
            "/api/goose/v1/identity/delete-builderlab-account",
        )?;
        ensure!(
            block_url.origin() == account_url.origin()
                && block_url != account_url
                && login_url.origin() == block_url.origin()
                && login_url != account_url
                && login_url != block_url,
            "identity endpoints must have distinct paths on the same origin"
        );
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(timeout);
        // Plain HTTP is only allowed to loopback; never send its bearer token
        // through an ambient proxy outside that machine.
        if block_url.scheme() == "http" {
            builder = builder.no_proxy();
        }
        Ok(Self {
            block_url,
            login_url,
            account_url,
            token_file,
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
        let token = read("BUZZ_USER_DELETION_IDENTITY_TOKEN_FILE")?;
        let timeout = read("BUZZ_USER_DELETION_IDENTITY_TIMEOUT_SECS")?;
        if block.is_none()
            && login.is_none()
            && account.is_none()
            && token.is_none()
            && timeout.is_none()
        {
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
            PathBuf::from(token.context("missing identity token file")?),
            Duration::from_secs(timeout),
        )?))
    }

    pub(super) async fn prepare_block(&self, body: &Value) -> Result<reqwest::Request> {
        self.prepare(&self.block_url, body).await
    }

    pub(in crate::user) async fn prepare_delete_login(
        &self,
        body: &Value,
    ) -> Result<reqwest::Request> {
        self.prepare(&self.login_url, body).await
    }

    pub(in crate::user) async fn prepare_delete_account(
        &self,
        body: &Value,
    ) -> Result<reqwest::Request> {
        self.prepare(&self.account_url, body).await
    }

    async fn prepare(&self, endpoint: &Url, body: &Value) -> Result<reqwest::Request> {
        let body = serde_json::to_vec(body)?;
        ensure!(
            body.len() <= MAX_BYTES,
            "identity request exceeds size limit"
        );
        let file = tokio::fs::File::open(&self.token_file)
            .await
            .map_err(|_| anyhow::anyhow!("could not open identity token file"))?;
        let mut bytes = Vec::new();
        file.take(MAX_TOKEN_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| anyhow::anyhow!("could not read identity token file"))?;
        ensure!(
            bytes.len() as u64 <= MAX_TOKEN_BYTES,
            "identity token exceeds size limit"
        );
        let token = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid identity bearer token"))?
            .trim();
        ensure!(
            !token.is_empty() && !token.chars().any(char::is_whitespace),
            "invalid identity bearer token"
        );
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| anyhow::anyhow!("invalid identity bearer token"))?;
        auth.set_sensitive(true);
        self.client
            .post(endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, auth)
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
    let local_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
    ensure!(
        (url.scheme() == "https" || local_http)
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "identity endpoint requires HTTPS (or a loopback HTTP address) without credentials, query or fragment"
    );
    ensure!(url.path() == path, "identity endpoint requires path {path}");
    Ok(url)
}
