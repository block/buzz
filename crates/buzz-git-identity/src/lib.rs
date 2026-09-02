//! Deterministic agent git identity — the single source of truth for the
//! `GIT_CONFIG_*` environment that attributes and signs an agent's commits.
//!
//! Two processes build this env: `buzz-dev-mcp`'s shim (for its own shell-tool
//! children) and the `buzz-acp` harness (for the agent-runtime child, so every
//! native shell of claude-code/codex/goose inherits it too). Both call into
//! this crate so the identity an agent commits under is byte-for-byte identical
//! regardless of which surface applied it.
//!
//! The pieces are deliberately separated:
//! - [`write_keyfile`] persists the nostr secret to a 0600 file and returns the
//!   derived public identity.
//! - [`identity_signing_entries`] is the author/email + NIP-GS signing config.
//! - [`nostr_credential_entries`] is the relay credential helper (applied
//!   globally only by the shim; the desktop scopes it per-URL instead).
//! - [`to_git_config_env`] flattens entries into `GIT_CONFIG_*` env pairs,
//!   composing over any `GIT_CONFIG_COUNT` already in the environment.

use nostr::ToBech32;
use std::path::Path;
use zeroize::Zeroize;

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

    pub(crate) fn remove(&mut self, key: &'static str) {
        self.remember(key);
        std::env::remove_var(key);
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

/// Public identity derived from a nostr secret key.
pub struct KeyIdentity {
    /// Absolute path to the 0600 keyfile holding the secret.
    pub keyfile_path: String,
    /// Lowercase hex public key — the stable attribution signal.
    pub pubkey_hex: String,
    /// `npub1…` bech32 form, used as the author-name fallback.
    pub npub: String,
}

/// Env var carrying the agent's Buzz display name, used as the git author name.
/// Distinct from the per-session UI title: commits outlive sessions, so
/// attribution must not follow a mutable, channel-qualified title.
pub const DISPLAY_NAME_ENV_VAR: &str = "BUZZ_ACP_DISPLAY_NAME";

/// Operator-controlled selector for whose identity an agent's commits carry.
///
/// Read **once by the harness/shim at spawn** — never by the wrapper
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

/// Max characters in a git author name. Nostr display names are unbounded.
const MAX_GIT_USER_NAME_CHARS: usize = 80;

/// Write the nostr private key to an owner-only file inside `dir` (which the
/// caller must have created 0700), then derive the public identity.
///
/// Returns `None` — and warns to stderr — when the key is empty, unparseable,
/// or the file cannot be written or named. Callers treat `None` as "no identity
/// env", which keeps a mis-provisioned session able to run (unattributed)
/// rather than aborting every commit.
pub fn write_keyfile(dir: &Path, raw: &str) -> Option<KeyIdentity> {
    if raw.is_empty() {
        return None;
    }
    let keys = match nostr::Keys::parse(raw) {
        Ok(k) => k,
        Err(e) => {
            eprintln!(
                "buzz-git-identity: warning: nostr key is set but invalid ({e}); \
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

    let keyfile = dir.join(".nostr-key");
    if write_keyfile_atomic(&keyfile, raw.as_bytes()).is_err() {
        eprintln!(
            "buzz-git-identity: warning: failed to write nostr keyfile; git auth/signing disabled"
        );
        return None;
    }
    let keyfile_path = match keyfile.to_str() {
        Some(s) => s.to_owned(),
        None => {
            eprintln!(
                "buzz-git-identity: warning: keyfile path is not valid UTF-8; \
                 git auth/signing disabled"
            );
            return None;
        }
    };

    Some(KeyIdentity {
        keyfile_path,
        pubkey_hex,
        npub,
    })
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

/// Derive a NIP-05-style email from the pubkey and relay URL.
/// Format: `<hex_pubkey>@<relay_host>`. Falls back to `<hex_pubkey>@buzz` when
/// no usable relay host is configured (unset, or a loopback host that carries
/// no attribution meaning). The pubkey — not the display name — is the stable
/// key NIP-98 auth, NIP-GS signing, and contributor matching all read.
pub fn derive_git_email(pubkey_hex: &str) -> String {
    let host = std::env::var("BUZZ_RELAY_URL")
        .ok()
        .and_then(|url| host_from_relay_url(&url))
        .filter(|h| !h.is_empty() && !h.starts_with("localhost") && !h.starts_with("127."))
        .unwrap_or_else(|| "buzz".to_owned());
    format!("{pubkey_hex}@{host}")
}

fn host_from_relay_url(url: &str) -> Option<String> {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("wss://"))
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    let host_port = stripped.split('/').next()?;
    Some(host_port.split(':').next().unwrap_or(host_port).to_owned())
}

/// Resolve the git author name: the sanitized [`DISPLAY_NAME_ENV_VAR`], or the
/// `npub` when that env var is unset or sanitizes away to nothing usable.
pub fn resolve_user_name(npub: &str) -> String {
    std::env::var(DISPLAY_NAME_ENV_VAR)
        .ok()
        .as_deref()
        .and_then(sanitize_git_user_name)
        .unwrap_or_else(|| npub.to_owned())
}

/// The author identity git config (`user.name` + `user.email`). Always safe to
/// apply: it introduces no dependency on an external signing program.
pub fn authorship_entries(id: &KeyIdentity) -> Vec<(String, String)> {
    let email = derive_git_email(&id.pubkey_hex);
    let user_name = resolve_user_name(&id.npub);
    vec![
        ("user.name".into(), user_name),
        ("user.email".into(), email),
    ]
}

/// The signing-config entries whose values are the same for every agent — the
/// invariant part of [`signing_entries`]. This is the single source of truth
/// for the fixed signing contract: [`signing_entries`] writes it, and the
/// wrapper's manifest validator ([`crate::git_wrapper`]) accepts a manifest
/// only when it carries exactly these key/value pairs, so the written and
/// validated contracts cannot drift apart.
pub const FIXED_SIGNING_ENTRIES: &[(&str, &str)] = &[
    ("gpg.format", "x509"),
    ("gpg.x509.program", "git-sign-nostr"),
    ("commit.gpgSign", "true"),
    ("tag.gpgSign", "true"),
];

/// The NIP-GS signing git config. Applying this makes git invoke
/// `git-sign-nostr` for every commit/tag, so the caller MUST ensure that
/// program is reachable on the child's PATH — otherwise every commit fails.
pub fn signing_entries(id: &KeyIdentity) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = FIXED_SIGNING_ENTRIES
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    entries.push(("user.signingkey".into(), id.pubkey_hex.clone()));
    entries.push(keyfile_entry(id));
    entries
}

/// The `nostr.keyfile` git config pointing at the 0600 keyfile. Both
/// `git-sign-nostr` and `git-credential-nostr` fall back to it to load the
/// secret when `NOSTR_PRIVATE_KEY` is scrubbed from the child env. It is part
/// of [`signing_entries`], but the credential helper needs it even in
/// `user`-identity mode (where signing/authorship are off) — auth ≠
/// attribution — so it is exposed separately.
pub fn keyfile_entry(id: &KeyIdentity) -> (String, String) {
    ("nostr.keyfile".into(), id.keyfile_path.clone())
}

/// The identity + signing git config for an agent, as ordered `(key, value)`
/// pairs. Excludes the credential helper — that is applied separately (globally
/// by the shim, per-URL by the desktop) so this set can be lifted onto the
/// agent-runtime child without duplicating the desktop's scoped helper.
pub fn identity_signing_entries(id: &KeyIdentity) -> Vec<(String, String)> {
    let mut entries = authorship_entries(id);
    entries.extend(signing_entries(id));
    entries
}

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
/// newline — [`sanitize_git_user_name`] strips control characters and the
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

/// The nostr credential helper git config (relay git-over-HTTP auth). Additive:
/// the helper silently declines non-Buzz remotes, so git falls through to
/// system helpers for GitHub/GitLab/etc.
pub fn nostr_credential_entries() -> Vec<(String, String)> {
    vec![
        ("credential.helper".into(), "nostr".into()),
        ("credential.useHttpPath".into(), "true".into()),
    ]
}

/// Flatten `(key, value)` git config entries into `GIT_CONFIG_COUNT`/`KEY_n`/
/// `VALUE_n` env pairs, composing over any `GIT_CONFIG_COUNT` already present
/// **in this process's environment** so a caller's existing config is preserved
/// rather than clobbered.
///
/// Use this only when the base config lives in the process env (e.g. the
/// dev-mcp shim, which sets `GIT_CONFIG_*` as its own env for shell children).
/// When the base is staged on a child `Command` instead — the Desktop-to-harness
/// path, where the process env's count is 0 but the child already carries
/// `GIT_CONFIG_COUNT=2` and indices 0–1 — read that count and call
/// [`to_git_config_env_from_base`] with it, or the appended entries will
/// overwrite the child's indices 0..n and orphan its per-relay credential
/// helper.
pub fn to_git_config_env(entries: &[(String, String)]) -> Vec<(String, String)> {
    let base: usize = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    to_git_config_env_from_base(entries, base)
}

/// Flatten `(key, value)` git config entries into `GIT_CONFIG_COUNT`/`KEY_n`/
/// `VALUE_n` env pairs, composing over `base` existing entries: the new count is
/// `base + entries.len()` and the new indices start at `base`, so entries
/// already occupying indices `0..base` are preserved.
pub fn to_git_config_env_from_base(
    entries: &[(String, String)],
    base: usize,
) -> Vec<(String, String)> {
    let mut env = Vec::with_capacity(entries.len() * 2 + 1);
    env.push((
        "GIT_CONFIG_COUNT".into(),
        (base + entries.len()).to_string(),
    ));
    for (i, (key, val)) in entries.iter().enumerate() {
        let idx = base + i;
        env.push((format!("GIT_CONFIG_KEY_{idx}"), key.clone()));
        env.push((format!("GIT_CONFIG_VALUE_{idx}"), val.clone()));
    }
    env
}

/// Convenience for the common case: read the secret from `NOSTR_PRIVATE_KEY`,
/// remove it from this process's env so children never inherit it, write the
/// keyfile into `dir`, and return the derived identity. Mirrors the shim's
/// contract. `None` when the var is unset/empty/invalid or the keyfile fails.
pub fn take_key_and_write(dir: &Path) -> Option<KeyIdentity> {
    let mut raw = std::env::var("NOSTR_PRIVATE_KEY").ok();
    std::env::remove_var("NOSTR_PRIVATE_KEY");
    let id = raw.as_deref().and_then(|k| write_keyfile(dir, k));
    if let Some(ref mut k) = raw {
        k.zeroize();
    }
    id
}

/// Like [`take_key_and_write`] but **leaves** `NOSTR_PRIVATE_KEY` in the env.
///
/// The harness uses this: it lifts identity onto the agent-runtime child, but
/// that child still needs `NOSTR_PRIVATE_KEY` to reach `buzz-dev-mcp`, whose
/// shim performs the remove-from-env dance for its own subtree. Removing it at
/// the harness layer would blind dev-mcp's credential helper and signer.
pub fn read_key_and_write(dir: &Path) -> Option<KeyIdentity> {
    let mut raw = std::env::var("NOSTR_PRIVATE_KEY").ok();
    let id = raw.as_deref().and_then(|k| write_keyfile(dir, k));
    if let Some(ref mut k) = raw {
        k.zeroize();
    }
    id
}

/// Characters git's `ident.c` treats as "crud": stripped from both ends of a
/// name, and — when a name is *nothing but* these — rejected outright with
/// `fatal: name consists only of disallowed characters`.
///
/// Verified empirically against git 2.54.0 by committing with each ASCII byte
/// 32..=126 as the entire `user.name`: exactly space, `"`, `'`, `,`, `:`, `;`,
/// `<`, `>`, and `\` abort. Control characters abort too (the predicate is
/// `c <= 32`). Note `.` is *not* crud in this version despite older lore.
fn is_git_crud(c: char) -> bool {
    c <= ' ' || matches!(c, '"' | '\'' | ',' | ':' | ';' | '<' | '>' | '\\')
}

/// Characters in Unicode general category `Cf` (format): zero-width space and
/// joiners, bidi embedding/override marks, invisible math operators, interlinear
/// annotations, and tag characters.
///
/// `char::is_control` covers only `Cc`, so every one of these survives it — and
/// none is whitespace or [`is_git_crud`]. A display name of nothing but U+200B
/// ZERO WIDTH SPACE would therefore satisfy the "at least one non-crud
/// character" gate and hand git a visually blank author instead of falling back
/// to the npub. An embedded U+202E RIGHT-TO-LEFT OVERRIDE is worse: it makes a
/// commit's persisted author line render as something other than what it says,
/// the same confusion the angle-bracket filter exists to prevent.
///
/// The whole category is rejected rather than the two known-bad marks, because
/// the boundary that matters is "invisible or reorders text", not "the codepoint
/// someone thought of". Ranges transcribed from the UCD's
/// `DerivedGeneralCategory.txt` (17.0.0) and independently cross-checked against
/// Python's `unicodedata` (16.0.0); both yield exactly these 21 ranges. Inlined
/// rather than taking a Unicode-tables dependency for one predicate.
fn is_unicode_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}'
        | '\u{0600}'..='\u{0605}'
        | '\u{061C}'
        | '\u{06DD}'
        | '\u{070F}'
        | '\u{0890}'..='\u{0891}'
        | '\u{08E2}'
        | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}'
        | '\u{110BD}'
        | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0001}'
        | '\u{E0020}'..='\u{E007F}'
    )
}

