//! Private one-request Beehive native bridge. stdin/stdout are owned pipes, never
//! a terminal protocol. No environment discovery or Desktop credential access.
use buzz_agent::auth::{AuthIntent, BrowserOpener, OAuthTokenCustody, PkceOAuthTokenSource};
use buzz_agent::{config::Config, types::AgentError};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{IsTerminal, Read, Write},
    path::PathBuf,
    sync::Arc,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    action: String,
    host: String,
    account: String,
    coordination: PathBuf,
}
fn failure() -> AgentError {
    AgentError::Llm("Beehive OAuth credential operation failed".into())
}
struct Custody {
    host: String,
    account: String,
}
impl Custody {
    fn decode(&self, bytes: &[u8]) -> Result<Option<String>, AgentError> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| failure())?;
        if value["host"].as_str() != Some(&self.host) {
            return Err(failure());
        }
        Ok(Some(
            value["token"].as_str().ok_or_else(failure)?.to_owned(),
        ))
    }
    fn store_verified(
        &self,
        secret: &str,
        write: impl FnOnce(&[u8]) -> Result<(), AgentError>,
        read: impl FnOnce() -> Result<Option<String>, AgentError>,
    ) -> Result<(), AgentError> {
        let bytes = serde_json::to_vec(&json!({"host": self.host, "token": secret}))
            .map_err(|_| failure())?;
        write(&bytes)?;
        if read()?.as_deref() != Some(secret) {
            return Err(failure());
        }
        Ok(())
    }
}
impl OAuthTokenCustody for Custody {
    fn load(&self) -> Result<Option<String>, AgentError> {
        #[cfg(target_os = "macos")]
        {
            let bytes =
                match security_framework::passwords::get_generic_password("beehive", &self.account)
                {
                    Ok(v) => v,
                    Err(e) if e.code() == -25300 => return Ok(None),
                    Err(_) => return Err(failure()),
                };
            self.decode(&bytes)
        }
        #[cfg(not(target_os = "macos"))]
        Err(failure())
    }
    fn store(&self, secret: &str) -> Result<(), AgentError> {
        #[cfg(target_os = "macos")]
        {
            self.store_verified(
                secret,
                |bytes| {
                    security_framework::passwords::set_generic_password(
                        "beehive",
                        &self.account,
                        bytes,
                    )
                    .map_err(|_| failure())
                },
                || self.load(),
            )
        }
        #[cfg(not(target_os = "macos"))]
        Err(failure())
    }
}
struct Browser;
impl BrowserOpener for Browser {
    fn open(&self, url: &str) -> Result<(), String> {
        webbrowser::open(url).map_err(|_| "Browser unavailable".into())
    }
}
async fn execute(request: Request) -> Result<serde_json::Value, AgentError> {
    let url = url::Url::parse(&request.host).map_err(|_| failure())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.port().is_some()
        || url.host_str().is_none()
        || url.host_str() == Some("localhost")
        || url::Host::parse(url.host_str().ok_or_else(failure)?)
            .map_err(|_| failure())?
            .to_string()
            .parse::<std::net::IpAddr>()
            .is_ok()
    {
        return Err(failure());
    }
    let suffix = request
        .account
        .strip_prefix("provider:")
        .ok_or_else(failure)?;
    if suffix.len() != 36
        || !suffix.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        || !request.coordination.is_absolute()
        || !["login", "models", "token"].contains(&request.action.as_str())
    {
        return Err(failure());
    }
    // The parent owns this mode-0700 directory. This exact-account outer lock
    // spans read, grant rotation, read-back and response across helper processes.
    let metadata = std::fs::symlink_metadata(&request.coordination).map_err(|_| failure())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
            return Err(failure());
        }
    }
    let lock_path = request
        .coordination
        .parent()
        .ok_or_else(failure)?
        .join(format!("{}.lock", request.account));
    if let Ok(meta) = std::fs::symlink_metadata(&lock_path) {
        if !meta.is_file() {
            return Err(failure());
        }
    }
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| failure())?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|_| failure())?;
    let host = url.origin().ascii_serialization();
    let custody: Arc<dyn OAuthTokenCustody> = Arc::new(Custody {
        host: host.clone(),
        account: request.account,
    });
    let source = PkceOAuthTokenSource::new_with_custody(
        buzz_agent::databricks_oauth_config(&host, request.coordination),
        Arc::new(Browser),
        custody,
    )?;
    let intent = if request.action == "login" {
        AuthIntent::UserInitiated
    } else {
        AuthIntent::Headless
    };
    let token = source
        .acquire_with_intent(intent, None)
        .await
        .map_err(|_| failure())?;
    match request.action.as_str() {
        "login" => Ok(json!({"ok":true})),
        "token" => Ok(json!({"ok":true,"secret":token})),
        _ => {
            // Parent launches with a closed environment; these are explicit
            // request values, not passive provider environment discovery.
            std::env::set_var("BUZZ_AGENT_PROVIDER", "databricks_v2");
            std::env::set_var("DATABRICKS_HOST", &host);
            std::env::set_var("BUZZ_AGENT_MODEL", "catalog-discovery-only");
            let cfg = Config::from_env().map_err(|_| failure())?;
            let models =
                buzz_agent::catalog::discover_databricks_models_with_token_source(&cfg, source)
                    .await?;
            Ok(json!({"ok":true,"models":models.iter().map(|m| &m.id).collect::<Vec<_>>()}))
        }
    }
}
fn main() {
    if std::io::stdin().is_terminal() || std::io::stdout().is_terminal() {
        std::process::exit(2);
    }
    let result = (|| -> Result<_, AgentError> {
        let mut input = Vec::new();
        std::io::stdin()
            .take(8193)
            .read_to_end(&mut input)
            .map_err(|_| failure())?;
        if input.len() > 8192 {
            return Err(failure());
        }
        let request = serde_json::from_slice(&input).map_err(|_| failure())?;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| failure())?
            .block_on(execute(request))
    })();
    let value = result.unwrap_or_else(|_| json!({"ok":false}));
    let _ = std::io::stdout().write_all(value.to_string().as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_custody_requires_exact_readback_and_workspace() {
        let custody = Custody {
            host: "https://fixture.example".into(),
            account: "provider:00000000-0000-0000-0000-000000000000".into(),
        };
        let recorded = std::cell::RefCell::new(Vec::new());
        custody
            .store_verified(
                "fixture-secret",
                |bytes| {
                    *recorded.borrow_mut() = bytes.to_vec();
                    Ok(())
                },
                || custody.decode(&recorded.borrow()),
            )
            .unwrap();
        assert!(custody
            .store_verified("fixture-secret", |_| Ok(()), || Ok(None))
            .is_err());
        assert!(custody
            .store_verified("fixture-secret", |_| Ok(()), || Ok(Some("wrong".into())))
            .is_err());
        assert!(custody
            .store_verified(
                "fixture-secret",
                |_| Err(failure()),
                || panic!("read after failed write")
            )
            .is_err());
        assert!(custody
            .store_verified("fixture-secret", |_| Ok(()), || Err(failure()))
            .is_err());
        let other = Custody {
            host: "https://other.example".into(),
            account: custody.account.clone(),
        };
        assert!(other.decode(&recorded.borrow()).is_err());
    }
}
