//! Harness-owned, ephemeral Git identity, credentials and signing for every runtime.
use nostr::ToBech32;
use std::path::Path;
use tempfile::TempDir;
use zeroize::Zeroizing;

/// Keep this guard alive until the adapter pool has shut down.
pub(crate) struct GitEnvironment {
    _dir: TempDir,
    pub(crate) env: Vec<(String, String)>,
}

impl GitEnvironment {
    pub(crate) fn for_config(config: &mut crate::config::Config) -> anyhow::Result<Self> {
        let environment =
            Self::install(&config.keys, &config.relay_url, &std::env::current_exe()?)?;
        config
            .persona_env_vars
            .retain(|(name, _)| !is_managed_env(name));
        config
            .persona_env_vars
            .extend(environment.env.iter().cloned());
        Ok(environment)
    }

    pub(crate) fn install(
        keys: &nostr::Keys,
        relay_url: &str,
        executable: &Path,
    ) -> anyhow::Result<Self> {
        // Read once, here: the wrapper never consults it, so changing
        // `BUZZ_GIT_IDENTITY` mid-session cannot change the mode. An
        // unrecognized value aborts startup rather than silently picking a mode.
        let mode = buzz_git_identity::GitIdentityMode::from_env().map_err(anyhow::Error::msg)?;
        let agent = mode == buzz_git_identity::GitIdentityMode::Agent;
        let dir = tempfile::Builder::new().prefix("buzz-acp-git-").tempdir()?;
        set_owner_only(dir.path())?;
        symlink(executable, &dir.path().join("git-credential-nostr"))?;
        if agent {
            symlink(executable, &dir.path().join("git-sign-nostr"))?;
        }
        let keyfile = dir.path().join(".nostr-key");
        let secret = Zeroizing::new(keys.secret_key().to_secret_hex());
        write_keyfile_atomic(&keyfile, secret.as_bytes())?;
        let info = KeyInfo {
            keyfile_path: keyfile
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Git keyfile path is not UTF-8"))?
                .to_owned(),
            pubkey_hex: keys.public_key().to_hex(),
            npub: keys.public_key().to_bech32()?,
        };
        let mut relay = url::Url::parse(relay_url)?;
        let scheme = match relay.scheme() {
            "wss" | "https" => "https",
            "ws" | "http" => "http",
            _ => anyhow::bail!("unsupported Git relay URL scheme"),
        };
        relay
            .set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid Git relay URL"))?;
        relay.set_query(None);
        relay.set_fragment(None);
        let relay = relay.as_str().trim_end_matches('/');
        // `user` mode keeps only relay auth; its commits carry the operator's
        // own identity, so no attribution, signing or wrapper is installed.
        let managed = if agent {
            let display_name = std::env::var("BUZZ_ACP_DISPLAY_NAME").ok();
            identity_entries(&info, relay, display_name.as_deref())
        } else {
            vec![("nostr.keyfile".into(), info.keyfile_path.clone())]
        };
        // The enforcement wrapper re-applies the manifest before exec and
        // verifies pushes against it; the config block below is built from the
        // same entries, so the two cannot disagree.
        #[cfg(unix)]
        if agent {
            symlink(executable, &dir.path().join("git"))?;
            buzz_git_identity::write_identity_manifest(dir.path(), &managed)?;
        }
        // In `user` mode nothing later overrides inherited identity or signing,
        // so drop agent mode's own managed keys, the author/committer
        // overrides and includes.
        let mut inherited = inherited_config()?;
        if !agent {
            let managed_keys = identity_entries(&info, relay, None);
            inherited.retain(|(key, _)| {
                !is_config_include(key)
                    && !AUTHOR_COMMITTER_KEYS
                        .iter()
                        .any(|identity| same_config_key(identity, key))
                    && !managed_keys
                        .iter()
                        .any(|(managed, _)| same_config_key(managed, key))
            });
        }
        let mut env = build_git_env(relay, &managed, inherited);
        // A `user`-mode child must not resolve `git` to a parent harness's
        // wrapper, whose install dir carries the identity manifest.
        let mut paths = vec![dir.path().to_path_buf()];
        paths.extend(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).filter(|entry| {
                agent
                    || entry
                        .join(buzz_git_identity::IDENTITY_MANIFEST_NAME)
                        .symlink_metadata()
                        .is_err()
            }),
        );
        env.push((
            "PATH".into(),
            std::env::join_paths(paths)?
                .into_string()
                .map_err(|_| anyhow::anyhow!("Git PATH is not UTF-8"))?,
        ));
        env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
        // CLI flags must work even when the caller did not export these variables.
        env.push(("BUZZ_PRIVATE_KEY".into(), secret.to_string()));
        env.push(("BUZZ_RELAY_URL".into(), relay_url.to_owned()));
        Ok(Self { _dir: dir, env })
    }
}

