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
/// Shell children receive `path_env`, `git_env`, and `BUZZ_PRIVATE_KEY` (for
/// the buzz CLI). `NOSTR_PRIVATE_KEY` is removed from the process env after
/// the keyfile is written — git helpers read from the keyfile only.
/// Cleaned up on drop (TempDir).
pub struct Shim {
    _dir: TempDir,
    pub path_env: String,
    pub git_env: Vec<(String, String)>,
}

impl Shim {
    pub fn install() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("buzz-dev-mcp-").tempdir()?;
        set_owner_only(dir.path())?;

        let self_exe = std::env::current_exe()?;

        // Multicall symlinks — all resolve back to this binary.
        for name in [
            "rg",
            "tree",
            "buzz",
            "git-credential-nostr",
            "git-sign-nostr",
        ] {
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
                "buzz-dev-mcp: warning: NOSTR_PRIVATE_KEY is set but invalid ({e}); \
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
            "buzz-dev-mcp: warning: failed to write nostr keyfile; git auth/signing disabled"
        );
        return None;
    }
    let keyfile_path = match keyfile.to_str() {
        Some(s) => s.to_owned(),
        None => {
            eprintln!(
                "buzz-dev-mcp: warning: tempdir path is not valid UTF-8; git auth/signing disabled"
            );
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
/// Format: `<hex_pubkey>@<relay_host>` (e.g., `ab12...cd@relay.buzz.dev`).
/// Falls back to `<hex_pubkey>@buzz` if no relay URL is configured.
fn derive_git_email(pubkey_hex: &str) -> String {
    let host = std::env::var("BUZZ_RELAY_URL")
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
        .unwrap_or_else(|| "buzz".to_owned());
    format!("{pubkey_hex}@{host}")
}

/// Stable identity contract for git attribution: the bare agent display name,
/// never channel-qualified, safe to embed in commit history.
///
/// Deliberately distinct from `BUZZ_ACP_SESSION_TITLE`, which is per-session UI
/// chrome and may be composed (`Agent · #channel`) by consumers. Commits
/// outlive sessions, so git attribution must not follow a mutable title.
///
/// Nothing writes this yet — when unset, [`build_git_env`] falls back to the
/// npub, which is byte-for-byte today's behavior.
const DISPLAY_NAME_ENV_VAR: &str = "BUZZ_ACP_DISPLAY_NAME";

/// Max characters in a git author name. Nostr display names are unbounded.
const MAX_GIT_USER_NAME_CHARS: usize = 80;

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
fn sanitize_git_user_name(raw: &str) -> Option<String> {
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

/// Build GIT_CONFIG_COUNT/KEY/VALUE env vars for ephemeral nostr git config.
/// Composes with any existing GIT_CONFIG_COUNT in the environment. When launched
/// via buzz-agent (which clears env), the base is always 0 — composition only
/// matters when dev-mcp is run directly with pre-existing GIT_CONFIG vars.
fn build_git_env(info: &KeyInfo) -> Vec<(String, String)> {
    let email = derive_git_email(&info.pubkey_hex);
    // Display name for humans reading `git log`; the pubkey stays in the email,
    // which is what NIP-98 auth, NIP-GS signing, and contributor matching key on.
    let user_name = std::env::var(DISPLAY_NAME_ENV_VAR)
        .ok()
        .as_deref()
        .and_then(sanitize_git_user_name)
        .unwrap_or_else(|| info.npub.clone());
    let entries: Vec<(&str, String)> = vec![
        // Identity — Buzz display name (npub fallback), NIP-05-style email
        ("user.name", user_name),
        ("user.email", email),
        // Nostr credential helper is additive — it silently declines non-Buzz
        // remotes (exits 0, no credential), so git falls through to system
        // helpers (osxkeychain, store, etc.) for GitHub/GitLab/etc.
        ("credential.helper", "nostr".into()),
        // Required: Buzz relay verifies NIP-98 against the full repo-root URL.
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

#[cfg(test)]
mod git_user_name_tests {
    use super::{
        build_git_env, is_git_crud, is_unicode_format, sanitize_git_user_name, KeyInfo,
        MAX_GIT_USER_NAME_CHARS,
    };
    use std::sync::Mutex;

    /// Env-var-touching tests must run serially — env vars are process-global.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const PUBKEY_HEX: &str = "dcfd242e557282d7a1e2cf2e6877522682f1e5c6156dc92ca7d90eaedd3b0f95";
    const NPUB: &str = "npub1mn7jgtj4w2pd0g0zeuhxsa6jy6p0rewxz4kujt98my82ahfmp72sxjexk7";

    fn key_info() -> KeyInfo {
        KeyInfo {
            keyfile_path: "/tmp/.nostr-key".into(),
            pubkey_hex: PUBKEY_HEX.into(),
            npub: NPUB.into(),
        }
    }

    /// Read a git config value back out of the flat GIT_CONFIG_KEY_n/VALUE_n pairs.
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

    #[test]
    fn test_ordinary_name_passes_through_unchanged() {
        assert_eq!(sanitize_git_user_name("Duncan"), Some("Duncan".into()));
    }

    #[test]
    fn test_angle_brackets_are_stripped_so_no_second_email_is_rendered() {
        // git drops the brackets itself and renders `Duncan evil@x.com
        // <hex@relay>` — no forgery, but a confusing author line.
        assert_eq!(
            sanitize_git_user_name("Duncan <evil@x.com>"),
            Some("Duncan evil@x.com".into())
        );
    }

    #[test]
    fn test_whitespace_control_characters_become_a_single_separator() {
        // Newline, tab and carriage return are whitespace: they collapse to one
        // space like any other run, so a multi-line name stays readable.
        assert_eq!(
            sanitize_git_user_name("Dun\ncan\tThe\r\nIdaho"),
            Some("Dun can The Idaho".into())
        );
    }

    #[test]
    fn test_non_whitespace_control_characters_are_dropped_outright() {
        // NUL is the important one: an interior NUL makes `Command::env` fail
        // the entire spawn upstream, so it must never survive to git config.
        let got = sanitize_git_user_name("Idaho\0Blade\u{7}").expect("non-empty");
        assert_eq!(got, "IdahoBlade");
        assert!(!got.chars().any(char::is_control));
    }

    #[test]
    fn test_internal_whitespace_runs_collapse_to_one_space() {
        assert_eq!(
            sanitize_git_user_name("  Duncan   Idaho  "),
            Some("Duncan Idaho".into())
        );
    }

    #[test]
    fn test_whitespace_only_name_falls_back_to_npub() {
        assert_eq!(sanitize_git_user_name("   \t\n  "), None);
    }

    #[test]
    fn test_empty_name_falls_back_to_npub() {
        assert_eq!(sanitize_git_user_name(""), None);
    }

    #[test]
    fn test_crud_only_name_falls_back_rather_than_aborting_every_commit() {
        // git rejects a name built only of crud with `fatal: name consists
        // only of disallowed characters`, which would break EVERY commit the
        // agent makes. Verified against git 2.54.0.
        for raw in ["<>", ";;", "\"\"", "''", ",", ":", "\\", ",;:"] {
            assert_eq!(
                sanitize_git_user_name(raw),
                None,
                "crud-only name {raw:?} must fall back to the npub"
            );
        }
    }

    #[test]
    fn test_crud_mixed_with_real_characters_is_kept() {
        // Legitimate names contain crud; only an all-crud result is fatal.
        assert_eq!(sanitize_git_user_name("O'Brien"), Some("O'Brien".into()));
        assert_eq!(
            sanitize_git_user_name("Smith, Jr."),
            Some("Smith, Jr.".into())
        );
    }

    #[test]
    fn test_over_length_name_is_truncated_to_the_cap() {
        let long = "a".repeat(200);
        let got = sanitize_git_user_name(&long).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
    }

    #[test]
    fn test_truncation_never_splits_a_multibyte_character() {
        let long = "🐝".repeat(200);
        let got = sanitize_git_user_name(&long).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
        assert!(got.chars().all(|c| c == '🐝'), "no replacement chars");
    }

    #[test]
    fn test_truncation_does_not_leave_a_trailing_space() {
        // Cutting mid-word would otherwise strand the separator at the end.
        let raw = format!("{} tail", "a".repeat(MAX_GIT_USER_NAME_CHARS - 1));
        let got = sanitize_git_user_name(&raw).expect("non-empty");
        assert!(!got.ends_with(' '), "got {got:?}");
    }

    #[test]
    fn test_non_ascii_names_survive() {
        assert_eq!(
            sanitize_git_user_name("Élodie 🐝"),
            Some("Élodie 🐝".into())
        );
    }

    #[test]
    fn test_format_only_name_falls_back_to_npub() {
        // U+200B is neither control, nor whitespace, nor crud, so before Cf
        // filtering this passed the non-crud gate and handed git a visually
        // blank author instead of falling back.
        assert_eq!(sanitize_git_user_name("\u{200B}\u{200B}"), None);
        // Same class, different marks: joiner, word joiner, BOM, bidi override.
        for raw in ["\u{200D}", "\u{2060}", "\u{FEFF}", "\u{202E}", "\u{00AD}"] {
            assert_eq!(
                sanitize_git_user_name(raw),
                None,
                "format-only name {raw:?} must fall back to the npub"
            );
        }
    }

    #[test]
    fn test_bidi_override_is_stripped_and_the_name_is_kept() {
        // A trailing RLO would reorder everything after it in `git log`, so the
        // mark goes and the readable name stays.
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
    fn test_zero_width_space_inside_a_word_is_removed_without_splitting_it() {
        // U+200B is not whitespace, so it must not become a separator: the word
        // rejoins rather than turning into "Dun can".
        assert_eq!(
            sanitize_git_user_name("Dun\u{200B}can"),
            Some("Duncan".into())
        );
    }

    #[test]
    fn test_format_characters_do_not_consume_the_length_budget() {
        // Filtering happens before truncation, so invisible padding cannot
        // shorten the visible name.
        let raw = format!("{}{}", "\u{200B}".repeat(200), "a".repeat(90));
        let got = sanitize_git_user_name(&raw).expect("non-empty");
        assert_eq!(got.chars().count(), MAX_GIT_USER_NAME_CHARS);
        assert!(got.chars().all(|c| c == 'a'), "got {got:?}");
    }

    #[test]
    fn test_unicode_format_covers_every_cf_range_and_nothing_adjacent() {
        // Both endpoints of each of the 21 `Cf` ranges in UCD 17.0.0. Endpoints
        // are what a transcription error moves, so they are what gets asserted.
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
        // Codepoints immediately outside those ranges, plus ordinary characters.
        // U+2065 is the notable one: it sits *inside* the 2060..206F block but
        // is unassigned, not `Cf`.
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
    fn test_build_git_env_uses_display_name_and_leaves_email_on_the_pubkey() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("BUZZ_ACP_DISPLAY_NAME", "Duncan");
        std::env::remove_var("BUZZ_RELAY_URL");
        std::env::remove_var("GIT_CONFIG_COUNT");
        let env = build_git_env(&key_info());
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");

        assert_eq!(git_config(&env, "user.name").as_deref(), Some("Duncan"));
        // The pubkey — the thing NIP-98 auth, NIP-GS signing, and contributor
        // matching key on — must stay in the email untouched.
        assert_eq!(
            git_config(&env, "user.email").as_deref(),
            Some(format!("{PUBKEY_HEX}@buzz").as_str())
        );
        assert_eq!(
            git_config(&env, "user.signingkey").as_deref(),
            Some(PUBKEY_HEX)
        );
    }

    #[test]
    fn test_build_git_env_falls_back_to_npub_when_display_name_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");
        std::env::remove_var("BUZZ_RELAY_URL");
        std::env::remove_var("GIT_CONFIG_COUNT");
        let env = build_git_env(&key_info());

        // Today's behavior, and what every agent gets until a writer for
        // BUZZ_ACP_DISPLAY_NAME lands on the Desktop side.
        assert_eq!(git_config(&env, "user.name").as_deref(), Some(NPUB));
        assert_eq!(
            git_config(&env, "user.email").as_deref(),
            Some(format!("{PUBKEY_HEX}@buzz").as_str())
        );
    }

    #[test]
    fn test_build_git_env_falls_back_to_npub_when_display_name_is_unusable() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("BUZZ_RELAY_URL");
        std::env::remove_var("GIT_CONFIG_COUNT");

        // Crud-only and format-only names both reach git as the npub — one
        // would abort every commit, the other would render as blank.
        for raw in ["<>", "\u{200B}"] {
            std::env::set_var("BUZZ_ACP_DISPLAY_NAME", raw);
            let env = build_git_env(&key_info());
            assert_eq!(
                git_config(&env, "user.name").as_deref(),
                Some(NPUB),
                "unusable display name {raw:?} must reach git as the npub"
            );
        }
        std::env::remove_var("BUZZ_ACP_DISPLAY_NAME");
    }

    #[test]
    fn test_git_crud_set_matches_observed_git_behavior() {
        // Empirically derived from git 2.54.0: these bytes, alone, abort a commit.
        for c in [' ', '"', '\'', ',', ':', ';', '<', '>', '\\', '\t', '\n'] {
            assert!(is_git_crud(c), "{c:?} should be crud");
        }
        for c in ['.', '-', '_', '@', '(', 'a', '🐝'] {
            assert!(!is_git_crud(c), "{c:?} should not be crud");
        }
    }
}
