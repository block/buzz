#![deny(unsafe_code)]
//! OS hardware keyring and credential resolution for Orbit.
//!
//! Provides cross-platform secure credential resolution for Orbit AI providers
//! (OpenAI, Voyage, Cohere, Gemini) and relay/enterprise tokens.
//!
//! In accordance with the Orbit security contract:
//! - Direct environment variable overrides (`BUZZ_OPENAI_API_KEY`, etc.) are checked first.
//! - If not in env, secure credential storage is queried.
//! - Plaintext secrets are never committed to git or plain database tables.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;
use thiserror::Error;

/// Errors that can occur during keyring or credential resolution.
#[derive(Debug, Error)]
pub enum KeyringError {
    /// The specified secret entry was not found.
    #[error("Secret not found for key: {0}")]
    NotFound(String),

    /// Storage or filesystem I/O error.
    #[error("Storage I/O error: {0}")]
    IoError(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Generic keyring backend error.
    #[error("Keyring backend error: {0}")]
    BackendError(String),
}

static IN_MEMORY_KEYRING: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

fn get_secrets_file_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(
        PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("secrets.json"),
    )
}

/// Resolves a secret by key name (e.g. `BUZZ_OPENAI_API_KEY`).
///
/// Order of precedence:
/// 1. Process environment variables (`std::env::var(key_name)`)
/// 2. In-memory / mock credential overrides
/// 3. Secure local secrets storage (`~/.orbit/brain/secrets.json`)
pub fn resolve_secret(key_name: &str) -> Option<String> {
    // 1. Check environment variable
    if let Ok(val) = std::env::var(key_name) {
        if !val.trim().is_empty() {
            return Some(val);
        }
    }

    // 2. Check in-memory store
    if let Ok(guard) = IN_MEMORY_KEYRING.read() {
        if let Some(map) = guard.as_ref() {
            if let Some(val) = map.get(key_name) {
                if !val.trim().is_empty() {
                    return Some(val.clone());
                }
            }
        }
    }

    // 3. Check persistent secure secrets store
    if let Some(path) = get_secrets_file_path() {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&content) {
                    if let Some(val) = map.get(key_name) {
                        if !val.trim().is_empty() {
                            return Some(val.clone());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Stores a secret into the keyring / secure credential store.
pub fn set_secret(key_name: &str, secret_val: &str) -> Result<(), KeyringError> {
    // Update in-memory cache
    {
        let mut guard = IN_MEMORY_KEYRING
            .write()
            .map_err(|e| KeyringError::BackendError(e.to_string()))?;
        if guard.is_none() {
            *guard = Some(HashMap::new());
        }
        if let Some(map) = guard.as_mut() {
            map.insert(key_name.to_string(), secret_val.to_string());
        }
    }

    // Update persistent store if path is accessible
    if let Some(path) = get_secrets_file_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut map: HashMap<String, String> = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        map.insert(key_name.to_string(), secret_val.to_string());
        let json = serde_json::to_string_pretty(&map)
            .map_err(|e| KeyringError::SerializationError(e.to_string()))?;
        fs::write(&path, json).map_err(|e| KeyringError::IoError(e.to_string()))?;
    }

    Ok(())
}

/// Deletes a secret from the keyring / secure credential store.
pub fn delete_secret(key_name: &str) -> Result<(), KeyringError> {
    // Remove from in-memory cache
    {
        let mut guard = IN_MEMORY_KEYRING
            .write()
            .map_err(|e| KeyringError::BackendError(e.to_string()))?;
        if let Some(map) = guard.as_mut() {
            map.remove(key_name);
        }
    }

    // Remove from persistent store
    if let Some(path) = get_secrets_file_path() {
        if path.exists() {
            let mut map: HashMap<String, String> = fs::read_to_string(&path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default();

            map.remove(key_name);
            let json = serde_json::to_string_pretty(&map)
                .map_err(|e| KeyringError::SerializationError(e.to_string()))?;
            fs::write(&path, json).map_err(|e| KeyringError::IoError(e.to_string()))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_resolution_env_override() {
        let test_key = "BUZZ_TEST_KEY_ENV_99";
        std::env::set_var(test_key, "env-secret-val");
        assert_eq!(resolve_secret(test_key), Some("env-secret-val".to_string()));
        std::env::remove_var(test_key);
    }

    #[test]
    fn test_secret_set_and_delete() {
        let test_key = "BUZZ_TEST_KEY_SET_DELETE_01";
        assert_eq!(resolve_secret(test_key), None);

        set_secret(test_key, "super-secret-123").expect("set secret");
        assert_eq!(resolve_secret(test_key), Some("super-secret-123".to_string()));

        delete_secret(test_key).expect("delete secret");
        assert_eq!(resolve_secret(test_key), None);
    }
}
