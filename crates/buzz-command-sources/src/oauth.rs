use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use fs2::FileExt;
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const TOKEN_ENDPOINT: &str = "https://api.worldmonitor.app/oauth/token";
const MAX_CREDENTIAL_BYTES: u64 = 16 * 1024;
const REFRESH_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
const REFRESH_MARGIN_SECONDS: i64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum WorldMonitorOAuthError {
    #[error("World Monitor sign-in is required")]
    NotConnected,
    #[error("World Monitor sign-in has expired")]
    Reauthorise,
    #[error("World Monitor OAuth state is unavailable")]
    State,
    #[error("World Monitor OAuth service is unavailable")]
    Unavailable,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldMonitorOAuthCredentials {
    version: u8,
    client_id: String,
    access_token: String,
    refresh_token: String,
    access_expires_at: i64,
    refresh_expires_at: i64,
}

impl std::fmt::Debug for WorldMonitorOAuthCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorldMonitorOAuthCredentials")
            .field("version", &self.version)
            .field("client_id", &self.client_id)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("access_expires_at", &self.access_expires_at)
            .field("refresh_expires_at", &self.refresh_expires_at)
            .finish()
    }
}

impl WorldMonitorOAuthCredentials {
    pub fn from_exchange(
        client_id: String,
        access_token: String,
        refresh_token: String,
        expires_in: i64,
    ) -> Result<Self, WorldMonitorOAuthError> {
        validate_field(&client_id, 512)?;
        validate_field(&access_token, 2048)?;
        validate_field(&refresh_token, 2048)?;
        if expires_in <= 0 || expires_in > 24 * 60 * 60 {
            return Err(WorldMonitorOAuthError::State);
        }
        let now = now_epoch()?;
        Ok(Self {
            version: 1,
            client_id,
            access_token,
            refresh_token,
            access_expires_at: now.saturating_add(expires_in),
            refresh_expires_at: now.saturating_add(REFRESH_TTL_SECONDS),
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn refresh_expires_at(&self) -> i64 {
        self.refresh_expires_at
    }
}

#[derive(Clone, Debug)]
pub struct WorldMonitorOAuthStore {
    path: PathBuf,
    token_endpoint: Url,
}

impl WorldMonitorOAuthStore {
    pub fn new(path: PathBuf) -> Result<Self, WorldMonitorOAuthError> {
        let token_endpoint =
            Url::parse(TOKEN_ENDPOINT).map_err(|_| WorldMonitorOAuthError::State)?;
        Ok(Self {
            path,
            token_endpoint,
        })
    }

    #[cfg(test)]
    fn with_token_endpoint(
        path: PathBuf,
        token_endpoint: Url,
    ) -> Result<Self, WorldMonitorOAuthError> {
        Ok(Self {
            path,
            token_endpoint,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<WorldMonitorOAuthCredentials>, WorldMonitorOAuthError> {
        read_credentials(&self.path)
    }

    pub fn save(
        &self,
        credentials: &WorldMonitorOAuthCredentials,
    ) -> Result<(), WorldMonitorOAuthError> {
        let lock = lock_file(&self.path)?;
        lock.lock_exclusive()
            .map_err(|_| WorldMonitorOAuthError::State)?;
        let result = write_credentials(&self.path, credentials);
        let _ = FileExt::unlock(&lock);
        result
    }

    pub fn clear(&self) -> Result<(), WorldMonitorOAuthError> {
        let lock = lock_file(&self.path)?;
        lock.lock_exclusive()
            .map_err(|_| WorldMonitorOAuthError::State)?;
        let result = match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(WorldMonitorOAuthError::State),
        };
        let _ = FileExt::unlock(&lock);
        result
    }

    pub async fn bearer_token(&self) -> Result<Zeroizing<String>, WorldMonitorOAuthError> {
        let lock = lock_file(&self.path)?;
        lock.lock_exclusive()
            .map_err(|_| WorldMonitorOAuthError::State)?;
        let result = self.bearer_token_locked().await;
        let _ = FileExt::unlock(&lock);
        result
    }

    async fn bearer_token_locked(&self) -> Result<Zeroizing<String>, WorldMonitorOAuthError> {
        let Some(mut credentials) = read_credentials(&self.path)? else {
            return Err(WorldMonitorOAuthError::NotConnected);
        };
        let now = now_epoch()?;
        if credentials.access_expires_at > now.saturating_add(REFRESH_MARGIN_SECONDS) {
            return Ok(Zeroizing::new(credentials.access_token));
        }
        if credentials.refresh_expires_at <= now {
            return Err(WorldMonitorOAuthError::Reauthorise);
        }

        #[derive(Deserialize)]
        struct RefreshResponse {
            access_token: String,
            refresh_token: String,
            expires_in: i64,
            token_type: String,
        }

        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|_| WorldMonitorOAuthError::Unavailable)?;
        let response = client
            .post(self.token_endpoint.clone())
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", credentials.refresh_token.as_str()),
                ("client_id", credentials.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|_| WorldMonitorOAuthError::Unavailable)?;
        if response.status() == reqwest::StatusCode::BAD_REQUEST
            || response.status() == reqwest::StatusCode::UNAUTHORIZED
            || response.status() == reqwest::StatusCode::FORBIDDEN
        {
            return Err(WorldMonitorOAuthError::Reauthorise);
        }
        if !response.status().is_success() {
            return Err(WorldMonitorOAuthError::Unavailable);
        }
        let refreshed: RefreshResponse = response
            .json()
            .await
            .map_err(|_| WorldMonitorOAuthError::Unavailable)?;
        if !refreshed.token_type.eq_ignore_ascii_case("bearer") {
            return Err(WorldMonitorOAuthError::State);
        }
        credentials = WorldMonitorOAuthCredentials::from_exchange(
            credentials.client_id,
            refreshed.access_token,
            refreshed.refresh_token,
            refreshed.expires_in,
        )?;
        write_credentials(&self.path, &credentials)?;
        Ok(Zeroizing::new(credentials.access_token))
    }
}

fn validate_field(value: &str, maximum: usize) -> Result<(), WorldMonitorOAuthError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(WorldMonitorOAuthError::State);
    }
    Ok(())
}

fn now_epoch() -> Result<i64, WorldMonitorOAuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .map_err(|_| WorldMonitorOAuthError::State)
}

fn lock_file(path: &Path) -> Result<File, WorldMonitorOAuthError> {
    let parent = path.parent().ok_or(WorldMonitorOAuthError::State)?;
    std::fs::create_dir_all(parent).map_err(|_| WorldMonitorOAuthError::State)?;
    let lock_path = path.with_extension("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|_| WorldMonitorOAuthError::State)?;
    restrict_file(&file)?;
    Ok(file)
}

fn read_credentials(
    path: &Path,
) -> Result<Option<WorldMonitorOAuthCredentials>, WorldMonitorOAuthError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(WorldMonitorOAuthError::State),
    };
    if file
        .metadata()
        .map_err(|_| WorldMonitorOAuthError::State)?
        .len()
        > MAX_CREDENTIAL_BYTES
    {
        return Err(WorldMonitorOAuthError::State);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| WorldMonitorOAuthError::State)?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(WorldMonitorOAuthError::State);
    }
    let credentials: WorldMonitorOAuthCredentials =
        serde_json::from_slice(&bytes).map_err(|_| WorldMonitorOAuthError::State)?;
    if credentials.version != 1 {
        return Err(WorldMonitorOAuthError::State);
    }
    validate_field(&credentials.client_id, 512)?;
    validate_field(&credentials.access_token, 2048)?;
    validate_field(&credentials.refresh_token, 2048)?;
    Ok(Some(credentials))
}

