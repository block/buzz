//! Deterministic agent git identity: the enforcement `git` wrapper and the
//! contract it shares with the `buzz-acp` harness that installs it.
//!
//! The harness writes the identity manifest ([`write_identity_manifest`]) into
//! its private install dir; the wrapper reads it back
//! ([`read_identity_manifest`]) and accepts it only when it carries exactly the
//! agent identity plus [`FIXED_SIGNING_ENTRIES`]. [`GitIdentityMode`] is the
//! operator's selector, read once by the harness at startup.

use std::path::Path;

pub mod git_wrapper;

/// A crate-wide, panic-safe environment harness for tests that must mutate the
/// process environment. The mutex coordinates sibling test modules; `Drop`
/// restores each variable's exact prior `OsString` while the lock is still held.
#[cfg(test)]
pub(crate) struct TestEnv {
    _lock: std::sync::MutexGuard<'static, ()>,
    prior: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(test)]
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
impl TestEnv {
    pub(crate) fn lock() -> Self {
        Self {
            _lock: ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            prior: Vec::new(),
        }
    }

    fn remember(&mut self, key: &'static str) {
        if !self.prior.iter().any(|(saved, _)| *saved == key) {
            self.prior.push((key, std::env::var_os(key)));
        }
    }

    pub(crate) fn set(&mut self, key: &'static str, value: impl AsRef<std::ffi::OsStr>) {
        self.remember(key);
        std::env::set_var(key, value);
    }
}

#[cfg(test)]
impl Drop for TestEnv {
    fn drop(&mut self) {
        for (key, value) in self.prior.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Operator-controlled selector for whose identity an agent's commits carry.
///
/// Read **once by the harness at startup** — never by the wrapper
/// per-invocation, which would let any agent `export BUZZ_GIT_IDENTITY=user`
/// mid-session and hollow out enforcement. Sovereignty lever, not an agent
/// escape hatch (VISION_SOVEREIGN.md: the operator overrides platform policy
/// on their own machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIdentityMode {
    /// Default. Commits are authored + NIP-GS-signed as the agent identity;
    /// the full enforcement wrapper is installed.
    Agent,
    /// The operator's own git identity authors commits. No wrapper, manifest,
    /// or injected authorship/signing config — vanilla git resolves the
    /// operator's repo/global config. Relay git-over-HTTP auth (the nostr
    /// credential helper) is unaffected: auth ≠ attribution.
    User,
}

impl GitIdentityMode {
    /// The operator-facing environment variable that selects the mode.
    pub const ENV_VAR: &'static str = "BUZZ_GIT_IDENTITY";

    /// Resolve from an explicit optional raw value: unset (`None`) → [`Agent`];
    /// exactly `agent` or `user` (surrounding whitespace tolerated) → the
    /// matching mode; anything else → `Err`.
    ///
    /// An unrecognized value **fails loudly** rather than silently falling back
    /// to either mode — silent fallback in an identity control is the failure
    /// class this whole feature exists to close.
    ///
    /// [`Agent`]: GitIdentityMode::Agent
    pub fn from_value(raw: Option<&std::ffi::OsStr>) -> Result<Self, String> {
        let Some(os) = raw else {
            return Ok(Self::Agent);
        };
        let value = os
            .to_str()
            .ok_or_else(|| format!("{} must be valid UTF-8 (`agent` or `user`)", Self::ENV_VAR))?;
        match value.trim() {
            "agent" => Ok(Self::Agent),
            "user" => Ok(Self::User),
            other => Err(format!(
                "{} must be `agent` or `user`, got {other:?}",
                Self::ENV_VAR
            )),
        }
    }

