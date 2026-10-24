//! Process-isolated secret storage for desktop nsec private keys.
//!
//! Buzz never calls a platform keyring. Every operation uses the public
//! `daz-secrets` client, which starts the provider configured in the current
//! OS account's owner-only `~/.config/daz-secrets/provider.toml`. Darren's
//! machine selects a private encrypted provider; other installations may
//! select any conforming provider, including an optional OS-keyring adapter.
//! Secret bytes travel only through anonymous child-process pipes.

use daz_secrets::{BlockingClient, ErrorCode, Metadata, Secret};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Result of probing the configured provider before a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringProbe {
    /// The provider contains the requested item.
    Present,
    /// The provider is reachable but the requested item is absent.
    ReachableButEmpty,
    /// The provider is unavailable or returned an unverifiable result.
    Unreachable,
}

/// Secret storage namespace backed by the configured daz-secrets provider.
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    /// Construct a provider-backed store.
    ///
    /// The method name is retained for source compatibility with older Buzz
    /// code; it does not access an operating-system keyring.
    pub fn keyring(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Return the process-global store for the configured service.
    pub fn shared(service: &'static str) -> &'static Self {
        static INSTANCE: OnceLock<SecretStore> = OnceLock::new();
        INSTANCE.get_or_init(|| Self::keyring(service))
    }

    /// Probe whether `key` is present without displaying authentication UI.
    pub fn probe(&self, key: &str) -> KeyringProbe {
        match provider_get(self.service.clone(), key.to_string()) {
            Ok(_) => KeyringProbe::Present,
            Err(ProviderFailure::NotFound) => KeyringProbe::ReachableButEmpty,
            Err(ProviderFailure::Other) => KeyringProbe::Unreachable,
        }
    }

    /// Load a UTF-8 secret. Missing items return `Ok(None)`.
    pub fn load(&self, key: &str) -> Result<Option<String>, String> {
        match provider_get(self.service.clone(), key.to_string()) {
            Ok(secret) => String::from_utf8(secret.value)
                .map(Some)
                .map_err(|_| "secret provider returned non-UTF-8 data".to_string()),
            Err(ProviderFailure::NotFound) => Ok(None),
            Err(ProviderFailure::Other) => Err("secret provider unavailable".to_string()),
        }
    }

    /// Load every UTF-8 secret in this store's namespace.
    pub fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        let rows = provider_list(self.service.clone())
            .map_err(|_| "secret provider unavailable".to_string())?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut result = HashMap::with_capacity(rows.len());
        for (account, value) in rows {
            let value = String::from_utf8(value)
                .map_err(|_| "secret provider returned non-UTF-8 data".to_string())?;
            result.insert(account, value);
        }
        Ok(Some(result))
    }

    /// Store every supplied entry in this store's namespace.
    pub fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        provider_set_all(self.service.clone(), entries.clone())
            .map_err(|_| "secret provider unavailable".to_string())
    }

    /// Verify a value by reading it back from the provider.
    pub fn verify_stored_raw(&self, key: &str, expected: &str) -> Result<bool, String> {
        match provider_get(self.service.clone(), key.to_string()) {
            Ok(secret) => Ok(secret.value == expected.as_bytes()),
            Err(ProviderFailure::NotFound) => Ok(false),
            Err(ProviderFailure::Other) => Err("secret provider unavailable".to_string()),
        }
    }

    /// Store a UTF-8 value in the provider.
    pub fn store(&self, key: &str, value: &str) -> Result<(), String> {
        provider_set(
            self.service.clone(),
            key.to_string(),
            value.as_bytes().to_vec(),
        )
        .map_err(|_| "secret provider unavailable".to_string())
    }

    /// Delete every provider item in this store's namespace.
    ///
    /// Legacy Keychain cleanup is deliberately excluded: reading or deleting
    /// those entries from a rebuilt app could itself prompt. The standalone
    /// one-time migration tool performs exact legacy cleanup after verifying
    /// that the provider holds identical bytes.
    pub fn delete_all_with_legacy_cleanup(&self) -> Result<(), String> {
        provider_delete_all(self.service.clone())
            .map_err(|_| "secret provider unavailable".to_string())
    }

    /// Verify that this namespace contains no provider items.
    pub fn verify_fully_wiped(&self) -> bool {
        provider_metadata()
            .map(|items| items.iter().all(|item| item.service != self.service))
            .unwrap_or(false)
    }

    /// Delete one item. A missing item is not an error.
    pub fn delete(&self, key: &str) -> Result<(), String> {
        provider_delete(self.service.clone(), key.to_string())
            .map_err(|_| "secret provider unavailable".to_string())
    }
}

#[derive(Clone, Copy)]
enum ProviderFailure {
    NotFound,
    Other,
}

fn classify_error(error: daz_secrets::Error) -> ProviderFailure {
    if error.code() == ErrorCode::NotFound {
        ProviderFailure::NotFound
    } else {
        ProviderFailure::Other
    }
}

fn provider_get(service: String, account: String) -> Result<Secret, ProviderFailure> {
    BlockingClient::from_default_config()
        .map_err(classify_error)?
        .get(&service, &account)
        .map_err(classify_error)
}

fn provider_set(service: String, account: String, value: Vec<u8>) -> Result<(), ProviderFailure> {
    BlockingClient::from_default_config()
        .map_err(classify_error)?
        .set(&service, &account, &value, None)
        .map(|_| ())
        .map_err(classify_error)
}

fn provider_metadata() -> Result<Vec<Metadata>, ProviderFailure> {
    BlockingClient::from_default_config()
        .map_err(classify_error)?
        .list_metadata()
        .map_err(classify_error)
}

fn provider_list(service: String) -> Result<Vec<(String, Vec<u8>)>, ProviderFailure> {
    let client = BlockingClient::from_default_config().map_err(classify_error)?;
    let metadata = client.list_metadata().map_err(classify_error)?;
    let mut rows = Vec::new();
    for item in metadata.into_iter().filter(|item| item.service == service) {
        let secret = client
            .get(&item.service, &item.account)
            .map_err(classify_error)?;
        rows.push((item.account, secret.value));
    }
    Ok(rows)
}

fn provider_set_all(
    service: String,
    entries: HashMap<String, String>,
) -> Result<(), ProviderFailure> {
    let client = BlockingClient::from_default_config().map_err(classify_error)?;
    for (account, value) in entries {
        client
            .set(&service, &account, value.as_bytes(), None)
            .map_err(classify_error)?;
    }
    Ok(())
}

fn provider_delete(service: String, account: String) -> Result<(), ProviderFailure> {
    let client = BlockingClient::from_default_config().map_err(classify_error)?;
    match client.delete(&service, &account, None) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(()),
        Err(error) => Err(classify_error(error)),
    }
}

fn provider_delete_all(service: String) -> Result<(), ProviderFailure> {
    let client = BlockingClient::from_default_config().map_err(classify_error)?;
    let metadata = client.list_metadata().map_err(classify_error)?;
    for item in metadata.into_iter().filter(|item| item.service == service) {
        match client.delete(&item.service, &item.account, None) {
            Ok(()) => {}
            Err(error) if error.code() == ErrorCode::NotFound => {}
            Err(error) => return Err(classify_error(error)),
        }
    }
    Ok(())
}
