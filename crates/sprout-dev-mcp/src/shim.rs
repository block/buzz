use nostr::ToBech32;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zeroize::Zeroize;

/// Session-scoped shim directory providing tools and git config to shell children.
///
/// On install:
/// 1. Creates a 0700 tempdir with symlinks back to our binary (multicall)
/// 2. If `NOSTR_PRIVATE_KEY` is set: writes a 0600 keyfile, derives the pubkey,
///    builds ephemeral `GIT_CONFIG_*` env vars, then removes the env var
/// 3. Prepends the shim dir to PATH
///
/// Shell children receive `path_env`, `git_env`, and `SPROUT_PRIVATE_KEY` (for
/// the sprout CLI). `NOSTR_PRIVATE_KEY` is removed from the process env after
/// the keyfile is written — git helpers read from the keyfile only.
/// Cleaned up on drop (TempDir).
pub struct Shim {
    _dir: TempDir,
    pub path_env: String,
    pub git_env: Vec<(String, String)>,
}

impl Shim {
    pub fn install() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("sprout-dev-mcp-")
            .tempdir()?;
        set_owner_only(dir.path())?;

        let self_exe = std::env::current_exe()?;

        // Multicall symlinks — all resolve back to this binary.
        for name in [
            "rg",
            "tree",
            "sprout",
            "git-credential-nostr",
            "git-sign-nostr",
        ] {
            symlink(&self_exe, &dir.path().join(name))?;
        }

        let original = std::env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::from(dir.path());
        if !original.is_empty() {
            new_path.push(":");
            new_path.push(&original);
        }
        let path_env = new_path.to_string_lossy().into_owned();

        // Read and unconditionally remove NOSTR_PRIVATE_KEY from this process's
        // env. The key must never leak to child processes regardless of whether
        // keyfile creation succeeds.
        let mut nostr_key = std::env::var("NOSTR_PRIVATE_KEY").ok();
        std::env::remove_var("NOSTR_PRIVATE_KEY");

        // Ephemeral git config: write key to 0600 keyfile, derive pubkey, build
        // GIT_CONFIG_* env vars for nostr auth + signing.
        let git_env = match nostr_key
            .as_deref()
            .and_then(|k| write_keyfile(dir.path(), k))
        {
            Some(info) => build_git_env(&info),
            None => Vec::new(),
        };
        if let Some(ref mut k) = nostr_key {
            k.zeroize();
        }

        Ok(Self {
            _dir: dir,
            path_env,
            git_env,
        })
    }
}

struct KeyInfo {
    keyfile_path: String,
    pubkey_hex: String,
    npub: String,
}

/// Write the nostr private key to an owner-only file in the shim dir.
/// Returns key metadata or None if key is empty/invalid.
/// Warns to stderr if the key is invalid (operator mistake).
fn write_keyfile(shim_dir: &Path, raw: &str) -> Option<KeyInfo> {
    if raw.is_empty() {
        return None;
    }
    let keys = match nostr::Keys::parse(raw) {
        Ok(k) => k,
        Err(e) => {
            eprintln!(
                "sprout-dev-mcp: warning: NOSTR_PRIVATE_KEY is set but invalid ({e}); \
                 git auth/signing will be disabled"
            );
            return None;
        }
    };
    let pubkey_hex = keys.public_key().to_hex();
    let npub = keys
        .public_key()
        .to_bech32()
        .unwrap_or_else(|_| pubkey_hex.clone());

    let keyfile = shim_dir.join(".nostr-key");
    if write_keyfile_atomic(&keyfile, raw.as_bytes()).is_err() {
        eprintln!(
            "sprout-dev-mcp: warning: failed to write nostr keyfile; git auth/signing disabled"
        );
        return None;
    }
    let keyfile_path = match keyfile.to_str() {
        Some(s) => s.to_owned(),
        None => {
            eprintln!("sprout-dev-mcp: warning: tempdir path is not valid UTF-8; git auth/signing disabled");
            return None;
        }
    };

    Some(KeyInfo {
        keyfile_path,
        pubkey_hex,
        npub,
    })
}

/// Write `data` to `path` with 0600 permissions set at creation time via
/// `OpenOptions::mode()` (no window where the file is world-readable).
/// Non-Unix: plain write — acceptable inside our 0700 tempdir.
#[cfg(unix)]
fn write_keyfile_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_keyfile_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Derive a NIP-05-style email from the pubkey and relay URL.
/// Format: `<hex_pubkey>@<relay_host>` (e.g., `ab12...cd@relay.sprout.dev`).
/// Falls back to `<hex_pubkey>@sprout` if no relay URL is configured.
fn derive_git_email(pubkey_hex: &str) -> String {
    let host = std::env::var("SPROUT_RELAY_URL")
        .ok()
        .and_then(|url| {
            // Strip scheme, port, and trailing paths
            let stripped = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .or_else(|| url.strip_prefix("wss://"))
                .or_else(|| url.strip_prefix("ws://"))
                .unwrap_or(&url);
            let host_port = stripped.split('/').next()?;
            // Strip port number (e.g., "localhost:3000" → "localhost")
            Some(host_port.split(':').next().unwrap_or(host_port).to_owned())
        })
        .filter(|h| !h.is_empty() && !h.starts_with("localhost") && !h.starts_with("127."))
        .unwrap_or_else(|| "sprout".to_owned());
    format!("{pubkey_hex}@{host}")
}

/// Build GIT_CONFIG_COUNT/KEY/VALUE env vars for ephemeral nostr git config.
/// Composes with any existing GIT_CONFIG_COUNT in the environment. When launched
/// via sprout-agent (which clears env), the base is always 0 — composition only
/// matters when dev-mcp is run directly with pre-existing GIT_CONFIG vars.
fn build_git_env(info: &KeyInfo) -> Vec<(String, String)> {
    let email = derive_git_email(&info.pubkey_hex);
    let entries: Vec<(&str, String)> = vec![
        // Identity — npub as display name, NIP-05-style email
        ("user.name", info.npub.clone()),
        ("user.email", email),
        // Nostr credential helper is additive — it silently declines non-Sprout
        // remotes (exits 0, no credential), so git falls through to system
        // helpers (osxkeychain, store, etc.) for GitHub/GitLab/etc.
        ("credential.helper", "nostr".into()),
        // Required: Sprout relay verifies NIP-98 against the full repo-root URL.
        // Without useHttpPath, git only passes the host and auth is rejected.
        ("credential.useHttpPath", "true".into()),
        ("nostr.keyfile", info.keyfile_path.clone()),
        ("gpg.format", "x509".into()),
        ("gpg.x509.program", "git-sign-nostr".into()),
        ("commit.gpgSign", "true".into()),
        ("tag.gpgSign", "true".into()),
        ("user.signingkey", info.pubkey_hex.clone()),
    ];

    // Compose with existing GIT_CONFIG_COUNT — don't clobber caller's config.
    let base: usize = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut env = Vec::with_capacity(entries.len() * 2 + 1);
    env.push((
        "GIT_CONFIG_COUNT".into(),
        (base + entries.len()).to_string(),
    ));
    for (i, (key, val)) in entries.iter().enumerate() {
        let idx = base + i;
        env.push((format!("GIT_CONFIG_KEY_{idx}"), key.to_string()));
        env.push((format!("GIT_CONFIG_VALUE_{idx}"), val.to_string()));
    }
    env
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_owner_only(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(not(unix))]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst).map(|_| ())
}

pub fn artifact_dir(session_root: &Path) -> PathBuf {
    let p = session_root.join("artifacts");
    let _ = std::fs::create_dir_all(&p);
    p
}
