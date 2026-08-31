//! Tracer-only native keychain pinned to a fixture file. Never changes the user's
//! default keychain/search list, and never substitutes an in-memory secret store.
use keyring::{
    credential::{Credential, CredentialApi, CredentialBuilderApi},
    macos::decode_error,
};
use security_framework::os::macos::{
    keychain::{CreateOptions, SecKeychain},
    passwords::find_generic_password,
};
use std::{
    any::Any,
    path::{Path, PathBuf},
};

pub(super) fn install(home: &Path) -> Result<(), String> {
    let path = home.join("tracer.keychain");
    // Persist only inside the restricted fixture, never in source or evidence.
    let password_path = home.join("keychain-password");
    let password = if path.exists() {
        std::fs::read_to_string(&password_path).map_err(|_| "fixture keychain password missing")?
    } else {
        let password = nostr::Keys::generate().secret_key().to_secret_hex();
        crate::managed_agents::atomic_write_json_restricted(&password_path, password.as_bytes())
            .map_err(|_| "fixture keychain password write failed")?;
        password
    };
    let mut chain = if path.exists() {
        SecKeychain::open(&path)
    } else {
        CreateOptions::new().password(&password).create(&path)
    }
    .map_err(|_| "fixture keychain create/open failed")?;
    chain
        .unlock(Some(&password))
        .map_err(|_| "fixture keychain unlock failed")?;
    keyring::set_default_credential_builder(Box::new(Builder(path)));
    Ok(())
}
struct Builder(PathBuf);
impl CredentialBuilderApi for Builder {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(Entry {
            path: self.0.clone(),
            service: service.into(),
            user: user.into(),
        }))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct Entry {
    path: PathBuf,
    service: String,
    user: String,
}
impl Entry {
    fn chain(&self) -> keyring::Result<SecKeychain> {
        SecKeychain::open(&self.path).map_err(decode_error)
    }
}
impl CredentialApi for Entry {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        self.chain()?
            .set_generic_password(&self.service, &self.user, secret)
            .map_err(decode_error)
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        let (bytes, _) = find_generic_password(Some(&[self.chain()?]), &self.service, &self.user)
            .map_err(decode_error)?;
        Ok(bytes.to_owned())
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        let (_, item) = find_generic_password(Some(&[self.chain()?]), &self.service, &self.user)
            .map_err(decode_error)?;
        item.delete();
        Ok(())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