    /// Resolve from this process's environment. See [`from_value`].
    ///
    /// [`from_value`]: GitIdentityMode::from_value
    pub fn from_env() -> Result<Self, String> {
        Self::from_value(std::env::var_os(Self::ENV_VAR).as_deref())
    }
}

/// The signing-config entries whose values are the same for every agent — the
/// invariant part of the managed signing config. This is the single source of
/// truth for the fixed signing contract: the harness writes it, and the
/// wrapper's manifest validator ([`crate::git_wrapper`]) accepts a manifest
/// only when it carries exactly these key/value pairs, so the written and
/// validated contracts cannot drift apart.
pub const FIXED_SIGNING_ENTRIES: &[(&str, &str)] = &[
    ("gpg.format", "x509"),
    ("gpg.x509.program", "git-sign-nostr"),
    ("commit.gpgSign", "true"),
    ("tag.gpgSign", "true"),
];

/// Filename of the harness-owned identity manifest, written 0600 beside the
/// keyfile in the same 0700 install dir. It is the wrapper's authoritative
/// source for the identity/signing config it re-applies and the expected author
/// email it verifies pushes against — never the caller-mutable `GIT_CONFIG_*`
/// environment the wrapper is meant to constrain.
pub const IDENTITY_MANIFEST_NAME: &str = ".git-identity";

/// Serialize identity/signing `(key, value)` entries into the manifest and
/// write it 0600 into `dir` (which the caller created 0700). One `key=value`
/// per line; keys are fixed git config names (no `=`) so a first-`=` split
/// round-trips values that themselves contain `=`. Values never contain a
/// newline — the harness sanitizer strips control characters and the
/// keyfile path/email cannot — so lines are unambiguous.
pub fn write_identity_manifest(dir: &Path, entries: &[(String, String)]) -> std::io::Result<()> {
    let mut body = String::new();
    for (key, value) in entries {
        body.push_str(key);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    }
    write_keyfile_atomic(&dir.join(IDENTITY_MANIFEST_NAME), body.as_bytes())
}

/// Parse the identity manifest in `dir`, or `None` when it is absent/unreadable.
/// A present-but-empty or entry-less manifest yields `Some(vec![])`, which the
/// wrapper treats as an inconsistent authority and fails closed on.
pub fn read_identity_manifest(dir: &Path) -> Option<Vec<(String, String)>> {
    let body = std::fs::read_to_string(dir.join(IDENTITY_MANIFEST_NAME)).ok()?;
    Some(
        body.lines()
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect(),
    )
}

/// Write `data` to `path` with 0600 permissions set at creation time via
/// `OpenOptions::mode()` (no window where the file is world-readable).
/// `create_new` refuses to follow a pre-existing file or symlink.
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── GitIdentityMode ───────────────────────────────────────────────────────

    #[test]
    fn mode_unset_defaults_to_agent() {
        assert_eq!(
            GitIdentityMode::from_value(None).unwrap(),
            GitIdentityMode::Agent
        );
    }

    #[test]
    fn mode_parses_agent_and_user_tolerating_whitespace() {
        use std::ffi::OsStr;
        for raw in ["agent", " agent ", "agent\n"] {
            assert_eq!(
                GitIdentityMode::from_value(Some(OsStr::new(raw))).unwrap(),
                GitIdentityMode::Agent,
                "{raw:?} must parse as Agent"
            );
        }
        for raw in ["user", " user ", "user\n"] {
            assert_eq!(
                GitIdentityMode::from_value(Some(OsStr::new(raw))).unwrap(),
                GitIdentityMode::User,
                "{raw:?} must parse as User"
            );
        }
    }

    #[test]
    fn mode_rejects_unrecognized_value_naming_var_and_accepted_values() {
        use std::ffi::OsStr;
        // No silent fallback in an identity control: typos, empty, and
        // case-variants all fail loudly (git config values are case-sensitive).
        for raw in ["usr", "Agent", "USER", "true", "", "1"] {
            let err = GitIdentityMode::from_value(Some(OsStr::new(raw)))
                .expect_err(&format!("{raw:?} must be rejected"));
            assert!(
                err.contains("BUZZ_GIT_IDENTITY") && err.contains("agent") && err.contains("user"),
                "error must name the var and both values; got {err:?}"
            );
        }
    }
}
