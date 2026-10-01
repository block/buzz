//! Resolve one stable identity, then reuse it on every configured relay.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nostr::{Keys, ToBech32};

use crate::config::{Secret, SecretRef, Spec};
use crate::env::Env;

pub struct Identity {
    pub secret: Secret,
    pub pubkey: String,
    pub owner: String,
    pub auth_tag: String,
    pub generated: Option<GeneratedKey>,
}

pub struct GeneratedKey {
    pub path: PathBuf,
    keys: Keys,
}

impl GeneratedKey {
    pub fn persist(&self) -> Result<()> {
        write_secret(&self.path, &self.keys)
    }
}

pub fn key_path(directory: &Path, name: &str) -> PathBuf {
    directory
        .join("keys")
        .join(format!("{}.nsec", name.to_ascii_lowercase()))
}

impl Identity {
    pub fn resolve(
        name: &str,
        spec: &Spec,
        directory: &Path,
        owner: Option<&Keys>,
        conditions: &str,
        env: &Env,
    ) -> Result<Self> {
        let path = key_path(directory, name);
        let (keys, generated_path) = match &spec.nsec {
            Some(source) => (
                Keys::parse(source.resolve(env)?.expose().trim())
                    .context("invalid agent `nsec`")?,
                None,
            ),
            None if path.try_exists()? => (
                Keys::parse(SecretRef::File(path.clone()).resolve(env)?.expose())
                    .with_context(|| format!("invalid agent key in {}", path.display()))?,
                None,
            ),
            None => (Keys::generate(), Some(path)),
        };
        let auth_tag = match &spec.auth_tag {
            Some(source) => source
                .resolve(env)
                .context("resolving `auth_tag`")?
                .expose()
                .to_owned(),
            None => buzz_sdk::nip_oa::compute_auth_tag(
                owner.context("set `owner.nsec` or provide this agent's `nsec` and `auth_tag`")?,
                &keys.public_key(),
                conditions,
            )
            .context("signing agent delegation")?,
        };
        let delegated_owner = buzz_sdk::nip_oa::verify_auth_tag_for_auth_event(
            &auth_tag,
            &keys.public_key(),
            nostr::Timestamp::now().as_secs(),
        )
        .context("`auth_tag` does not authorize this agent key")?;
        Ok(Self {
            secret: Secret::new(keys.secret_key().to_bech32()?),
            pubkey: keys.public_key().to_hex(),
            owner: delegated_owner.to_hex(),
            auth_tag,
            generated: generated_path.map(|path| GeneratedKey { path, keys }),
        })
    }
}

/// Create the key file with 0600 from the start — never world-readable for the
/// window between `create` and `chmod` — and refuse to clobber an existing key.
pub fn write_secret(path: &Path, keys: &Keys) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut handle = tempfile::NamedTempFile::new_in(parent)?;
    let nsec = keys
        .secret_key()
        .to_bech32()
        .context("encoding the new secret key")?;
    writeln!(handle, "{nsec}").with_context(|| format!("writing {}", path.display()))?;
    handle.as_file().sync_all()?;
    handle.persist_noclobber(path).with_context(|| {
        format!(
            "creating {} — refusing to overwrite an existing key",
            path.display()
        )
    })?;
    Ok(())
}
