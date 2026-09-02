use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Session-scoped shim directory providing tools and git config to shell children.
///
/// On install:
/// 1. Creates a 0700 tempdir with symlinks back to our binary (multicall)
/// 2. If `NOSTR_PRIVATE_KEY` is set: writes a 0600 keyfile, derives the pubkey,
///    builds ephemeral `GIT_CONFIG_*` env vars, then removes the env var
/// 3. Prepends the shim dir to PATH
///
/// Shell children receive `path_env`, `git_env`, and `BUZZ_PRIVATE_KEY` (for
/// the buzz CLI). `NOSTR_PRIVATE_KEY` is removed from the process env after
/// the keyfile is written — git helpers read from the keyfile only.
/// Cleaned up on drop (TempDir).
#[derive(Debug)]
pub struct Shim {
    _dir: TempDir,
    pub path_env: String,
    pub git_env: Vec<(String, String)>,
}

impl Shim {
    pub fn install() -> std::io::Result<Self> {
        // Read the operator's identity mode once, here at install. A bad value
        // fails the session loudly rather than silently picking a mode — silent
        // fallback in an identity control is the failure class this exists to
        // close. The enforcement wrapper never reads this var (it would let an
        // agent disable enforcement mid-session); the mode is fixed at install.
        let mode = buzz_git_identity::GitIdentityMode::from_env()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let enforce = mode == buzz_git_identity::GitIdentityMode::Agent;

        let dir = tempfile::Builder::new().prefix("buzz-dev-mcp-").tempdir()?;
        set_owner_only(dir.path())?;

        let self_exe = std::env::current_exe()?;

        // Multicall symlinks. `rg`/`tree`/`buzz` and the `git-credential-nostr`
        // helper are present in both modes: relay git-over-HTTP auth is
        // orthogonal to commit attribution (auth ≠ attribution). The `git`
        // enforcement wrapper (see git_wrapper.rs) and the `git-sign-nostr`
        // signer are installed only once a valid identity + manifest exists
        // (the `Some(id) if enforce` arm below) — never merely because the mode
        // is `agent`. The wrapper reads the manifest as its authority and fails
        // closed when it is absent, so installing it on a keyless/invalid/
        // unwritable session (where no manifest is written) would shadow `git`
        // with a wrapper that refuses EVERY command — including plain
        // `git status`/`clone` — instead of the documented keyless passthrough.
        // (Fail-closed for a manifest removed AFTER a managed install still
        // holds: that leaves the wrapper on PATH with no manifest, which the
        // wrapper itself refuses.) In `user` mode the agent likewise uses
        // vanilla git resolving the operator's own identity, so no wrapper.
        let mut names = vec!["rg", "tree", "buzz", "git-credential-nostr"];

        // Ephemeral git config: NOSTR_PRIVATE_KEY → 0600 keyfile (and removed
        // from this process's env so children never see it) → derive identity →
        // build the GIT_CONFIG_* env. The identity primitives live in
        // `buzz-git-identity`, shared with the harness so an agent commits under
        // the same identity regardless of which surface applied it.
        //
        // `agent` mode installs authorship + NIP-GS signing + the credential
        // helper, and writes the authoritative identity manifest the wrapper
        // reads. `user` mode installs only the credential helper (plus the
        // `nostr.keyfile` pointer it loads the key from, since NOSTR_PRIVATE_KEY
        // was just scrubbed) — no authorship, no signing, no manifest, no
        // wrapper — so commits carry the operator's own identity. A missing/
        // invalid/unwritable key (`None`) installs neither wrapper nor config:
        // plain passthrough git in either mode.
        let git_env = match buzz_git_identity::take_key_and_write(dir.path()) {
            Some(id) if enforce => {
                let identity = buzz_git_identity::identity_signing_entries(&id);
                // Write the authoritative identity manifest the enforcement
                // wrapper reads (its expected author + the config it re-applies
                // before exec). Without it the wrapper cannot fail closed on an
                // env-scrubbed identity, so a manifest-write failure disables
                // enforcement — treat it as fatal rather than ship a wrapper
                // that silently trusts mutable env. Only after the manifest
                // exists do we install the wrapper + signer, so the wrapper
                // always has an authority to enforce against.
                buzz_git_identity::write_identity_manifest(dir.path(), &identity)?;
                names.push("git");
                names.push("git-sign-nostr");
                let mut entries = identity;
                entries.extend(buzz_git_identity::nostr_credential_entries());
                buzz_git_identity::to_git_config_env(&entries)
            }
            Some(id) => {
                let mut entries = buzz_git_identity::nostr_credential_entries();
                entries.push(buzz_git_identity::keyfile_entry(&id));
                buzz_git_identity::to_git_config_env(&entries)
            }
            None => Vec::new(),
        };

        for name in names {
            symlink(&self_exe, &dir.path().join(name))?;
        }

        let original = std::env::var_os("PATH").unwrap_or_default();
        let mut entries = vec![PathBuf::from(dir.path())];
        entries.extend(std::env::split_paths(&original));
        // join_paths uses the platform separator (':' on Unix, ';' on Windows).
        let path_env = std::env::join_paths(entries)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
            .to_string_lossy()
            .into_owned();

        Ok(Self {
            _dir: dir,
            path_env,
            git_env,
        })
    }
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
    // No symlinks without elevation on Windows; copy instead. The target needs
    // a .exe extension or PATH lookup (via PATHEXT) won't treat it as runnable.
    let dst = dst.with_extension("exe");
    std::fs::copy(src, dst).map(|_| ())
}