struct KeyInfo {
    keyfile_path: String,
    pubkey_hex: String,
    npub: String,
}

fn inherited_config() -> anyhow::Result<Vec<(String, String)>> {
    let count = match std::env::var("GIT_CONFIG_COUNT") {
        Ok(value) => value.parse::<usize>()?,
        Err(std::env::VarError::NotPresent) => 0,
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(count <= 1024, "too many inherited Git config entries");
    (0..count)
        .map(|i| {
            Ok((
                std::env::var(format!("GIT_CONFIG_KEY_{i}"))?,
                std::env::var(format!("GIT_CONFIG_VALUE_{i}"))?,
            ))
        })
        .collect()
}

/// Per-role identity keys that override `user.*`; not agent-managed entries.
const AUTHOR_COMMITTER_KEYS: [&str; 4] = [
    "author.name",
    "author.email",
    "committer.name",
    "committer.email",
];

/// Split a Git config key into section, optional subsection and variable. The
/// subsection is everything between the first and last dot, so it may itself
/// contain dots.
fn split_config_key(key: &str) -> Option<(&str, Option<&str>, &str)> {
    let (section, rest) = key.split_once('.')?;
    Some(match rest.rsplit_once('.') {
        Some((subsection, variable)) => (section, Some(subsection), variable),
        None => (section, None, rest),
    })
}

/// Git's key equality: section and variable ignore ASCII case, the subsection
/// does not.
fn same_config_key(a: &str, b: &str) -> bool {
    match (split_config_key(a), split_config_key(b)) {
        (Some((a_section, a_sub, a_var)), Some((b_section, b_sub, b_var))) => {
            a_section.eq_ignore_ascii_case(b_section)
                && a_sub == b_sub
                && a_var.eq_ignore_ascii_case(b_var)
        }
        _ => false,
    }
}

/// `include.path` or `includeIf.<condition>.path`: an included file could set
/// any identity key.
fn is_config_include(key: &str) -> bool {
    match split_config_key(key) {
        Some((section, None, variable)) => {
            section.eq_ignore_ascii_case("include") && variable.eq_ignore_ascii_case("path")
        }
        Some((section, Some(_), variable)) => {
            section.eq_ignore_ascii_case("includeIf") && variable.eq_ignore_ascii_case("path")
        }
        None => false,
    }
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

/// Stable identity contract for git attribution: the bare agent display name,
/// never channel-qualified, safe to embed in commit history.
///
/// Deliberately distinct from `BUZZ_ACP_SESSION_TITLE`, which is per-session UI
/// chrome and may be composed (`Agent · #channel`) by consumers. Commits
/// outlive sessions, so git attribution must not follow a mutable title.
///
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

/// The agent's attribution and signing config, which is also the enforcement
/// wrapper's manifest. The fixed signing values come from the contract the
/// wrapper validates, so the written and checked sets share one definition.
fn identity_entries(
    info: &KeyInfo,
    relay: &str,
    display_name: Option<&str>,
) -> Vec<(String, String)> {
    let host = url::Url::parse(relay)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .filter(|host| !host.starts_with("localhost") && !host.starts_with("127."))
        .unwrap_or_else(|| "buzz".into());
    let mut entries = vec![
        (
            "user.name".into(),
            display_name
                .and_then(sanitize_git_user_name)
                .unwrap_or_else(|| info.npub.clone()),
        ),
        ("user.email".into(), format!("{}@{host}", info.pubkey_hex)),
    ];
    entries.extend(
        buzz_git_identity::FIXED_SIGNING_ENTRIES
            .iter()
            .map(|&(key, value)| (key.into(), value.into())),
    );
    entries.extend([
        ("user.signingkey".into(), info.pubkey_hex.clone()),
        ("nostr.keyfile".into(), info.keyfile_path.clone()),
    ]);
    entries
}

/// Compose a complete config block, including the caller's entries, for env-cleared MCP children.
fn build_git_env(
    relay: &str,
    managed: &[(String, String)],
    mut entries: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let scope = format!("credential.{relay}/git");
    entries.extend([
        // Reset helpers only inside this relay's Git URL namespace. Unrelated
        // remotes retain their own helper chain and never receive this key.
        (format!("{scope}.helper"), String::new()),
        (format!("{scope}.helper"), "nostr".into()),
        (format!("{scope}.useHttpPath"), "true".into()),
    ]);
    entries.extend_from_slice(managed);
    let mut env = vec![("GIT_CONFIG_COUNT".into(), entries.len().to_string())];
    for (i, (key, value)) in entries.into_iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{i}"), key));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), value));
    }
    env
}

/// Variables owned by the harness and forwarded across the MCP env-clear boundary.
pub(crate) fn is_managed_env(name: &str) -> bool {
    name.starts_with("GIT_CONFIG_")
        || matches!(
            name,
            "PATH" | "GIT_TERMINAL_PROMPT" | "BUZZ_PRIVATE_KEY" | "BUZZ_RELAY_URL"
        )
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

#[cfg(test)]
mod git_user_name_tests {
    use super::{
        build_git_env, identity_entries, is_git_crud, is_unicode_format, sanitize_git_user_name,
        KeyInfo, MAX_GIT_USER_NAME_CHARS,
    };

    const PUBKEY_HEX: &str = "dcfd242e557282d7a1e2cf2e6877522682f1e5c6156dc92ca7d90eaedd3b0f95";
    const NPUB: &str = "npub1mn7jgtj4w2pd0g0zeuhxsa6jy6p0rewxz4kujt98my82ahfmp72sxjexk7";

    fn key_info() -> KeyInfo {
        KeyInfo {
            keyfile_path: "/tmp/.nostr-key".into(),
            pubkey_hex: PUBKEY_HEX.into(),
            npub: NPUB.into(),
        }
    }

    fn git_env(display_name: Option<&str>) -> Vec<(String, String)> {
        let relay = "https://localhost:3000";
        build_git_env(
            relay,
            &identity_entries(&key_info(), relay, display_name),
            vec![],
        )
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
        let env = git_env(Some("Duncan"));

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
        let env = git_env(None);

        // Without a display name, attribution falls back to the npub.
        assert_eq!(git_config(&env, "user.name").as_deref(), Some(NPUB));
        assert_eq!(
            git_config(&env, "user.email").as_deref(),
            Some(format!("{PUBKEY_HEX}@buzz").as_str())
        );
    }

    #[test]
    fn test_build_git_env_falls_back_to_npub_when_display_name_is_unusable() {
        // Crud-only and format-only names both reach git as the npub — one
        // would abort every commit, the other would render as blank.
        for raw in ["<>", "\u{200B}"] {
            let env = git_env(Some(raw));
            assert_eq!(
                git_config(&env, "user.name").as_deref(),
                Some(NPUB),
                "unusable display name {raw:?} must reach git as the npub"
            );
        }
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