fn write_credentials(
    path: &Path,
    credentials: &WorldMonitorOAuthCredentials,
) -> Result<(), WorldMonitorOAuthError> {
    let payload = serde_json::to_vec(credentials).map_err(|_| WorldMonitorOAuthError::State)?;
    if payload.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(WorldMonitorOAuthError::State);
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| WorldMonitorOAuthError::State)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| WorldMonitorOAuthError::State)?;
    }
    file.write_all(&payload)
        .map_err(|_| WorldMonitorOAuthError::State)?;
    file.commit().map_err(|_| WorldMonitorOAuthError::State)
}

fn restrict_file(file: &File) -> Result<(), WorldMonitorOAuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| WorldMonitorOAuthError::State)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::{
        extract::State, http::StatusCode, response::IntoResponse, routing::post, Form, Router,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn refresh_rotates_and_persists_credentials() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let server_seen = Arc::clone(&seen);
        let app = Router::new()
            .route(
                "/token",
                post(
                    |State(seen): State<Arc<Mutex<Vec<String>>>>,
                     Form(form): Form<std::collections::HashMap<String, String>>| async move {
                        seen.lock()
                            .expect("lock")
                            .push(form.get("refresh_token").cloned().unwrap_or_default());
                        (
                            StatusCode::OK,
                            axum::Json(json!({
                                "access_token": "new-access",
                                "refresh_token": "new-refresh",
                                "expires_in": 3600,
                                "token_type": "Bearer"
                            })),
                        )
                            .into_response()
                    },
                ),
            )
            .with_state(server_seen);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let directory = tempdir().expect("directory");
        let path = directory.path().join("oauth.json");
        let store = WorldMonitorOAuthStore::with_token_endpoint(
            path.clone(),
            Url::parse(&format!("http://{address}/token")).expect("url"),
        )
        .expect("store");
        let mut credentials = WorldMonitorOAuthCredentials::from_exchange(
            "client".to_string(),
            "old-access".to_string(),
            "old-refresh".to_string(),
            3600,
        )
        .expect("credentials");
        credentials.access_expires_at = 0;
        store.save(&credentials).expect("save");

        assert_eq!(
            store.bearer_token().await.expect("token").as_str(),
            "new-access"
        );
        assert_eq!(
            seen.lock().expect("seen").as_slice(),
            ["old-refresh".to_string()]
        );
        let persisted = store.load().expect("load").expect("credentials");
        assert_eq!(persisted.refresh_token, "new-refresh");
        assert!(!std::fs::read_to_string(path)
            .expect("read")
            .contains("old-refresh"));
    }

    #[test]
    fn debug_redacts_tokens_and_file_is_restricted() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("oauth.json");
        let store = WorldMonitorOAuthStore::new(path.clone()).expect("store");
        let credentials = WorldMonitorOAuthCredentials::from_exchange(
            "client".to_string(),
            "secret-access".to_string(),
            "secret-refresh".to_string(),
            3600,
        )
        .expect("credentials");
        assert!(!format!("{credentials:?}").contains("secret-access"));
        store.save(&credentials).expect("save");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