/// Normalize a Buzz display name into a git author name, or `None` to fall
/// back to the npub.
///
/// Strips control and Unicode format characters plus angle brackets, collapses
/// whitespace runs, trims, and caps at [`MAX_GIT_USER_NAME_CHARS`] by `chars()`
/// so a multi-byte name cannot be split mid-UTF-8. Angle brackets go because git
/// silently drops them rather than erroring — `Duncan <evil@x.com>` would
/// render as `Duncan evil@x.com <hex@relay>`, which forges nothing but reads as
/// though it might.
///
/// Returns `None` unless at least one non-crud character survives. A bare
/// emptiness check is not sufficient: git rejects a name built only of crud,
/// so a display name of `;;` or `""` would abort **every commit** the agent
/// makes. Falling back to the npub keeps the agent able to commit.
pub fn sanitize_git_user_name(raw: &str) -> Option<String> {
    let collapsed = raw
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| !c.is_control() && !is_unicode_format(*c) && *c != '<' && *c != '>')
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let name: String = collapsed
        .chars()
        .take(MAX_GIT_USER_NAME_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string();
    name.chars().any(|c| !is_git_crud(c)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBKEY_HEX: &str = "dcfd242e557282d7a1e2cf2e6877522682f1e5c6156dc92ca7d90eaedd3b0f95";
    const NPUB: &str = "npub1mn7jgtj4w2pd0g0zeuhxsa6jy6p0rewxz4kujt98my82ahfmp72sxjexk7";

    fn identity() -> KeyIdentity {
        KeyIdentity {
            keyfile_path: "/tmp/.nostr-key".into(),
            pubkey_hex: PUBKEY_HEX.into(),
            npub: NPUB.into(),
        }
    }

    /// Read a git config value back out of flattened GIT_CONFIG_KEY_n/VALUE_n pairs.
    fn git_config(env: &[(String, String)], key: &str) -> Option<String> {
        let idx = env
            .iter()
            .find(|(k, v)| k.starts_with("GIT_CONFIG_KEY_") && v == key)?
            .0
            .strip_prefix("GIT_CONFIG_KEY_")?
            .to_owned();
        env.iter()
            .find(|(k, _)| *k == format!("GIT_CONFIG_VALUE_{idx}"))
            .map(|(_, v)| v.clone())
    }

    // ── sanitize_git_user_name ────────────────────────────────────────────────

    #[test]
    fn ordinary_name_passes_through_unchanged() {
        assert_eq!(sanitize_git_user_name("Duncan"), Some("Duncan".into()));
    }

    #[test]
    fn angle_brackets_are_stripped_so_no_second_email_is_rendered() {
        assert_eq!(
            sanitize_git_user_name("Duncan <evil@x.com>"),
            Some("Duncan evil@x.com".into())
        );
    }

    #[test]
    fn whitespace_control_characters_become_a_single_separator() {
        assert_eq!(
            sanitize_git_user_name("Dun\ncan\tThe\r\nIdaho"),
            Some("Dun can The Idaho".into())
        );
    }

    #[test]
    fn non_whitespace_control_characters_are_dropped_outright() {
        // NUL is the important one: an interior NUL makes `Command::env` fail
        // the entire spawn upstream, so it must never survive to git config.
        let got = sanitize_git_user_name("Idaho\0Blade\u{7}").expect("non-empty");
        assert_eq!(got, "IdahoBlade");
        assert!(!got.chars().any(char::is_control));
    }

    #[test]
    fn internal_whitespace_runs_collapse_to_one_space() {
        assert_eq!(
            sanitize_git_user_name("  Duncan   Idaho  "),
            Some("Duncan Idaho".into())
        );
    }

    #[test]
    fn whitespace_only_name_falls_back_to_npub() {
        assert_eq!(sanitize_git_user_name("   \t\n  "), None);
    }

    #[test]
    fn empty_name_falls_back_to_npub() {
        assert_eq!(sanitize_git_user_name(""), None);
    }

    #[test]
    fn crud_only_name_falls_back_rather_than_aborting_every_commit() {
        for raw in ["<>", ";;", "\"\"", "''", ",", ":", "\\", ",;:"] {
            assert_eq!(
                sanitize_git_user_name(raw),
                None,
                "crud-only name {raw:?} must fall back to the npub"
            );
        }
    }

    #[test]
    fn crud_mixed_with_real_characters_is_kept() {
        assert_eq!(sanitize_git_user_name("O'Brien"), Some("O'Brien".into()));
        assert_eq!(
            sanitize_git_user_name("Smith, Jr."),
            Some("Smith, Jr.".into())
        );
    }

    #[test]
    fn over_length_name_is_truncated_to_the_cap() {
        let long = "a".repeat(200);
        let got = sanitize_git_user_name(&long).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let long = "🐝".repeat(200);
        let got = sanitize_git_user_name(&long).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
        assert!(got.chars().all(|c| c == '🐝'), "no replacement chars");
    }

    #[test]
    fn truncation_does_not_leave_a_trailing_space() {
        let raw = format!("{} tail", "a".repeat(MAX_GIT_USER_NAME_CHARS - 1));
        let got = sanitize_git_user_name(&raw).expect("non-empty");
        assert!(!got.ends_with(' '), "got {got:?}");
    }

    #[test]
    fn non_ascii_names_survive() {
        assert_eq!(
            sanitize_git_user_name("Élodie 🐝"),
            Some("Élodie 🐝".into())
        );
    }

    #[test]
    fn format_only_name_falls_back_to_npub() {
        assert_eq!(sanitize_git_user_name("\u{200B}\u{200B}"), None);
        for raw in ["\u{200D}", "\u{2060}", "\u{FEFF}", "\u{202E}", "\u{00AD}"] {
            assert_eq!(
                sanitize_git_user_name(raw),
                None,
                "format-only name {raw:?} must fall back to the npub"
            );
        }
    }

    #[test]
    fn bidi_override_is_stripped_and_the_name_is_kept() {
        assert_eq!(
            sanitize_git_user_name("Duncan\u{202E}"),
            Some("Duncan".into())
        );
        assert_eq!(
            sanitize_git_user_name("Dun\u{202E}can Idaho"),
            Some("Duncan Idaho".into())
        );
    }

    #[test]
    fn zero_width_space_inside_a_word_is_removed_without_splitting_it() {
        assert_eq!(
            sanitize_git_user_name("Dun\u{200B}can"),
            Some("Duncan".into())
        );
    }

    #[test]
    fn format_characters_do_not_consume_the_length_budget() {
        let raw = format!("{}{}", "\u{200B}".repeat(200), "a".repeat(90));
        let got = sanitize_git_user_name(&raw).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
        assert!(got.chars().all(|c| c == 'a'), "got {got:?}");
    }

    #[test]
    fn unicode_format_covers_every_cf_range_and_nothing_adjacent() {
        for c in [
            '\u{00AD}',
            '\u{0600}',
            '\u{0605}',
            '\u{061C}',
            '\u{06DD}',
            '\u{070F}',
            '\u{0890}',
            '\u{0891}',
            '\u{08E2}',
            '\u{180E}',
            '\u{200B}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2060}',
            '\u{2064}',
            '\u{2066}',
            '\u{206F}',
            '\u{FEFF}',
            '\u{FFF9}',
            '\u{FFFB}',
            '\u{110BD}',
            '\u{110CD}',
            '\u{13430}',
            '\u{1343F}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
            '\u{E0001}',
            '\u{E0020}',
            '\u{E007F}',
        ] {
            assert!(is_unicode_format(c), "U+{:04X} is Cf", c as u32);
        }
        for c in [
            '\u{00AC}',
            '\u{00AE}',
            '\u{05FF}',
            '\u{0606}',
            '\u{061B}',
            '\u{061D}',
            '\u{200A}',
            '\u{2010}',
            '\u{2029}',
            '\u{202F}',
            '\u{2065}',
            '\u{205F}',
            '\u{2070}',
            '\u{FEFE}',
            '\u{FFF8}',
            '\u{FFFC}',
            '\u{110BC}',
            '\u{1342F}',
            '\u{E0000}',
            '\u{E0080}',
            'a',
            ' ',
            '🐝',
            'É',
        ] {
            assert!(!is_unicode_format(c), "U+{:04X} is not Cf", c as u32);
        }
    }

    #[test]
    fn git_crud_set_matches_observed_git_behavior() {
        for c in [' ', '"', '\'', ',', ':', ';', '<', '>', '\\', '\t', '\n'] {
            assert!(is_git_crud(c), "{c:?} should be crud");
        }
        for c in ['.', '-', '_', '@', '(', 'a', '🐝'] {
            assert!(!is_git_crud(c), "{c:?} should not be crud");
        }
    }

    // ── identity_signing_entries / resolve_user_name ──────────────────────────

    #[test]
    fn identity_uses_display_name_and_leaves_email_on_the_pubkey() {
        let mut env = crate::TestEnv::lock();
        env.set(DISPLAY_NAME_ENV_VAR, "Duncan");
        env.remove("BUZZ_RELAY_URL");
        let entries = identity_signing_entries(&identity());

        assert_eq!(entry(&entries, "user.name").as_deref(), Some("Duncan"));
        assert_eq!(
            entry(&entries, "user.email").as_deref(),
            Some(format!("{PUBKEY_HEX}@buzz").as_str())
        );
        assert_eq!(
            entry(&entries, "user.signingkey").as_deref(),
            Some(PUBKEY_HEX)
        );
        assert_eq!(entry(&entries, "gpg.format").as_deref(), Some("x509"));
        assert_eq!(
            entry(&entries, "gpg.x509.program").as_deref(),
            Some("git-sign-nostr")
        );
        assert_eq!(entry(&entries, "commit.gpgSign").as_deref(), Some("true"));
        assert_eq!(
            entry(&entries, "nostr.keyfile").as_deref(),
            Some("/tmp/.nostr-key")
        );
        // The credential helper is deliberately NOT part of this set.
        assert_eq!(entry(&entries, "credential.helper"), None);
    }

    #[test]
    fn identity_falls_back_to_npub_when_display_name_unset() {
        let mut env = crate::TestEnv::lock();
        env.remove(DISPLAY_NAME_ENV_VAR);
        env.remove("BUZZ_RELAY_URL");
        let entries = identity_signing_entries(&identity());
        assert_eq!(entry(&entries, "user.name").as_deref(), Some(NPUB));
    }

    #[test]
    fn identity_falls_back_to_npub_when_display_name_is_unusable() {
        let mut env = crate::TestEnv::lock();
        env.remove("BUZZ_RELAY_URL");
        for raw in ["<>", "\u{200B}"] {
            env.set(DISPLAY_NAME_ENV_VAR, raw);
            let entries = identity_signing_entries(&identity());
            assert_eq!(
                entry(&entries, "user.name").as_deref(),
                Some(NPUB),
                "unusable display name {raw:?} must reach git as the npub"
            );
        }
    }

    fn entry(entries: &[(String, String)], key: &str) -> Option<String> {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

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

    // ── derive_git_email ──────────────────────────────────────────────────────

    #[test]
    fn email_uses_relay_host_stripping_scheme_port_and_path() {
        let mut env = crate::TestEnv::lock();
        env.set("BUZZ_RELAY_URL", "wss://relay.example.com:443/path");
        assert_eq!(
            derive_git_email(PUBKEY_HEX),
            format!("{PUBKEY_HEX}@relay.example.com")
        );
    }

    #[test]
    fn email_falls_back_to_buzz_for_loopback_and_unset() {
        let mut env = crate::TestEnv::lock();
        for url in ["http://localhost:3000", "ws://127.0.0.1:8080"] {
            env.set("BUZZ_RELAY_URL", url);
            assert_eq!(derive_git_email(PUBKEY_HEX), format!("{PUBKEY_HEX}@buzz"));
        }
        env.remove("BUZZ_RELAY_URL");
        assert_eq!(derive_git_email(PUBKEY_HEX), format!("{PUBKEY_HEX}@buzz"));
    }

    // ── to_git_config_env composition ─────────────────────────────────────────

    #[test]
    fn to_git_config_env_composes_over_existing_count() {
        let mut env_guard = crate::TestEnv::lock();
        env_guard.set("GIT_CONFIG_COUNT", "2");
        let env = to_git_config_env(&[("user.name".into(), "Duncan".into())]);

        assert_eq!(git_config_count(&env), 3);
        // The new entry lands at index 2 (0 and 1 belong to the caller).
        assert_eq!(git_config(&env, "user.name").as_deref(), Some("Duncan"));
        assert!(env.iter().any(|(k, _)| k == "GIT_CONFIG_KEY_2"));
    }

    #[test]
    fn to_git_config_env_base_zero_when_unset() {
        let mut env_guard = crate::TestEnv::lock();
        env_guard.remove("GIT_CONFIG_COUNT");
        let env = to_git_config_env(&[
            ("user.name".into(), "Duncan".into()),
            ("user.email".into(), "x@y".into()),
        ]);
        assert_eq!(git_config_count(&env), 2);
        assert!(env.iter().any(|(k, _)| k == "GIT_CONFIG_KEY_0"));
        assert!(env.iter().any(|(k, _)| k == "GIT_CONFIG_KEY_1"));
    }

    fn git_config_count(env: &[(String, String)]) -> usize {
        env.iter()
            .find(|(k, _)| k == "GIT_CONFIG_COUNT")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap()
    }

    // ── write_keyfile / take_key_and_write ────────────────────────────────────

    #[test]
    fn write_keyfile_rejects_empty_and_invalid() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_keyfile(dir.path(), "").is_none());
        assert!(write_keyfile(dir.path(), "not-a-key").is_none());
    }

    #[test]
    fn write_keyfile_persists_0600_and_derives_pubkey() {
        let dir = tempfile::tempdir().unwrap();
        let nsec = nostr::Keys::generate().secret_key().to_bech32().unwrap();
        let id = write_keyfile(dir.path(), &nsec).expect("valid key");
        assert_eq!(id.pubkey_hex.len(), 64);
        assert!(id.npub.starts_with("npub1"));
        assert!(std::path::Path::new(&id.keyfile_path).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&id.keyfile_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn take_key_and_write_removes_var_from_env() {
        let mut env = crate::TestEnv::lock();
        let dir = tempfile::tempdir().unwrap();
        let nsec = nostr::Keys::generate().secret_key().to_bech32().unwrap();
        env.set("NOSTR_PRIVATE_KEY", &nsec);
        let id = take_key_and_write(dir.path());
        assert!(id.is_some());
        assert!(
            std::env::var("NOSTR_PRIVATE_KEY").is_err(),
            "secret must be removed from the process env"
        );
    }
}