pub fn artifact_dir(session_root: &Path) -> PathBuf {
    let p = session_root.join("artifacts");
    let _ = std::fs::create_dir_all(&p);
    p
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use nostr::ToBech32;
    use std::sync::Mutex;

    /// `Shim::install` reads process-global env (BUZZ_GIT_IDENTITY,
    /// NOSTR_PRIVATE_KEY); serialize these tests so they don't race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn install_with(mode: Option<&str>) -> Shim {
        let nsec = nostr::Keys::generate().secret_key().to_bech32().unwrap();
        install_with_key(mode, Some(&nsec))
    }

    /// Install with an explicit `NOSTR_PRIVATE_KEY` state: `Some(k)` sets it,
    /// `None` unsets it (the keyless direct-shim session).
    fn install_with_key(mode: Option<&str>, key: Option<&str>) -> Shim {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match key {
            Some(k) => std::env::set_var("NOSTR_PRIVATE_KEY", k),
            None => std::env::remove_var("NOSTR_PRIVATE_KEY"),
        }
        match mode {
            Some(m) => std::env::set_var("BUZZ_GIT_IDENTITY", m),
            None => std::env::remove_var("BUZZ_GIT_IDENTITY"),
        }
        let shim = Shim::install().expect("install");
        std::env::remove_var("BUZZ_GIT_IDENTITY");
        // `take_key_and_write` already scrubbed NOSTR_PRIVATE_KEY, but be sure.
        std::env::remove_var("NOSTR_PRIVATE_KEY");
        shim
    }

    fn shim_dir(shim: &Shim) -> PathBuf {
        std::env::split_paths(&shim.path_env)
            .next()
            .expect("shim dir is first PATH entry")
    }

    fn has(env: &[(String, String)], key: &str) -> bool {
        env.iter()
            .any(|(k, v)| k.starts_with("GIT_CONFIG_KEY_") && v == key)
    }

    #[test]
    fn agent_mode_installs_wrapper_signer_and_manifest() {
        let shim = install_with(Some("agent"));
        let dir = shim_dir(&shim);
        assert!(dir.join("git").exists(), "git wrapper must be installed");
        assert!(dir.join("git-sign-nostr").exists(), "signer installed");
        assert!(
            dir.join("git-credential-nostr").exists(),
            "credential helper installed"
        );
        assert!(
            buzz_git_identity::read_identity_manifest(&dir).is_some(),
            "manifest must be written"
        );
        assert!(has(&shim.git_env, "user.email"), "authorship injected");
        assert!(has(&shim.git_env, "credential.helper"), "helper injected");
    }

    #[test]
    fn user_mode_keeps_credential_helper_but_no_wrapper_signing_or_manifest() {
        let shim = install_with(Some("user"));
        let dir = shim_dir(&shim);
        // Auth survives: the credential helper and its keyfile pointer.
        assert!(
            dir.join("git-credential-nostr").exists(),
            "credential helper must survive user mode (auth ≠ attribution)"
        );
        assert!(has(&shim.git_env, "credential.helper"), "helper injected");
        assert!(
            has(&shim.git_env, "nostr.keyfile"),
            "keyfile pointer injected so the helper can load the key"
        );
        // Attribution machinery is absent: no wrapper, signer, manifest, or
        // authorship/signing config.
        assert!(
            !dir.join("git").exists(),
            "no git wrapper — vanilla git resolves the operator's identity"
        );
        assert!(!dir.join("git-sign-nostr").exists(), "no signer");
        assert!(
            buzz_git_identity::read_identity_manifest(&dir).is_none(),
            "no manifest in user mode"
        );
        assert!(!has(&shim.git_env, "user.email"), "no authorship");
        assert!(!has(&shim.git_env, "commit.gpgSign"), "no signing");
    }

    #[test]
    fn unset_defaults_to_agent() {
        let shim = install_with(None);
        assert!(shim_dir(&shim).join("git").exists(), "default is agent");
    }

    #[test]
    fn keyless_agent_session_gets_passthrough_git_not_a_bricked_wrapper() {
        // A direct dev-mcp session with NO key (unset) must not shadow `git`:
        // the wrapper reads a manifest as its authority and fails closed when it
        // is absent, but no key means no manifest, so installing it would refuse
        // even `git status`/`clone`. Documented keyless behavior is passthrough.
        let shim = install_with_key(Some("agent"), None);
        let dir = shim_dir(&shim);
        assert!(
            !dir.join("git").exists(),
            "keyless session must not install the enforcement wrapper"
        );
        assert!(
            !dir.join("git-sign-nostr").exists(),
            "no signer without a key"
        );
        assert!(
            buzz_git_identity::read_identity_manifest(&dir).is_none(),
            "no manifest without a key"
        );
        assert!(shim.git_env.is_empty(), "no git config without a key");
        // Non-git tools still install so the session is otherwise functional.
        assert!(dir.join("buzz").exists(), "buzz tool still installed");
    }

    #[test]
    fn invalid_key_agent_session_gets_passthrough_git_not_a_bricked_wrapper() {
        // An unparseable key is treated like no key (write_keyfile → None): the
        // session runs unattributed with plain passthrough git rather than a
        // wrapper that would refuse every command.
        let shim = install_with_key(Some("agent"), Some("not-a-valid-nsec"));
        let dir = shim_dir(&shim);
        assert!(
            !dir.join("git").exists(),
            "invalid-key session must not install the enforcement wrapper"
        );
        assert!(buzz_git_identity::read_identity_manifest(&dir).is_none());
        assert!(shim.git_env.is_empty(), "no git config on an invalid key");
    }

    #[test]
    fn invalid_mode_fails_install() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("BUZZ_GIT_IDENTITY", "usr");
        let err = Shim::install().expect_err("invalid mode must fail install");
        std::env::remove_var("BUZZ_GIT_IDENTITY");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
