//! Enforcement `git` wrapper — the L2/L3 half of deterministic agent identity.
//!
//! Installed on PATH (shim dir and the harness's agent-runtime PATH) as `git`,
//! ahead of the real binary. Every `git` an agent's shell runs lands here first.
//! The wrapper:
//!
//! 1. **Scrubs** `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` and the committer pair from
//!    the child env — the env-var identity-override vector.
//! 2. **Rejects loudly** the flag-based override vectors: `-c user.name=`/
//!    `-c user.email=` (and `--config-env` for the same keys) in global position,
//!    and `--author`/`--reset-author` on `commit`/`am`.
//! 3. On `push`, **verifies** that every outgoing commit not already on a remote
//!    is authored by the agent identity and carries a valid NIP-GS signature by
//!    the agent key. This closes unsigned agent commits from
//!    `merge`/`pull`/`commit-tree`/plumbing that the flag-based `enforce`
//!    cannot reject. Fails the push otherwise.
//! 4. Execs the real `git` (found by skipping PATH entries that resolve back to
//!    this binary), so nothing the agent can pass reaches git with a spoofed
//!    identity on the default path.
//!
//! Exit codes: `1` for a rejected override, `1` for a failed push verification,
//! `127` when the real git cannot be found. Otherwise the real git's own status.

use std::path::{Path, PathBuf};

/// Author/committer identity env vars the agent's shell must not use to override
/// the configured Buzz identity. DATE is intentionally left alone — it carries
/// no attribution signal and rebase/cherry-pick rely on it internally.
const SCRUBBED_ENV: &[&str] = &[
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
];

/// Maximum number of ordinary alias substitutions the managed wrapper resolves.
/// The next command token is always inspected too, so a chain with exactly this
/// many aliases may terminate at a real subcommand; a further alias is refused.
const MAX_ALIAS_HOPS: usize = 10;

/// Long global options that consume the *following* argv token as their value
/// (the `--opt value` form; the `--opt=value` form is self-contained). This
/// table is load-bearing for the whole enforcement design: [`split_globals`] is
/// the single point of truth for where the subcommand begins, and every alias
/// and push-verification probe resolves under the globals it extracts. Any
/// separate-value global git honors but this table omits desyncs the probe from
/// the real invocation and reopens an enforcement bypass (round-7 `--shallow-file`).
///
/// Must match git's `handle_options()` exactly. Audited against
/// [git.c v2.54.0](https://github.com/git/git/blob/v2.54.0/git.c#L233-L370):
/// the complete set of separate-value globals is `--git-dir`, `--work-tree`,
/// `--namespace`, `--config-env`, `--attr-source`, and `--shallow-file`.
/// `--super-prefix` is retained though v2.54 rejects it globally (over-enumeration
/// is inert: it only ever pairs a value git would itself refuse). Re-audit this
/// table when bumping the pinned git version.
const VALUE_LONG_OPTS: &[&str] = &[
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--config-env",
    "--attr-source",
    "--shallow-file",
];

/// The harness-owned identity authority: the identity/signing config the
/// wrapper re-applies before exec, and the agent author email push
/// verification checks against. Read from the 0600 manifest the harness/shim
/// wrote beside the keyfile — never from the caller-mutable `GIT_CONFIG_*`
/// environment the wrapper exists to constrain.
struct Authority {
    /// Ordered identity + signing `(key, value)` git config entries.
    entries: Vec<(String, String)>,
    /// The expected commit author email (`<64-hex>@host`).
    email: String,
}

/// Result of locating the wrapper's identity authority.
enum AuthorityState {
    /// The wrapper was not reached through an install symlink on `PATH`, so its
    /// own dir (and any manifest) cannot be located. This is the accepted local
    /// ceiling — e.g. the real `git` invoked by absolute path — so there is no
    /// authority to enforce against: passthrough.
    Unmanaged,
    /// The install dir is located and holds a complete, self-consistent
    /// manifest: enforce (author identity AND a signature by the agent key).
    Managed(Authority),
    /// The install dir is located but its manifest is missing, unreadable, or
    /// does not carry the complete managed signing contract (see
    /// [`Authority::classify`]). A managed install always writes the full
    /// contract via [`crate::identity_signing_entries`], and `user` mode
    /// installs no manifest at all — so an incomplete or inconsistent manifest
    /// means the authority was removed, corrupted, or tampered after install.
    /// Fail closed rather than silently drop or misdirect enforcement.
    Tampered,
}

impl Authority {
    /// Locate and classify the wrapper's identity authority. Distinguishing a
    /// genuinely unmanaged wrapper (no install dir on `PATH`) from a located
    /// install dir whose manifest was removed/corrupted is load-bearing: the
    /// former passes through (accepted ceiling), the latter fails closed.
    fn load() -> AuthorityState {
        let Some(dir) = locate_install_dir() else {
            return AuthorityState::Unmanaged;
        };
        let Some(entries) = crate::read_identity_manifest(&dir) else {
            return AuthorityState::Tampered; // manifest missing/unreadable
        };
        Self::classify(entries)
    }

    /// Validate a parsed manifest against the complete managed signing contract,
    /// returning [`AuthorityState::Managed`] only when every part holds and
    /// [`AuthorityState::Tampered`] otherwise. Pure over its `entries` input so
    /// it is testable without `PATH`/filesystem.
    ///
    /// A managed install is written solely by [`crate::identity_signing_entries`]
    /// (agent mode); `user` mode writes no manifest. So a valid manifest carries
    /// EXACTLY the eight canonical keys that function writes, each once, with the
    /// fixed values ([`crate::FIXED_SIGNING_ENTRIES`]) and a `user.signingkey`
    /// equal to the pubkey encoded in `user.email`. Any deviation is tampering
    /// and fails closed:
    ///
    /// - Dropping/falsifying `commit.gpgSign` would leave the signature gate off.
    /// - Swapping `user.signingkey` to another key would make the push probe
    ///   accept a valid signature by the *wrong* key while the commit still
    ///   appears authored as the agent.
    /// - Because git config is last-value-wins and the accepted entries are
    ///   injected verbatim as `-c` into every commit and the signature probe, a
    ///   *duplicate* later `user.signingkey`, or any *unknown* key (e.g. an
    ///   `include.path` that pulls in another key file), could redirect the key
    ///   the probe trusts. Rejecting duplicate and unknown keys — and rebuilding
    ///   the authority's entries solely from the validated canonical fields, so
    ///   nothing unvalidated crosses into git — closes that class by
    ///   construction.
    fn classify(entries: Vec<(String, String)>) -> AuthorityState {
        // Collect each canonical key's single value, rejecting duplicates and
        // unknown keys. `CANONICAL_KEYS` mirrors `identity_signing_entries`.
        const CANONICAL_KEYS: &[&str] = &[
            "user.name",
            "user.email",
            "gpg.format",
            "gpg.x509.program",
            "commit.gpgSign",
            "tag.gpgSign",
            "user.signingkey",
            "nostr.keyfile",
        ];
        let mut seen: Vec<Option<String>> = vec![None; CANONICAL_KEYS.len()];
        for (key, value) in entries {
            let Some(idx) = CANONICAL_KEYS
                .iter()
                .position(|k| key.eq_ignore_ascii_case(k))
            else {
                return AuthorityState::Tampered; // unknown key
            };
            if seen[idx].replace(value).is_some() {
                return AuthorityState::Tampered; // duplicate key
            }
        }
        let get = |name: &str| {
            let idx = CANONICAL_KEYS.iter().position(|k| *k == name).unwrap();
            seen[idx].as_deref()
        };

        // Author identity must be present and non-empty.
        let Some(email) = get("user.email")
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
        else {
            return AuthorityState::Tampered;
        };
        if get("user.name").is_none_or(str::is_empty) {
            return AuthorityState::Tampered;
        }
        // The fixed signing entries must carry their canonical values byte for
        // byte. `gpg.x509.program` pins the verifier the push probe invokes, and
        // the installers emit exactly one canonical spelling, so any case
        // variance is tampering: on a case-sensitive host `GIT-SIGN-NOSTR`
        // resolves past the managed install to an agent-controlled binary later
        // on PATH. Comparing verbatim keeps the probe bound to the real signer.
        let fixed_ok = crate::FIXED_SIGNING_ENTRIES
            .iter()
            .all(|&(k, expected)| get(k) == Some(expected));
        if !fixed_ok || get("nostr.keyfile").is_none_or(str::is_empty) {
            return AuthorityState::Tampered;
        }
        // `user.signingkey` must be the pubkey the author email encodes
        // (`<pubkey_hex>@<host>`), so the key the probe trusts is the same key
        // the commit is attributed to.
        let email_pubkey = email.split('@').next().unwrap_or("");
        if email_pubkey.is_empty()
            || get("user.signingkey").is_none_or(|k| !k.eq_ignore_ascii_case(email_pubkey))
        {
            return AuthorityState::Tampered;
        }

        // Rebuild the entries from the validated canonical fields in the order
        // `identity_signing_entries` writes them — nothing unvalidated (a
        // duplicate, an unknown redirect key) crosses into the `-c` injection.
        let canonical: Vec<(String, String)> = CANONICAL_KEYS
            .iter()
            .map(|k| ((*k).to_owned(), get(k).unwrap_or_default().to_owned()))
            .collect();
        AuthorityState::Managed(Self {
            entries: canonical,
            email,
        })
    }
}

/// Entry point for the `git` multicall personality. Never returns on success
/// (execs real git on Unix); returns the process exit code on Windows/error.
pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // Classify the authority. `Tampered` (install dir located but manifest
    // missing/unreadable or not carrying the complete, self-consistent signing
    // contract) fails closed for every command: a managed install always writes
    // the full contract and `user` mode writes no manifest, so anything weaker
    // means the authority was removed, damaged, or tampered after install, and
    // continuing would silently drop or misdirect enforcement.
    let authority = match Authority::load() {
        AuthorityState::Managed(a) => Some(a),
        AuthorityState::Unmanaged => None,
        AuthorityState::Tampered => {
            eprintln!(
                "buzz git wrapper: refusing to run — this `git` is a managed enforcement \
                 wrapper but its identity manifest is missing, unreadable, or does not carry \
                 the complete signing contract (a valid manifest names the agent identity and \
                 a matching signing key). Enforcement fails closed rather than fall back to an \
                 ambient identity or trust the wrong signing key."
            );
            return 1;
        }
    };

    run_inner(argv, authority)
}

/// Inner dispatch — separated so tests can inject a controlled authority
/// without touching the filesystem manifest.  `run()` is the public entry
/// point; tests call `run_inner` directly.
fn run_inner(argv: Vec<String>, authority: Option<Authority>) -> i32 {
    if let Err(msg) = enforce(&argv, authority.as_ref()) {
        eprintln!("{msg}");
        return 1;
    }

    let real_git = match find_real_git() {
        Some(p) => p,
        None => {
            eprintln!(
                "buzz git wrapper: could not locate the real `git` binary on PATH. \
                 Agent git identity enforcement is active but git is not installed."
            );
            return 127;
        }
    };

    let ctx = caller_globals(&argv);

    // Alias preflight + unification. `verify_alias_safety` refuses every shell
    // (`!`) alias and every non-shell alias carrying config/quoting the wrapper
    // cannot classify; on success it returns the alias's fully-resolved bare-word
    // expansion (or `None` when no alias was involved). We then hold that
    // expansion to the SAME `enforce`/`verify_commit_author` policy as a directly
    // typed command — keyed on the *expanded* subcommand — so an alias can never
    // do more than its expansion could. `enforce`/`verify_commit_author` on the
    // literal argv below cannot catch alias-carried flags (`git human` expands to
    // `commit --author …`, but the literal subcommand is `human`); the expanded
    // preflight closes that gap by construction, with no alias-specific flag list.
    let mut alias_expanded: Option<Vec<String>> = None;
    if let Some(auth) = &authority {
        match verify_alias_safety(&real_git, &argv, &ctx) {
            Ok(None) => {}
            Ok(Some(expanded)) => {
                if let Err(msg) = enforce(&expanded, Some(auth)) {
                    eprintln!("{msg}");
                    return 1;
                }
                if let Err(msg) = verify_commit_author(&real_git, &expanded, &ctx, auth) {
                    eprintln!("{msg}");
                    return 1;
                }
                alias_expanded = Some(expanded);
            }
            Err(msg) => {
                eprintln!("{msg}");
                return 1;
            }
        }
    }

    // Author preflight (E): commit modes that reuse or preserve another author
    // (`-C`/`-c <sha>`, `--amend`) create NEW commits stamped with that author.
    // Re-applied identity config cannot fix this — git honours the reused
    // author — so reject when the resulting author would not be the agent. This
    // covers a directly-typed `commit`; an alias resolving to one is covered by
    // the expanded-command preflight above.
    if let Some(auth) = &authority {
        if let Err(msg) = verify_commit_author(&real_git, &argv, &ctx, auth) {
            eprintln!("{msg}");
            return 1;
        }
    }

    // Push verification (L3) runs before exec so a wrongly-authored commit
    // cannot leave the machine. The effective command is resolved through git
    // aliases (config-defined and inline `-c alias.*`), because `git pub` with
    // `alias.pub = push` reaches the real push after we hand off — keying on the
    // literal token alone would let an alias slip a wrong-authored commit past.
    // The alias-expanded argv (when an alias was involved) is used for the
    // receive-pack guard so a bare-word alias like `alias.p = push --exec evil`
    // does not bypass the flag scan. When no alias was involved, `effective_argv`
    // is just the original argv.
    if let Some(auth) = &authority {
        let effective_argv = alias_expanded.as_deref().unwrap_or(&argv);
        // Classify using the already-validated expansion, not the original argv.
        // `is_push_command` with the original argv re-resolves alias chains from
        // scratch: for `alias.pub = -p push` it takes `-p` as the command word
        // (the first non-global token in the alias body), looks up `alias.-p`
        // (no result), and returns `NotPush` — bypassing push verification
        // entirely.  `effective_argv` is the fully-expanded, safety-checked argv
        // whose subcommand is the real git subcommand (`push` in this case), so
        // classifying it produces the correct result without another alias walk.
        match is_push_command(&real_git, effective_argv, &ctx) {
            PushKind::NotPush => {}
            PushKind::Push => {
                if let Err(msg) =
                    verify_push(&real_git, argv.as_slice(), effective_argv, &ctx, auth)
                {
                    eprintln!("{msg}");
                    return 1;
                }
            }
        }
    }

    exec_real_git(&real_git, &argv, authority.as_ref())
}

/// The wrapper's own install dir: the first `PATH` entry whose `git` resolves
/// (through the install symlink) back to this binary. That dir holds the 0600
/// identity manifest and keyfile. Located by canonicalization — the same
/// env-independent trust channel as [`find_real_git`] — so an agent cannot
/// point the wrapper at a forged authority by rewriting environment variables.
fn locate_install_dir() -> Option<PathBuf> {
    let self_canon = std::env::current_exe().ok()?.canonicalize().ok()?;
    let git_name = if cfg!(windows) { "git.exe" } else { "git" };
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join(git_name);
        if candidate.canonicalize().ok().as_ref() == Some(&self_canon) {
            return Some(dir);
        }
    }
    None
}

/// Every global option the caller placed before the subcommand — the complete
/// set git itself consumes before dispatching to the subcommand. This is the
/// probe context for *every* verification probe: alias resolution
/// ([`verify_alias_safety`], [`is_push_command`]) and the outgoing-commit
/// authorship/signature checks ([`partition_outgoing`] via [`rev_list_outgoing`],
/// [`commit_author_email`], [`commit_signature_is_agent`]). Each probe MUST run
/// under the same effective repository and configuration git will use for the
/// real invocation, or it resolves a different view and enforcement is bypassed.
///
/// INVARIANT — pass the caller's globals through wholesale; never an allowlist.
/// This boundary was reopened three times by enumerating "known" context
/// globals and missing one: repo-config keys (round 4), the config-injection
/// channels `-c`/`--config-env`/`include.path` (round 6), then `--bare` — which
/// changes repository discovery and therefore *which* config file supplies an
/// alias, so an allowlist that dropped it let `git --bare <dir> x` expand an
/// alias the probe never saw. The complete-set rule closes the whole class:
///   • Any global that would corrupt a probe corrupts the real invocation
///     identically (git parses globals before subcommand dispatch), so probe
///     and real git always share one view and fail closed together — no probe
///     can go blind (fail-open) while real git still expands an alias.
///   • Pager/cosmetic globals (`-p`, `--paginate`, `--no-optional-locks`, …)
///     are inert: probes capture piped stdout, so git auto-disables the pager,
///     and none introduce config or aliases.
///   • `--exec-path=` is refused by [`inspect_push_config`] (Surface 1) before
///     any probe runs, so it cannot reach the probe stage at all.
/// The authoritative identity/signing `-c` entries always splice *after* these
/// caller globals ([`inject_identity_args`], [`commit_signature_is_agent`]), so
/// command-line last-wins precedence keeps a caller `-c` from overriding them.
fn caller_globals(argv: &[String]) -> Vec<String> {
    split_globals(argv).0
}

/// The effective-command classification of an invocation.
enum PushKind {
    /// The effective command is not `push`.
    NotPush,
    /// The effective command resolves to `push` through ordinary (config/inline)
    /// git aliases; its transport plan can be resolved safely with `--dry-run`.
    Push,
}

/// Classify the invocation's *effective* command, resolving ordinary git
/// aliases so `git pub` (with `alias.pub = push`) and `git -c alias.pub=push
/// pub` are both recognized. Aliases are read under `ctx` — the caller's
/// complete global set ([`caller_globals`]) — so the probe consults the exact
/// same aliases git will, including those introduced by a `-c include.path`/
/// `--config-env`, case-varied `-c ALIAS.x`, or a repo view selected by
/// `--bare`/`--git-dir`.
///
/// This runs only in a managed session, *after* [`verify_alias_safety`] has
/// already refused every shell (`!`) alias and every non-shell alias that is
/// not a trivially-safe bare-word chain — so a shell alias never reaches here.
/// Recursion is bounded to defeat cyclic alias definitions.
fn is_push_command(real_git: &Path, argv: &[String], ctx: &[String]) -> PushKind {
    let mut name = match subcommand(argv) {
        Some(s) => s,
        None => return PushKind::NotPush,
    };
    for _ in 0..10 {
        if name == "push" {
            return PushKind::Push;
        }
        let def = capture(
            real_git,
            ctx,
            &["config", "--get", &format!("alias.{name}")],
        );
        let def = match def {
            Some(d) => d,
            None => return PushKind::NotPush, // not an alias — effective command
        };
        if def.starts_with('!') {
            return PushKind::NotPush; // shell alias — already refused upstream
        }
        match def.split_whitespace().next() {
            Some(first) => name = first.to_string(),
            None => return PushKind::NotPush,
        }
    }
    PushKind::NotPush
}

/// Query git for its set of builtin subcommands via `--list-cmds=builtins`.
///
/// By default, git dispatches builtin commands BEFORE consulting `alias.*`
/// config — a name that matches a builtin is never treated as an alias.
/// However, on binaries where `--list-cmds=deprecated` succeeds, deprecated
/// builtins (`whatchanged`, `pack-redundant`) are alias-first: `run_argv()`
/// calls `handle_alias()` before `handle_builtin()` for those names.
///
/// The expansion loop in [`verify_alias_safety`] must mirror this behaviour
/// exactly: break on a builtin that is NOT deprecated (builtin-first), but
/// continue expanding aliases on a deprecated builtin (alias-first).  When git
/// does not support `--list-cmds=deprecated` (where the call exits non-zero),
/// the deprecated set is empty and ALL builtins are treated as builtin-first,
/// which is correct for those binaries.
///
/// Returns an empty set if the flag is unavailable; callers treat an empty set
/// conservatively (no builtin-precedence short-circuit — falls back to alias
/// lookup for every name).
fn git_builtin_commands(real_git: &Path) -> std::collections::HashSet<String> {
    let out = match capture_raw(real_git, &["--list-cmds=builtins"]) {
        Some(o) if o.status.success() => o.stdout,
        _ => return std::collections::HashSet::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Query git for its set of deprecated subcommands via `--list-cmds=deprecated`.
///
/// On binaries where this query succeeds, deprecated builtins (`whatchanged`,
/// `pack-redundant`) are handled alias-first: `run_argv()` calls
/// `handle_alias()` before `handle_builtin()` for those names, so an
/// `alias.<name>` setting IS expanded even though the name appears in
/// `--list-cmds=builtins`.
///
/// Returns an empty set if the flag is unsupported (the query exits non-zero);
/// callers use the empty set to infer that ALL builtins are builtin-first on
/// that binary (the alias-first exception does not apply).
fn git_deprecated_commands(real_git: &Path) -> std::collections::HashSet<String> {
    let out = match capture_raw(real_git, &["--list-cmds=deprecated"]) {
        Some(o) if o.status.success() => o.stdout,
        _ => return std::collections::HashSet::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Refuse any alias the wrapper cannot *trivially* prove safe, and — on success
/// — return the alias's fully-resolved expansion so the caller can hold it to
/// the same policy as a directly-typed command. Git expands an alias in-process,
/// and its config-bearing globals land *after* the identity/signing `-c` options
/// this wrapper injects before the subcommand ([`inject_identity_args`]), so an
/// alias could otherwise plant higher-precedence config that re-authors or
/// unsigns the commit. It could equally carry identity/signing *flags*
/// (`--author`, `--no-gpg-sign`, `--amend`, commit reuse). [`enforce`] and
/// [`verify_commit_author`] cannot catch either on the literal argv: they key on
/// the typed subcommand, which for `git human` is the alias name `human`, never
/// the expanded `commit`.
///
/// This is an allowlist, not a blocklist. Rather than model git's alias grammar
/// (whose quote-aware parser would let `'-c' 'user.email=…'` slip past a
/// naive token scan), it admits an alias only when every token of its body is a
/// trivially-safe bare word: no quote or backslash characters, no `-c`/
/// `--config-env` config channel, and no `=`-valued option. Anything the
/// wrapper cannot classify at a glance is refused (favor-rejection).
///
/// The returned expansion is exactly the token list git would run — the typed
/// globals, then the recursively-expanded command with accumulated body tokens
/// and the user's trailing argv. Holding it to [`enforce`]/[`verify_commit_author`]
/// keyed on the *expanded* subcommand means an alias can never do more than its
/// expansion could typed directly, so no alias-specific flag list exists to keep
/// in sync. `Ok(None)` means the typed subcommand was a real command (no alias).
///
/// Shell (`!`) aliases are refused outright in a managed session. Git runs an
/// `!` alias with its own exec-path prepended to `PATH`, so the inner `git` is
/// the *real* binary, not this wrapper — its `-c` outranks the inherited env
/// authority and can commit as an arbitrary human, unsigned. Their bodies are
/// arbitrary shell, so there is no safe subset to allow.
///
/// Allowed shapes stay working: `alias.ci = commit`, `alias.st = status`,
/// `alias.lg = log --oneline`, `alias.pub = push origin main`. Recursion is
/// bounded to defeat cyclic alias definitions. After the bound is reached, the
/// next command token must be a real subcommand or the wrapper refuses rather
/// than treating a partial expansion as resolved.
fn verify_alias_safety(
    real_git: &Path,
    argv: &[String],
    ctx: &[String],
) -> Result<Option<Vec<String>>, String> {
    let (_, sub_idx) = split_globals(argv);
    let Some(sub_idx) = sub_idx else {
        return Ok(None); // no subcommand — nothing to expand
    };
    // The command chain being expanded: the subcommand token and its trailing
    // argv. Typed globals (argv[..sub_idx]) are prepended to the final result.
    let mut chain: Vec<String> = argv[sub_idx..].to_vec();
    let mut resolved_any = false;
    // By default, ALL builtins are dispatched before alias config — an
    // `alias.<builtin>` entry is silently ignored.
    // On binaries where `--list-cmds=deprecated` succeeds, deprecated builtins
    // (`whatchanged`, `pack-redundant`) are alias-FIRST; `run_argv()` calls
    // `handle_alias()` before `handle_builtin()` for those.  We model this by
    // treating a name as a non-builtin (continuing alias lookup) when it is in
    // BOTH the builtins and the deprecated lists.
    // When `--list-cmds=deprecated` exits non-zero, `deprecated` is empty and we
    // apply builtin-first for every builtin — correct for that binary.
    let builtins = git_builtin_commands(real_git);
    let deprecated = git_deprecated_commands(real_git);
    for _ in 0..MAX_ALIAS_HOPS {
        // The command word within the current chain (git re-parses leading
        // options as globals after each expansion, so a body may begin with
        // bare-word options before its subcommand).
        let Some(cmd_idx) = split_globals(&chain).1 else {
            break; // no command word left
        };
        let name = &chain[cmd_idx];
        // A non-deprecated builtin is dispatched before alias config — break.
        // A deprecated builtin on a binary where --list-cmds=deprecated succeeds
        // is alias-first — fall through to the alias lookup below so the
        // wrapper's expansion matches that binary's real dispatch order.
        let is_nondeprecated_builtin = !builtins.is_empty()
            && builtins.contains(name.as_str())
            && !deprecated.contains(name.as_str());
        if is_nondeprecated_builtin {
            break;
        }
        let def = capture(
            real_git,
            ctx,
            &["config", "--get", &format!("alias.{name}")],
        );
        let Some(def) = def else {
            break; // real subcommand — chain is fully expanded
        };
        if def.starts_with('!') {
            return Err(shell_alias_reject_message(name));
        }
        let body: Vec<String> = def.split_whitespace().map(String::from).collect();
        if !body.iter().all(|t| is_safe_alias_token(t)) {
            return Err(alias_reject_message(name));
        }
        // Substitute the command word with its body, exactly as git does.
        chain.splice(cmd_idx..=cmd_idx, body);
        resolved_any = true;
    }
    let Some(cmd_idx) = split_globals(&chain).1 else {
        return Err(alias_limit_reject_message());
    };
    let name = &chain[cmd_idx];
    // A non-deprecated builtin at the end of the chain cannot be a further alias
    // hop — treat it as fully resolved.  A deprecated builtin may still have an
    // alias; probe config to ensure the hop limit was not just exhausted.
    let is_nondeprecated_builtin = !builtins.is_empty()
        && builtins.contains(name.as_str())
        && !deprecated.contains(name.as_str());
    if !is_nondeprecated_builtin
        && capture(
            real_git,
            ctx,
            &["config", "--get", &format!("alias.{name}")],
        )
        .is_some()
    {
        return Err(alias_limit_reject_message());
    }
    if !resolved_any {
        return Ok(None);
    }
    let mut expanded = argv[..sub_idx].to_vec();
    expanded.extend(chain);
    Ok(Some(expanded))
}

/// A single alias-body token is safe only when it is a plain bare word that
/// introduces no configuration and needs no shell/quote interpretation. This is
/// deliberately conservative: git's quote-aware alias parser sees a token
/// differently from our whitespace split, so any token carrying a quote or
/// escape is refused rather than guessed at.
fn is_safe_alias_token(token: &str) -> bool {
    // Quote/escape characters: git would dequote these, changing the token from
    // what we scanned. Refuse — the allowlist never reasons about quoted forms.
    if token.contains(['\'', '"', '\\']) {
        return false;
    }
    // The config-injection channels, in any spelling.
    if token == "-c"
        || token.starts_with("-c")
        || token == "--config-env"
        || token.starts_with("--config-env")
    {
        return false;
    }
    // Any other option carrying a `=` value (e.g. `--author=…`, `--foo=bar`)
    // can redirect identity/behaviour we cannot classify — refuse.
    if token.starts_with('-') && token.contains('=') {
        return false;
    }
    true
}

fn alias_reject_message(name: &str) -> String {
    format!(
        "buzz git wrapper: refusing `{name}` — this git alias contains tokens the managed \
         wrapper cannot verify as safe (quoting/escaping, `-c`/`--config-env`, or a \
         value-bearing option). Aliases that could carry configuration are refused because \
         git applies alias config after the managed agent identity and signing config. Run \
         the underlying git command directly; agent commit identity and signing are \
         machine-managed."
    )
}

fn alias_limit_reject_message() -> String {
    format!(
        "buzz git wrapper: refusing alias chain after {MAX_ALIAS_HOPS} expansions — the managed \
         wrapper only runs a command after proving its final command word is not another git alias. \
         Run the underlying git command directly; agent commit identity and signing are \
         machine-managed."
    )
}

fn shell_alias_reject_message(name: &str) -> String {
    format!(
        "buzz git wrapper: refusing `{name}` — it is a shell (`!`) git alias. Git runs `!` \
         aliases with the real git ahead of this wrapper on PATH, so their body can commit \
         or push under an arbitrary identity, unsigned. Run the underlying git command \
         directly; agent commit identity and signing are machine-managed."
    )
}

/// Reject the flag-based identity- and signing-override vectors. `Ok(())` means
/// the argv is clean and may proceed to the real git.
///
/// Only enforces in a managed session (`authority` present): an unmanaged
/// session has no injected identity or signing config to protect, so rejecting
/// `--no-gpg-sign` there would break ordinary use. The env-var forms of these
/// overrides (`GIT_CONFIG_*`) are defeated separately by re-applying the
/// authoritative config at the highest index before exec; this covers the
/// command-line forms, which win over env config and so must be refused.
fn enforce(argv: &[String], authority: Option<&Authority>) -> Result<(), String> {
    if authority.is_none() {
        return Ok(());
    }
    let (globals, sub_idx) = split_globals(argv);

    // Protected config keys set via `-c key=…`/`-ckey=…` or `--config-env=key=VAR`
    // in global position. `-c` only ever appears as a git *global* option, so
    // scanning globals both suffices and avoids misreading `git commit -c
    // <commit>` (reuse-message), where `-c` means something entirely different.
    for token in &globals {
        if let Some(key) = config_key_override(token) {
            return Err(reject_message(&format!("-c {key}=…")));
        }
        if let Some(key) = config_env_override(token) {
            return Err(reject_message(&format!("--config-env={key}=…")));
        }
    }

    // Subcommand-scoped identity/signing flags. `--author`/`--reset-author`
    // carry identity for `commit`/`am`; `--no-gpg-sign` disables the signing
    // the harness lifted. Scoping to the relevant subcommands is load-bearing:
    // `git log --author=…` is a legitimate read filter that must keep working.
    if let Some(sub) = sub_idx.map(|i| argv[i].as_str()) {
        let is_commit_or_am = sub == "commit" || sub == "am";
        let signs = matches!(
            sub,
            "commit" | "am" | "tag" | "rebase" | "cherry-pick" | "revert"
        );
        for token in &argv[sub_idx.unwrap() + 1..] {
            if is_commit_or_am
                && (token == "--author"
                    || token.starts_with("--author=")
                    || token == "--reset-author")
            {
                return Err(reject_message(token));
            }
            if signs && token == "--no-gpg-sign" {
                return Err(reject_message(token));
            }
        }
    }

    Ok(())
}

fn reject_message(what: &str) -> String {
    format!(
        "buzz git wrapper: refusing `{what}` — agent commit identity and signing are \
         machine-managed and cannot be overridden. Commits are automatically authored as your \
         agent identity (<pubkey>@<relay>) and signed. Credit the human operator with \
         `Co-authored-by`/`Signed-off-by` trailers instead."
    )
}

/// If `token` is a `-c <config>` value (attached `-cuser.email=x` or the bare
/// `user.email=x` that follows a standalone `-c`) setting a protected identity
/// or signing key, return the normalized key; else `None`.
fn config_key_override(token: &str) -> Option<&'static str> {
    // `-cuser.email=x` attached form, or the standalone value token that
    // `split_globals` already paired with a preceding `-c`.
    let cfg = token
        .strip_prefix("-c")
        .filter(|s| !s.is_empty())
        .unwrap_or(token);
    matches_protected_key(cfg)
}

/// If `token` is `--config-env=<key>=VAR` for a protected key, return it.
fn config_env_override(token: &str) -> Option<&'static str> {
    let rest = token.strip_prefix("--config-env=")?;
    matches_protected_key(rest)
}

/// Normalize a `name.subname[=value]` config spec and return the canonical key
/// when it names a protected identity or signing setting (case-insensitive).
/// These are exactly the keys [`crate::identity_signing_entries`] injects: an
/// agent must not be able to redirect authorship or disable/redirect signing.
fn matches_protected_key(cfg: &str) -> Option<&'static str> {
    let key = cfg.split('=').next().unwrap_or(cfg).to_ascii_lowercase();
    match key.as_str() {
        "user.name" => Some("user.name"),
        "user.email" => Some("user.email"),
        "user.signingkey" => Some("user.signingkey"),
        "commit.gpgsign" => Some("commit.gpgSign"),
        "tag.gpgsign" => Some("tag.gpgSign"),
        "gpg.format" => Some("gpg.format"),
        "gpg.x509.program" => Some("gpg.x509.program"),
        "nostr.keyfile" => Some("nostr.keyfile"),
        _ => None,
    }
}

/// Split argv into the global-option tokens (including `-c` values) and the
/// index of the subcommand token, if any. Walks the same value-consuming rules
/// git uses so the subcommand is located correctly.
fn split_globals(argv: &[String]) -> (Vec<String>, Option<usize>) {
    let mut globals = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if !arg.starts_with('-') {
            return (globals, Some(i)); // first non-option token = subcommand
        }
        globals.push(arg.clone());
        // `-c`/`-C` and the value-taking long options each consume the next
        // token as their value; pull it into globals so the subcommand scan
        // doesn't mistake a value for the subcommand.
        let takes_value = arg == "-c" || arg == "-C" || VALUE_LONG_OPTS.contains(&arg.as_str());
        if takes_value && i + 1 < argv.len() {
            i += 1;
            globals.push(argv[i].clone());
        }
        i += 1;
    }
    (globals, None)
}

/// The git subcommand (first non-option token), or `None` for a bare `git` /
/// `git --version`-style invocation.
fn subcommand(argv: &[String]) -> Option<String> {
    let (_, idx) = split_globals(argv);
    idx.map(|i| argv[i].clone())
}

/// Walk the outgoing commits for each source ref once, partitioning them into
/// `(offenders, agent_shas)`: non-agent-authored commits that are NOT a
/// replayed-upstream commit (attribution failures) and agent-authored commit
/// SHAs (whose signature the caller must still verify). Fails closed (`Err`) on
/// any probe error — an unverifiable commit must never be treated as clean.
///
/// Split out of [`verify_push`] so the authorship logic — including the
/// cherry-pick/rebase patch-id exemption — is unit-testable without a signer,
/// which the unconditional signature check in `verify_push` would otherwise
/// require for every agent commit.
///
/// `remote_ids` are the object ids the destination actually holds, read from
/// the remote itself (`git ls-remote`), NOT from the caller-writable
/// `refs/remotes/*`. Exclusions are derived only from what a real remote
/// reports, so an agent cannot forge a local remote-tracking ref to a wrong-
/// author or unsigned commit and have it treated as already-pushed.
#[allow(clippy::type_complexity)]
fn partition_outgoing(
    real_git: &Path,
    ctx: &[String],
    expected: &str,
    sources: &[String],
    remote_ids: &[String],
) -> Result<(Vec<(String, String)>, Vec<String>), String> {
    let mut offenders = Vec::new();
    let mut agent_shas = Vec::new();
    for from in sources {
        let shas = match rev_list_outgoing(real_git, ctx, from, remote_ids) {
            Some(s) => s,
            // The plan named this ref as an update, so it resolves — an inability
            // to compute its outgoing range is a verification failure, not an
            // empty set. Fail closed.
            None => {
                return Err(format!(
                    "buzz git wrapper: refusing to push — could not verify the authorship of \
                     outgoing commits for `{from}`. Enforcement fails closed rather than let \
                     an unverified commit leave the machine."
                ))
            }
        };
        // Patch-ids of commits on a remote but not on this tip — the pool a
        // replayed (cherry-picked/rebased) upstream commit matches. Computed
        // lazily and only when a non-agent author is actually found, so the
        // ordinary all-agent push pays nothing.
        let mut upstream: Option<std::collections::HashSet<String>> = None;
        for sha in shas {
            match commit_author_email(real_git, ctx, &sha) {
                // Agent-authored: attribution is correct. Signature is checked
                // by the caller.
                Some(email) if email == expected => agent_shas.push(sha),
                Some(email) => {
                    // A non-agent author is allowed only when this commit is a
                    // replay (same patch-id) of a commit already upstream —
                    // i.e. a cherry-picked/rebased human commit, which is
                    // correct attribution, not new agent work masquerading as
                    // someone else. Any other non-agent author is an offender.
                    let pool = upstream
                        .get_or_insert_with(|| upstream_patch_ids(real_git, ctx, remote_ids, from));
                    match commit_patch_id(real_git, ctx, &sha) {
                        Some(pid) if pool.contains(&pid) => {} // replayed upstream — exempt
                        Some(_) => offenders.push((sha, email)),
                        // No patch-id (e.g. a merge, or diff-tree failed) means
                        // we cannot prove it is a replay: fail closed on it.
                        None => offenders.push((sha, email)),
                    }
                }
                // Author lookup failed for a commit that rev-list just listed:
                // fail closed rather than silently skip.
                None => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — could not read the author of \
                         outgoing commit `{}`. Enforcement fails closed.",
                        &sha[..sha.len().min(12)]
                    ))
                }
            }
        }
    }
    Ok((offenders, agent_shas))
}

/// Returns `true` when `scheme` is a known git built-in transport — one that
/// git handles without invoking an external `git-remote-<scheme>` helper
/// executable from caller-controlled `PATH`.
///
/// The built-in set (as of Git 2.54) is:
///   `https`, `http`, `ssh`, `git`, `file`, `ftp`, `ftps`
///   plus compound SSH-over-git aliases: `git+ssh`, `ssh+git`
///
/// Note: `ssh://` and SCP-syntax endpoints fork the ambient `ssh` from PATH,
/// but so does a fake `git` placed before `find_real_git()`; both are in the
/// same deliberate-bypass ceiling this PR declares.  They remain allowed.
///
/// Every other `<scheme>://...` string (e.g. `evil://payload`) causes git to
/// look for a `git-remote-<scheme>` executable on `PATH`, giving the caller
/// full control over what happens during dry-run / `ls-remote` vs the real
/// push.  The `::` form is covered separately; this helper covers the
/// `<scheme>://` form.
fn is_builtin_url_scheme(scheme: &[u8]) -> bool {
    matches!(
        scheme,
        b"https" | b"http" | b"ssh" | b"git" | b"file" | b"ftp" | b"ftps" | b"git+ssh" | b"ssh+git"
    )
}

/// Returns `true` when `arg` is a git-push argv token that would resolve to
/// `--receive-pack` or its alias `--exec`, in any spelling git accepts.
///
/// Git accepts unique prefix abbreviations of long options. Empirically verified
/// against git 2.x on this host:
/// - `--rece` is the shortest accepted prefix of `--receive-pack` (both attached
///   `--rece=<cmd>` and separate-value `--rece <cmd>` are accepted). `--rec` and
///   shorter are rejected by git as ambiguous with `--recurse-submodules`.
/// - `--e` is the shortest accepted prefix of `--exec` (both `--e=<cmd>` and
///   `--e <cmd>` are accepted). `--exec` is a documented alias for `--receive-pack`.
///
/// We reject conservatively: any token whose prefix matches a uniquely-resolving
/// abbreviation. `--rece` covers `--rece`, `--recei`, `--receiv`, `--receive`,
/// `--receive-`, `--receive-p`, …, `--receive-pack` (all by `starts_with`).
/// The `--exec` family is covered by explicit arms down to `--e`.
///
/// This may refuse a future git-push option beginning with `rece` or `e`, but
/// fail-closed is the correct trade-off for an attack surface.
fn is_receive_pack_or_exec_flag(arg: &str) -> bool {
    // Any prefix of `--receive-pack` starting at the shortest unique prefix `--rece`.
    // (`--rec` is ambiguous with `--recurse-submodules` and rejected by git itself,
    // so `--rece` is the boundary that matters.)
    if arg.starts_with("--rece") {
        return true;
    }
    // `--exec` and all its prefix abbreviations down to `--e` / `--e=`.
    // Covers both bare form (separate value) and attached `=<cmd>` form.
    if arg == "--exec"
        || arg.starts_with("--exec=")
        || arg == "--exe"
        || arg.starts_with("--exe=")
        || arg == "--ex"
        || arg.starts_with("--ex=")
        || arg == "--e"
        || arg.starts_with("--e=")
    {
        return true;
    }
    false
}

/// Reject every receive-pack override form in managed mode. A caller-controlled
/// `--receive-pack`/`--exec` program or `remote.<name>.receivepack` config key
/// is executable code: it can answer with decoy old-OIDs (making git believe a
/// commit is already on the remote) while writing to an entirely different
/// endpoint. It is not an independent authority and must be refused before any
/// push-plan probe.
///
/// This function handles the **argv** surface only:
/// `--receive-pack=<cmd>`, `--receive-pack <cmd>` (separate value), `--exec=<cmd>`,
/// `--exec <cmd>`, and `--no-dry-run` (plus prefix abbreviations). The config
/// surface (`remote.<name>.receivepack`) is handled by [`inspect_push_config`],
/// which folds it into the single bounded config snapshot alongside all other
/// endpoint and transport keys.
///
/// Only enforces in a managed session; unmanaged pushes are unaffected.
fn reject_receive_pack_override(argv: &[String]) -> Result<(), String> {
    // Argv flags only (config is handled by inspect_push_config).
    //
    // Git accepts any unique prefix of a long option. `--receive-pack` can be
    // abbreviated as `--receive-p`, `--receive-pa`, `--receive=` (git also
    // accepts `=` as a separator for all long options), etc. `--exec` is an
    // alias for `--receive-pack` and likewise accepts prefix abbreviations:
    // `--exe=`, `--exe`, `--ex=`, and even `--e=` (when no other `--e*` option
    // would be ambiguous in this subcommand context). Rather than maintain an
    // exhaustive list of accepted git abbreviations — which would need updating
    // every time git adds a push option beginning with `e` or `receive` — we
    // reject conservatively: any flag whose leading letters could resolve to
    // `--receive-pack` or `--exec`.
    //
    // Specifically, an argument that begins with `--receive` (any prefix of
    // `--receive-pack`, and only `--receive-pack` starts with `--receive` among
    // git-push options) or `--exec` / `--exe=` / any `--e=` (sole `--e*`
    // accepting a value in this subcommand) is rejected. The separate-value
    // form (`--receive-p <cmd>`) is caught by the same prefix check because
    // the token itself begins with `--receive`.
    //
    // `--no-dry-run` clears the dry-run bit that the verification probe injects.
    // Without this guard, `git push --no-dry-run origin main` becomes
    // `git push --dry-run --porcelain --no-verify --no-dry-run origin main`
    // during the probe — the later `--no-dry-run` wins, turning the read-only
    // probe into a real push before authorship/signature checks run.  Git's
    // option parser treats `--no-<flag>` forms of negatable bit options as the
    // cleared form; any unique prefix abbreviation (`--no-dry-r`, `--no-dry`,
    // `--no-dr`) is also accepted.  `--no-d` is AMBIGUOUS (matches both
    // `--no-delete` and `--no-dry-run`); `--no-dr` is the minimal unique
    // prefix — `dr` prefix-matches only `dry-run` among all git-push options.
    // The `starts_with("--no-dr")` check covers every accepted negation form.
    let (_, sub_idx) = split_globals(argv);
    if let Some(si) = sub_idx {
        let push_args = &argv[si + 1..];
        let mut i = 0;
        while i < push_args.len() {
            let t = &push_args[i];
            if is_receive_pack_or_exec_flag(t) {
                return Err(format!(
                    "buzz git wrapper: refusing `{t}` — custom receive-pack/exec programs \
                     cannot be used in managed mode. They can advertise decoy old-OIDs and \
                     redirect pushes to a different endpoint, bypassing push verification."
                ));
            }
            // `--no-dry-run` (and any unique prefix abbreviation, down to
            // `--no-dr`) would clear the `--dry-run` flag the verification
            // probe injects after the subcommand, turning the read-only
            // probe into a real push.
            if t.starts_with("--no-dr") {
                return Err(format!(
                    "buzz git wrapper: refusing `{t}` — `--no-dry-run` (and its prefix \
                     abbreviations) cannot be used in managed mode. The push verification \
                     probe injects `--dry-run` immediately after the subcommand; a later \
                     `--no-dry-run` clears it, turning the supposedly read-only probe into \
                     a real push before authorship and signature checks run."
                ));
            }
            i += 1;
        }
    }
    Ok(())
}

/// Inspect push config and argv in a single bounded pass, refusing any
/// transport override, helper URL, or endpoint newline found in the effective
/// configuration or argv.
///
/// # What is checked
///
/// **Environment** (Surface 0): four env vars the caller can set to redirect
/// SSH/proxy transport or builtin lookup — checked synchronously against the
/// current process env via `var_os` so a non-UTF-8 value is still caught.
///
/// **Argv** (Surface 1): every effective-argv token is checked for:
/// - `--exec-path` in any form (attached `--exec-path=<dir>` or detached
///   `--exec-path <dir>`).  A caller-controlled exec-path directory can hold a
///   stateful fake `git-receive-pack` that behaves differently during dry-run /
///   `ls-remote` vs the real push.
/// - Any `::` sequence (`<transport>::<address>` or `ext::<cmd>`).  Remote
///   helpers can split fetch and push endpoints.
/// - CR or LF in any token.  A newline-bearing inline URL causes the porcelain
///   `To` line to be truncated at the newline, making the parsed destination a
///   seeded decoy prefix rather than the real target.
///
/// **Config** (Surface 2): ONE `git config --null --get-regexp` call covers all
/// endpoint-bearing and transport-overriding key families.  A single call bounds
/// the TOCTOU window and prevents the mutable-config inconsistency that would
/// arise from separate sequential probes.  The pattern uses POSIX ERE with
/// unescaped `|` alternation — `^(remote|url|branch|core)\.` — which git's
/// `--get-regexp` engine accepts.  (BRE-style `\(remote\|...\)` produces exit 1
/// + empty output because `\|` is not a BRE operator in the POSIX sense.)
///
/// For each NUL-delimited record (`<key>\n<value>\0`):
/// - Non-UTF-8 key or value fails closed (no silent skip).
/// - Malformed record (no `\n` separator) fails closed.
/// - Key-name-based policy is applied first, then value-content checks.
///
/// **Key-name policy:**
/// - `remote.*.receivepack`       — refused; custom receive-pack programs can
///   advertise decoy old-OIDs and redirect pushes to a different endpoint.
/// - `core.sshcommand`            — refused; overrides SSH program for all ops.
/// - `core.gitproxy`              — refused; overrides git:// protocol proxy.
/// - `remote.*.vcs`               — refused; invokes external VCS helpers.
/// - Any key whose **name** contains `::` (e.g. `url.ext::evil.pushinsteadof`) —
///   refused; the helper scheme is encoded in the key itself.
/// - `remote.*.url`, `remote.*.pushurl`, `url.*.insteadof`,
///   `url.*.pushinsteadof`, `remote.pushdefault`,
///   `branch.*.remote`, `branch.*.pushremote` — endpoint keys whose **value**
///   is checked for `::` (helper URL) and for CR/LF (newline bypass).
///
/// Note on HTTP: `ls-remote --upload-pack=git-receive-pack` still uses
/// `service=git-upload-pack` for HTTP(S) transports, so a caller-controlled
/// HTTP proxy (`http.proxy`, `remote.*.proxy`, `$http_proxy`) can distinguish
/// the inventory call from the real push, just as an adversarial smart-HTTP
/// endpoint can.  If HTTP(S) remotes are used, the above endpoint guards are a
/// best-effort ceiling for HTTP(S); `core.gitProxy` applies only to the native
/// `git://` protocol, not to HTTP(S).
fn inspect_push_config(
    real_git: &Path,
    effective_argv: &[String],
    ctx: &[String],
) -> Result<(), String> {
    // ── Surface 0: environment variables ─────────────────────────────────────
    //
    // Use `var_os` so a non-UTF-8 value (which `var` silently ignores) still
    // causes a refusal — git receives the raw OsString regardless of encoding.
    //
    // `GIT_SSH` / `GIT_SSH_COMMAND` redirect the SSH helper for all SSH ops.
    // `GIT_PROXY_COMMAND` redirects the proxy for git:// protocol ops.
    // `GIT_EXEC_PATH` directs git to find builtins (including `git-receive-pack`)
    // in a caller-controlled directory.  A stateful fake `git-receive-pack` in
    // that directory can advertise seeded-decoy IDs during dry-run / ls-remote
    // and then write to the empty actual target on the real push.
    for var in &[
        "GIT_SSH",
        "GIT_SSH_COMMAND",
        "GIT_PROXY_COMMAND",
        "GIT_EXEC_PATH",
    ] {
        if let Some(val) = std::env::var_os(var) {
            if !val.is_empty() {
                return Err(format!(
                    "buzz git wrapper: refusing to push — caller-controlled transport \
                     variable `{var}` is set in the environment. This variable can route \
                     `ls-remote` to a seeded decoy repo while routing `git-receive-pack` to \
                     a different target, bypassing push verification. \
                     Unset `{var}` before pushing in managed mode."
                ));
            }
        }
    }

    // ── Surface 1: effective argv ─────────────────────────────────────────────
    //
    // Scan token-by-token.  For `--exec-path` in the detached form
    // (`--exec-path /dir`), we must skip the following value token so it is
    // not double-counted; we still refuse because the flag itself is present.
    {
        let mut i = 0;
        while i < effective_argv.len() {
            let tok = &effective_argv[i];

            // `--exec-path` in any form — both attached (`--exec-path=/dir`)
            // and detached (`--exec-path /dir`).  Any prefix abbreviation of
            // `--exec-path` that git accepts is covered by the `starts_with`
            // guard.  We refuse on the flag token itself; there is no need to
            // inspect the value.
            if tok == "--exec-path"
                || tok.starts_with("--exec-path=")
                || (tok.starts_with("--exec-path") && !tok.starts_with("--exec-path-"))
            {
                return Err(format!(
                    "buzz git wrapper: refusing to push — `--exec-path` is set in effective \
                     argv (`{tok}`). A caller-controlled exec-path directory can contain a \
                     stateful fake `git-receive-pack` that behaves differently during dry-run \
                     / ls-remote vs the real push, bypassing push verification. \
                     Remove `--exec-path` before pushing in managed mode."
                ));
            }

            // Any `::` sequence in an argv token — `ext::<cmd>` or
            // `<transport>::<address>`.  We check raw bytes (no UTF-8 decode)
            // because git also processes the raw byte string.
            if tok.as_bytes().windows(2).any(|w| w == b"::") {
                return Err(format!(
                    "buzz git wrapper: refusing to push — push argument `{}` uses a \
                     remote helper transport (`<transport>::<address>` or `ext::<cmd>`). \
                     Remote helpers can split fetch and push endpoints, bypassing push \
                     verification. Use a built-in transport (https://, ssh://, local path) \
                     in managed mode.",
                    tok.escape_default()
                ));
            }

            // Unknown `<scheme>://` in an argv token.  Git dispatches any URL
            // whose scheme is not a built-in to an external `git-remote-<scheme>`
            // executable on PATH.  That helper controls both the dry-run inventory
            // call and the real push, which breaks push verification.
            //
            // We check the token's SCHEME FIELD, not the entire token before
            // `://`.  A token like `remote.origin.url=https://host/repo.git`
            // (the value after a `-c`) or `--repo=https://host/repo.git` has an
            // `=` sign before `://`; the part before `://` is a config key (or
            // option name), not a URL scheme.  Applying the scheme check to the
            // full `remote.origin.url=https` prefix would incorrectly refuse
            // valid inline config and `--repo=` built-in URLs.
            //
            // The scheme field is the portion between the last `=` that precedes
            // `://` (the delimiter between key and value) and the `://` itself.
            // When there is no `=` before `://`, the scheme is everything before
            // `://`, which covers bare URL arguments (`evil://payload`).
            if let Some(sep) = tok.as_bytes().windows(3).position(|w| w == b"://") {
                // Find the last `=` before `://`; the scheme starts after it.
                let scheme_start = tok.as_bytes()[..sep]
                    .iter()
                    .rposition(|&b| b == b'=')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let scheme = &tok.as_bytes()[scheme_start..sep];
                if !is_builtin_url_scheme(scheme) {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — push argument `{}` uses an \
                         unknown URL scheme `{}`. Only built-in git transports (https, http, \
                         ssh, git, git+ssh, ssh+git, file, ftp, ftps) are permitted in managed \
                         mode; other schemes invoke an external `git-remote-<scheme>` helper \
                         that can split fetch and push endpoints, bypassing push verification.",
                        tok.escape_default(),
                        String::from_utf8_lossy(scheme)
                    ));
                }
            }

            // CR or LF in any argv token.
            if tok.contains('\n') || tok.contains('\r') {
                return Err(format!(
                    "buzz git wrapper: refusing to push — push argument `{}` contains a CR or \
                     LF character. Endpoint URLs containing newlines make the push destination \
                     ambiguous in git's porcelain output. Enforcement fails closed.",
                    tok.escape_default()
                ));
            }

            i += 1;
        }
    }

    // ── Surface 2: effective config — single bounded snapshot ────────────────
    //
    // ONE `git config --null --get-regexp` call covers all key families that
    // carry push-endpoint selection or transport overrides.  Using a single call:
    //   (a) bounds the TOCTOU window to one snapshot of the mutable config state;
    //   (b) avoids up to 13 sequential 120 s-timeout stalls from separate probes.
    //
    // The pattern uses ERE (git's --get-regexp engine accepts POSIX ERE with
    // unescaped `|` alternation).  BRE-style `\(remote\|url\|...\)` produces
    // exit 1 + empty output on Git ≥ 2.0 because `\|` is not a BRE operator in
    // the POSIX sense, so git treats it as a literal-pipe pattern that matches
    // nothing.  Use unescaped `(remote|url|branch|core)`:
    //
    //   `r"^([Rr][Ee][Mm][Oo][Tt][Ee]|[Uu][Rr][Ll]|[Bb][Rr][Aa][Nn][Cc][Hh]|[Cc][Oo][Rr][Ee])\."` —
    //   matches any key whose section is one of the four families, case-insensitively
    //   (git 2.54 lowercases keys before matching but older versions may not).
    //   This is slightly over-inclusive (e.g.
    //   `core.editor`) but the per-record policy applied below ignores keys that
    //   are not in the forbidden or endpoint sets, so false-positive records are
    //   parsed but never acted upon.
    //
    // The per-record policy map (applied to canonical lowercase keys):
    //
    //   Refused (transport override keys):
    //     core.sshcommand, core.gitproxy, remote.*.vcs
    //   Endpoint keys (value checked for `::` helper and CR/LF):
    //     remote.*.url, remote.*.pushurl,
    //     url.*.insteadof, url.*.pushinsteadof,
    //     remote.pushdefault,
    //     branch.*.remote, branch.*.pushremote
    //
    //   Additionally: any key whose name itself contains `::` is refused
    //   regardless of family (e.g. `url.ext::evil.pushinsteadof`).
    //
    // NUL record format (--null output): `<key>\n<value>\0`
    // A malformed record (no `\n`) and a non-UTF-8 key or value both fail closed.
    {
        let mut args = ctx.to_vec();
        // The ERE `^` anchors to the start of the key name; `\.` matches a
        // literal dot.  The pattern covers all four top-level sections.
        // The pattern uses explicit character-class alternation for the four
        // section names to ensure case-insensitive matching independent of git
        // version.  Git 2.54 lowercases keys before applying the regex, so the
        // lowercase-only pattern works today, but older versions and any future
        // change in git's normalization order could miss a capitalized section
        // (`[Remote]`, `[Core]`).  Using `[Rr][Ee]...` makes the match correct
        // regardless of the key casing in the output stream.
        //
        // git's ERE engine does NOT support the `(?i)` flag, so inline
        // character classes are the only portable option.
        args.extend(
            [
                "config",
                "--null",
                "--get-regexp",
                r"^([Rr][Ee][Mm][Oo][Tt][Ee]|[Uu][Rr][Ll]|[Bb][Rr][Aa][Nn][Cc][Hh]|[Cc][Oo][Rr][Ee])\.",
            ]
            .map(String::from),
        );
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = match capture_raw_bounded(real_git, &arg_refs, DRY_RUN_TIMEOUT) {
            Some(o) => o,
            None => {
                return Err(String::from(
                    "buzz git wrapper: refusing to push — could not query effective config \
                     for push-relevant keys (timed out or spawn failed). \
                     Enforcement fails closed.",
                ));
            }
        };

        // Only exit 1 + empty stdout is accepted as "no matching keys".
        let is_no_match = out.status.code() == Some(1) && out.stdout.trim_ascii().is_empty();
        if is_no_match {
            return Ok(());
        }
        if !out.status.success() {
            let detail = if !out.stderr.trim_ascii().is_empty() {
                format!(
                    ": {}",
                    String::from_utf8_lossy(out.stderr.trim_ascii())
                        .lines()
                        .next()
                        .unwrap_or("?")
                )
            } else {
                format!(" (exit {:?})", out.status.code())
            };
            return Err(format!(
                "buzz git wrapper: refusing to push — querying effective config for \
                 push-relevant keys produced an unexpected result{detail}. \
                 Enforcement fails closed."
            ));
        }

        // Parse every NUL-delimited record and apply per-key policy.
        for record in out.stdout.split(|&b: &u8| b == 0) {
            if record.is_empty() {
                continue; // trailing NUL after last record — normal
            }
            // Find the `\n` separator between key and value.
            let sep = match record.iter().position(|&b| b == b'\n') {
                Some(pos) => pos,
                None => {
                    return Err(String::from(
                        "buzz git wrapper: refusing to push — `git config --null` produced \
                         a malformed record (no key/value separator). \
                         Enforcement fails closed.",
                    ));
                }
            };
            let key_bytes = &record[..sep];
            let val_bytes = &record[sep + 1..];

            // Non-UTF-8 key: fail closed.  Git itself rejects non-UTF-8 key names
            // during write, but a hand-crafted config file can carry them.
            let key_raw = match std::str::from_utf8(key_bytes) {
                Ok(k) => k,
                Err(_) => {
                    return Err(String::from(
                        "buzz git wrapper: refusing to push — effective config contains a \
                         key with invalid UTF-8 in its name. \
                         Enforcement fails closed.",
                    ));
                }
            };

            // Normalize the key for case-insensitive policy matching: section
            // and variable names are case-insensitive in git; subsections are
            // case-sensitive.  Produce `key` with section and variable
            // ASCII-lowercased, subsection bytes preserved.
            //
            // Key format: `section.variable`  (no subsection)
            //          or `section.subsection.variable`  (subsection may contain dots)
            //
            // Git's `--get-regexp` output already lowercases section and variable
            // in modern versions, but we normalize here defensively so the policy
            // checks are correct regardless of git version.
            let key_norm: String = {
                let first_dot = key_raw.find('.');
                let last_dot = key_raw.rfind('.');
                match (first_dot, last_dot) {
                    (Some(f), Some(l)) if f == l => {
                        // No subsection: `section.variable`
                        let section = &key_raw[..f];
                        let variable = &key_raw[f + 1..];
                        format!(
                            "{}.{}",
                            section.to_ascii_lowercase(),
                            variable.to_ascii_lowercase()
                        )
                    }
                    (Some(f), Some(l)) if f < l => {
                        // Has subsection: `section.subsection.variable`
                        let section = &key_raw[..f];
                        let subsection = &key_raw[f + 1..l]; // case-sensitive — not lowercased
                        let variable = &key_raw[l + 1..];
                        format!(
                            "{}.{}.{}",
                            section.to_ascii_lowercase(),
                            subsection,
                            variable.to_ascii_lowercase()
                        )
                    }
                    _ => key_raw.to_ascii_lowercase(), // malformed — lowercase everything
                }
            };
            let key = key_norm.as_str();

            // Non-UTF-8 value: fail closed.  A non-UTF-8 URL is not a built-in
            // transport form and cannot be proven safe.
            let value = match std::str::from_utf8(val_bytes) {
                Ok(v) => v,
                Err(_) => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — effective config key `{key}` \
                         has a value with invalid UTF-8. Non-UTF-8 config values cannot be \
                         proven safe. Enforcement fails closed."
                    ));
                }
            };

            // Any key whose name itself contains `::` encodes a helper scheme.
            // Example: `url.ext::evil.pushinsteadof` — the executable helper
            // `ext::evil` is in the key, not the value.
            if key.as_bytes().windows(2).any(|w| w == b"::") {
                return Err(format!(
                    "buzz git wrapper: refusing to push — effective config key `{key}` \
                     encodes a remote helper transport in its name (`::` sequence). \
                     Remote helpers can split fetch and push endpoints, bypassing push \
                     verification. Remove the helper transport config before pushing \
                     in managed mode."
                ));
            }

            // A `url.*` key whose subsection encodes an unknown `<scheme>://`
            // invokes `git-remote-<scheme>` when git applies the rewrite.
            // Example: `url.evil://payload.pushinsteadof` causes git to invoke
            // `git-remote-evil` when it resolves any endpoint that matches the
            // insteadOf pattern.
            //
            // Importantly, the scheme is the FULL subsection prefix before `://`,
            // NOT just the last dot-segment.  For `url.foo.https://bar.insteadof`
            // the subsection is `foo.https://bar`; git treats `foo.https` as the
            // scheme and invokes `git-remote-foo.https` — so extracting only
            // `https` (the last dot-segment) would falsely allow it.
            //
            // Correct extraction: strip the `url.` section prefix, then take
            // everything up to the first `://` as the bare scheme string.
            if let Some(subsection_and_rest) = key.strip_prefix("url.") {
                if let Some(sep) = subsection_and_rest
                    .as_bytes()
                    .windows(3)
                    .position(|w| w == b"://")
                {
                    let bare_scheme = &subsection_and_rest.as_bytes()[..sep];
                    if !is_builtin_url_scheme(bare_scheme) {
                        return Err(format!(
                            "buzz git wrapper: refusing to push — effective config key `{key}` \
                             encodes an unknown URL scheme `{}` in its subsection. Only \
                             built-in git transports (https, http, ssh, git, git+ssh, ssh+git, \
                             file, ftp, ftps) are permitted in managed mode; other schemes \
                             invoke an external `git-remote-<scheme>` helper.",
                            String::from_utf8_lossy(bare_scheme)
                        ));
                    }
                }
            }

            // Apply key-name-based policy.
            match key {
                // ── Refused transport/redirect override keys ──────────────────
                // `remote.*.receivepack`: custom receive-pack programs can
                // advertise decoy old-OIDs and redirect pushes.
                k if k.starts_with("remote.") && k.ends_with(".receivepack") => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — effective config contains \
                         `{key}` = `{value}`. Custom receive-pack programs cannot be used \
                         in managed mode: they can advertise decoy old-OIDs and redirect \
                         pushes to a different endpoint, bypassing push verification. \
                         Remove the `receivepack` config key before pushing in managed mode."
                    ));
                }
                // `core.sshcommand`: overrides the SSH program for all SSH ops.
                "core.sshcommand" => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — effective config contains \
                         `{key}` = `{value}`. This key overrides the SSH program for all \
                         git transport operations and can route `ls-remote` to a seeded \
                         decoy while routing `git-receive-pack` to a different target. \
                         Remove it before pushing in managed mode."
                    ));
                }
                // `core.gitproxy`: overrides the proxy for the native git:// protocol.
                "core.gitproxy" => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — effective config contains \
                         `{key}` = `{value}`. This key overrides the proxy for the native \
                         git:// protocol and can route `ls-remote` to a seeded decoy while \
                         routing `git-receive-pack` to a different target. \
                         Remove it before pushing in managed mode."
                    ));
                }
                // `remote.*.vcs`: invokes external VCS remote helpers.
                k if k.starts_with("remote.") && k.ends_with(".vcs") => {
                    return Err(format!(
                        "buzz git wrapper: refusing to push — effective config contains \
                         `{key}` = `{value}`. External VCS remote helpers can split fetch \
                         and push endpoints, bypassing push verification. \
                         Remove it before pushing in managed mode."
                    ));
                }

                // ── Endpoint keys: check value for helper URL and CR/LF ──────
                k if (k.starts_with("remote.")
                    && (k.ends_with(".url") || k.ends_with(".pushurl")))
                    || (k.starts_with("url.")
                        && (k.ends_with(".insteadof") || k.ends_with(".pushinsteadof")))
                    || k == "remote.pushdefault"
                    || (k.starts_with("branch.")
                        && (k.ends_with(".remote") || k.ends_with(".pushremote"))) =>
                {
                    // Helper transport in value: any `::` sequence.
                    if val_bytes.windows(2).any(|w| w == b"::") {
                        return Err(format!(
                            "buzz git wrapper: refusing to push — effective config key `{key}` \
                             has value `{value}` which uses a remote helper transport \
                             (`<transport>::<address>` or `ext::<cmd>`). Remote helpers can \
                             split fetch and push endpoints, bypassing push verification. \
                             Remove the helper transport URL before pushing in managed mode."
                        ));
                    }
                    // Unknown `<scheme>://` in value: git dispatches any URL whose
                    // scheme is not built-in to an external `git-remote-<scheme>`
                    // helper.  Same bypass risk as the `::` form.
                    if let Some(sep) = val_bytes.windows(3).position(|w| w == b"://") {
                        let scheme = &val_bytes[..sep];
                        if !is_builtin_url_scheme(scheme) {
                            return Err(format!(
                                "buzz git wrapper: refusing to push — effective config key \
                                 `{key}` has value `{value}` which uses an unknown URL scheme \
                                 `{}`. Only built-in git transports (https, http, ssh, git, \
                                 git+ssh, ssh+git, file, ftp, ftps) are permitted in managed \
                                 mode; other schemes invoke an external `git-remote-<scheme>` \
                                 helper that can split fetch and push endpoints.",
                                String::from_utf8_lossy(scheme)
                            ));
                        }
                    }
                    // CR or LF in value: newline-bypass.
                    if val_bytes.contains(&b'\n') || val_bytes.contains(&b'\r') {
                        return Err(format!(
                            "buzz git wrapper: refusing to push — effective config key `{key}` \
                             contains a CR or LF in its value. Endpoint URLs containing \
                             newlines make the push destination ambiguous in git's porcelain \
                             output. Enforcement fails closed."
                        ));
                    }
                }

                // All other keys in the remote/url/branch/core namespace are
                // informational or unrelated to push-transport security — ignore.
                _ => {}
            }
        }
    }

    Ok(())
}

/// Verify that every commit being pushed that is not already on a remote is
/// authored by the agent identity and carries a valid NIP-GS signature by the
/// agent key. `Ok(())` allows the push. Every valid [`Authority`] enforces
/// signing (the manifest contract guarantees it), so the signature requirement
/// is unconditional here.
///
/// The set of refs being pushed is git's own resolved update plan, obtained via
/// `push --no-verify --dry-run --porcelain` rather than reconstructed from a
/// partial argv grammar. That plan reflects `--all`/`--mirror`/`--tags`,
/// `remote.<name>.push`, `push.default`, wildcard refspecs, aliases, and `-C`
/// context exactly as git resolves them — the whole class of predictor gaps.
/// `--no-verify` on the *probe* skips the repo's own pre-push hook (the real
/// push still runs it); enforcement itself runs unconditionally, so a
/// `--no-verify` on the real push cannot bypass it.
///
/// Scope guard against false positives: `rev-list <from> --not --remotes`
/// yields only commits absent from every remote-tracking ref. Pre-existing
/// human commits pulled in by a plain merge are excluded (they are reachable
/// from `refs/remotes/*`). A commit that a cherry-pick or rebase *replayed*
/// gets a new SHA, so it is NOT reachable from a remote and would be flagged —
/// but its patch is identical to an upstream commit, so it is exempted by
/// patch-equivalence ([`patch_equivalent_upstream`]): only genuinely new
/// agent work is required to carry the agent identity. This is what lets a
/// branch carrying rebased/cherry-picked upstream human commits push cleanly.
fn verify_push(
    real_git: &Path,
    argv: &[String],
    effective_argv: &[String],
    ctx: &[String],
    authority: &Authority,
) -> Result<(), String> {
    let expected = &authority.email;

    // Reject every form of receive-pack customization in managed mode. Scanned
    // against `effective_argv` (the alias-expanded command) so that a bare-word
    // alias like `alias.p = push --exec evil` doesn't slip past a literal-argv
    // scan.
    reject_receive_pack_override(effective_argv)?;

    // Single-snapshot inspection of environment, argv, and effective config for
    // transport overrides, helper URLs, and endpoint newlines.  One bounded
    // `git config --null --get-regexp` covers all key families; env and argv
    // are checked synchronously.  See `inspect_push_config` for the full
    // bypass taxonomy and key-name policy.
    inspect_push_config(real_git, effective_argv, ctx)?;

    // git's resolved update plan. Unreachable remote / any dry-run failure =
    // fail closed with the loud message: the real push would fail anyway, and
    // an unverifiable plan must never be treated as "nothing to check".
    let plan = match resolve_push_sources(real_git, argv) {
        Some(p) => p,
        None => {
            return Err(String::from(
                "buzz git wrapper: refusing to push — could not verify outgoing commits: \
                 `git push --dry-run` failed (e.g. remote unreachable). Enforcement fails \
                 closed rather than let an unverified commit leave the machine.",
            ))
        }
    };

    // The destination is read from the `To` header of the dry-run output.
    // `parse_porcelain_destination_unique` returns `None` when there are zero
    // headers (unexpected shape — fail closed) or more than one (multiple
    // `pushurl`s — refused in managed mode rather than modeled, which prevents
    // the A-has-HEAD/B-empty bypass).
    let dest = match plan.destination.as_deref() {
        Some(d) => d,
        None => {
            return Err(String::from(
                "buzz git wrapper: refusing to push — could not identify a single push \
                 destination from git's plan. Either no `To` header was present (unexpected \
                 git output), or multiple `To` headers were present (multiple `pushurl`s are \
                 not supported in managed mode — they create a bypass where a destination that \
                 already holds a commit can mask it as already-remote, exempting it from \
                 verification at other destinations). Enforcement fails closed.",
            ))
        }
    };

    // Before reading the destination's object ids, verify that replaying `dest`
    // through `git ls-remote --get-url` does not produce a different URL. If it
    // does, a chained `insteadOf` rule would make `ls-remote <dest>` read a
    // different endpoint than the one git actually pushed to, poisoning the
    // exclusion set. Fail closed on any rewrite or probe failure.
    verify_destination_stable(real_git, ctx, dest)?;

    // The object ids the destination actually holds, read from the remote
    // itself. This is the ONLY trustworthy exclusion source: `refs/remotes/*` is
    // caller-writable local state, so deriving exclusions from it lets an agent
    // `git update-ref refs/remotes/forged/x HEAD` and hide an unsigned or
    // wrong-author commit from the walk. `git ls-remote` reflects the real
    // destination and supplies the complete ref set (not just the pushed refs),
    // which is required for the patch-id replay exemption to work correctly.
    let remote_ids = match remote_object_ids(real_git, ctx, dest) {
        Some(ids) => ids,
        None => {
            return Err(String::from(
                "buzz git wrapper: refusing to push — could not read the destination's \
                 commits (`git ls-remote` failed). Enforcement fails closed rather than \
                 derive exclusions from forgeable local remote-tracking refs.",
            ))
        }
    };

    // One walk of the outgoing commits, partitioned by the two distinct
    // enforcement concerns: authorship (non-agent authors that are not a
    // replayed-upstream commit) and signing (agent-authored commits whose
    // signature must be verified). Fails closed on any probe error.
    let (offenders, agent_shas) =
        partition_outgoing(real_git, ctx, expected, &plan.sources, &remote_ids)?;

    // Every valid `Authority` enforces signing (the manifest contract
    // guarantees `commit.gpgSign=true` and a signing key matching the author
    // email), so each agent-authored outgoing commit MUST carry a valid
    // signature by the agent key — the one check that covers every creation
    // path (`merge`/`pull`/`commit-tree`/plumbing) `enforce` cannot reject.
    let mut unsigned = Vec::new();
    for sha in agent_shas {
        match commit_signature_is_agent(real_git, ctx, authority, &sha) {
            Some(true) => {}
            Some(false) => unsigned.push(sha),
            // The verification probe itself failed to run: fail closed rather
            // than let an unverified commit leave.
            None => {
                return Err(format!(
                    "buzz git wrapper: refusing to push — could not verify the \
                     signature of outgoing commit `{}`. Enforcement fails closed.",
                    &sha[..sha.len().min(12)]
                ))
            }
        }
    }

    if offenders.is_empty() && unsigned.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    if !offenders.is_empty() {
        msg.push_str(
            "buzz git wrapper: refusing to push — these outgoing commits are not authored \
             by your agent identity (expected author email ",
        );
        msg.push_str(expected);
        msg.push_str("):\n");
        for (sha, email) in &offenders {
            msg.push_str(&format!(
                "  {} authored by {}\n",
                &sha[..sha.len().min(12)],
                email
            ));
        }
        msg.push_str(
            "Re-author them as your agent identity (e.g. `git rebase` with `--reset-author`-free \
             re-commits under the managed identity) before pushing.",
        );
    }
    if !unsigned.is_empty() {
        if !msg.is_empty() {
            msg.push('\n');
        }
        msg.push_str(
            "buzz git wrapper: refusing to push — these outgoing commits are authored by your \
             agent identity but carry no valid signature by your agent key (e.g. created via \
             `git merge`/`pull --no-gpg-sign` or `commit-tree`):\n",
        );
        for sha in &unsigned {
            msg.push_str(&format!("  {}\n", &sha[..sha.len().min(12)]));
        }
        msg.push_str(
            "Re-sign them under the managed identity (e.g. `git rebase --exec 'git commit \
             --amend --no-edit -S' <base>`) before pushing.",
        );
    }
    Err(msg)
}

/// git's resolved push plan: the local source refs whose outgoing commits must
/// be verified, plus the single destination git resolved for this push. The
/// destination comes from the `To` header of git's own `--porcelain` output and
/// is verified stable (no `insteadOf` replay) before being passed to
/// `git ls-remote`.
struct PushPlan {
    sources: Vec<String>,
    destination: Option<String>,
}

/// The local source refs a push will send, per git's own resolved plan.
///
/// Runs the user's exact invocation with `--dry-run --porcelain --no-verify`
/// injected right after the subcommand token, so config aliases expand and
/// repository context (`-C`, `--git-dir`) applies exactly as in the real push.
/// Returns `None` on any dry-run failure (caller fails closed). Deletions and
/// up-to-date refs contribute no source; every other line's local ref (left of
/// `:` in the `from:to` field) is a source whose outgoing commits are checked.
fn resolve_push_sources(real_git: &Path, argv: &[String]) -> Option<PushPlan> {
    let sub_idx = split_globals(argv).1?;
    let mut full = argv.to_vec();
    // Inject after the subcommand token (`push` or an alias resolving to it).
    full.splice(
        sub_idx + 1..sub_idx + 1,
        ["--dry-run", "--porcelain", "--no-verify"].map(String::from),
    );
    let arg_refs: Vec<&str> = full.iter().map(String::as_str).collect();
    // Bounded: the probe contacts the remote, so an unresponsive remote must not
    // hang the wrapper. Timeout returns `None`, which the caller fails closed on.
    let out = capture_raw_bounded(real_git, &arg_refs, DRY_RUN_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    // Strict UTF-8 decode: a remote URL can contain arbitrary bytes. Lossy
    // decoding would replace invalid sequences with U+FFFD, changing the `To`
    // destination. If a U+FFFD path happened to exist as a decoy, the
    // stability probe would validate and inventory the decoy while the real
    // push updated the original-byte path — same wrong-endpoint bypass class
    // as an insteadOf rewrite. Fail closed on any non-UTF-8 byte.
    let stdout = match std::str::from_utf8(&out.stdout) {
        Ok(s) => s,
        Err(_) => return None,
    };
    Some(PushPlan {
        sources: parse_porcelain_sources(stdout),
        destination: parse_porcelain_destination_unique(stdout),
    })
}

/// The single destination handle from a `--porcelain` push plan. Returns `None`
/// when there are zero `To` lines (unexpected shape) or MORE THAN ONE `To` line
/// (multiple `pushurl`s). Multiple destinations are refused in managed mode
/// rather than modeled — the A-has-HEAD/B-empty bypass cannot occur when at
/// most one destination is in scope. The caller fails closed on `None`.
///
/// Parses raw `\n`-delimited lines without calling `str::lines()`. `str::lines()`
/// silently strips `\r` before `\n`, so a destination `To /path/\r\n` would be
/// parsed as `/path/` instead of `/path/\r`. We split on `\n` directly.
///
/// If ANY `To` payload is empty or contains `\r`, `None` is returned for the
/// ENTIRE plan — the malformed destination is not filtered out silently.
/// Filtering would recreate the multi-destination bypass: with two `pushurl`s, a
/// CR-bearing destination silently dropped plus one clean destination would
/// produce exactly one accepted destination, defeating the multiple-`To` guard.
/// Fail closed on the whole plan instead.
///
/// The `To ` payload is otherwise taken verbatim — no `trim()` — preserving a
/// legitimate destination whose URL ends with a plain space: `verify_destination_stable`
/// compares it byte-for-byte against the `--get-url` output.
fn parse_porcelain_destination_unique(stdout: &str) -> Option<String> {
    let dests: Vec<&str> = stdout
        .split('\n')
        .filter_map(|l| l.strip_prefix("To "))
        .collect();
    // Any malformed payload (empty or containing CR) invalidates the entire plan.
    // We do NOT filter malformed destinations out — that would recreate the
    // multi-destination bypass (CR-bearing dest dropped → only one dest left →
    // single-dest path accepted).
    if dests.iter().any(|d| d.is_empty() || d.contains('\r')) {
        return None;
    }
    if dests.len() == 1 {
        Some(dests[0].to_string())
    } else {
        None // zero (missing/unexpected shape) or multiple (pushurl rejection)
    }
}

/// Count the number of distinct `To` header lines in `--porcelain` push output.
/// Used by tests to assert multiple-`pushurl` rejection.
#[cfg(test)]
fn count_porcelain_destinations(stdout: &str) -> usize {
    stdout.split('\n').filter(|l| l.starts_with("To ")).count()
}

/// Verify that `dest`, taken from the `--porcelain` `To` header, is stable
/// under URL rewriting — i.e. `git ls-remote --get-url <dest>` returns `dest`
/// unchanged. If git rewrites it, the `ls-remote <dest>` call that follows
/// would read a different endpoint than the one git actually pushed to, allowing
/// an `insteadOf`-chained bypass.
///
/// Returns `Ok(())` when stable, `Err(message)` when the probe fails or the
/// output differs (caller fails closed on either).
///
/// Bounded: uses the same `DRY_RUN_TIMEOUT` as all remote-contacting probes.
/// Runs under `ctx` so the same config/repo context applies.
fn verify_destination_stable(real_git: &Path, ctx: &[String], dest: &str) -> Result<(), String> {
    let mut args = ctx.to_vec();
    args.extend(["ls-remote", "--get-url", dest].map(String::from));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = match capture_raw_bounded(real_git, &arg_refs, DRY_RUN_TIMEOUT) {
        Some(o) => o,
        None => {
            return Err(format!(
                "buzz git wrapper: refusing to push — could not verify the push destination \
                 `{dest}` (ls-remote --get-url timed out or failed to run). Enforcement \
                 fails closed."
            ))
        }
    };
    if !out.status.success() {
        return Err(format!(
            "buzz git wrapper: refusing to push — could not verify the push destination \
             `{dest}` (ls-remote --get-url exited non-zero). Enforcement fails closed."
        ));
    }
    // Strict decode: the URL must be valid UTF-8. Lossy conversion could
    // normalize a byte sequence differently than git does, defeating the
    // byte-for-byte comparison.
    let raw_bytes = &out.stdout;
    let raw = match std::str::from_utf8(raw_bytes) {
        Ok(s) => s,
        Err(_) => {
            return Err(format!(
                "buzz git wrapper: refusing to push — the `--get-url` output for `{dest}` \
                 is not valid UTF-8. Enforcement fails closed."
            ))
        }
    };
    // Strip exactly one line terminator (`\n` or `\r\n`) and nothing else.
    // `str::trim` would silently normalize a URL ending with whitespace — e.g.
    // a destination `A ` rewritten to `A ` would compare equal to `A` after
    // trimming, which is exactly the second-stage rewrite bypass this check
    // exists to close. We therefore strip only the git line terminator.
    let resolved = raw
        .strip_suffix('\n')
        .map_or(raw, |s| s.strip_suffix('\r').unwrap_or(s));
    // Require exactly one output line: no trailing content after the terminator.
    // Extra bytes (a second line, trailing junk) would indicate an unexpected
    // output shape — fail closed rather than silently accept a partial match.
    if resolved.contains('\n') {
        return Err(format!(
            "buzz git wrapper: refusing to push — `--get-url` for `{dest}` produced \
             unexpected multi-line output. Enforcement fails closed."
        ));
    }
    if resolved != dest {
        return Err(format!(
            "buzz git wrapper: refusing to push — the push destination `{dest}` is rewritten \
             to `{resolved}` by effective `insteadOf` config. Replaying the rendered `To` \
             label through a second URL-resolution step yields a different endpoint, which \
             would make the exclusion set come from `{resolved}` while the real push targets \
             `{dest}`. Enforcement fails closed rather than read the wrong remote's IDs."
        ));
    }
    Ok(())
}

/// Parse `--porcelain` push output into the set of local source refs whose
/// outgoing commits must be verified. Each machine line is
/// `<flag>\t<from>:<to>\t<summary>`; header (`To …`) and trailer (`Done`) lines
/// lack the tab-delimited `from:to` field and are ignored. A `-` flag (deletion)
/// or empty `from` (deletion refspec) contributes nothing.
///
/// Uses raw `\n`-split (not `str::lines()`) for the same reason as
/// [`parse_porcelain_destination_unique`]: `str::lines()` silently strips `\r`
/// from plan lines. A spurious `\r` in a flag or refspec field would indicate
/// corrupt/unexpected output; we silently skip those lines (no source extracted).
fn parse_porcelain_sources(stdout: &str) -> Vec<String> {
    let mut sources = Vec::new();
    for line in stdout.split('\n') {
        // Strip any trailing \r (CRLF output on some platforms) from the line
        // before field parsing. Unlike the destination, source ref names are
        // always ASCII path components — a trailing \r in a ref name is
        // invalid and the line is skipped.
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut fields = line.split('\t');
        let flag = fields.next().unwrap_or("");
        let refspec = match fields.next() {
            Some(r) if r.contains(':') => r,
            _ => continue, // not a plan line
        };
        if flag == "-" {
            continue; // deletion
        }
        let from = refspec.split(':').next().unwrap_or("");
        if !from.is_empty() {
            sources.push(from.to_string());
        }
    }
    sources
}

/// Preflight the resulting author of a commit-creating invocation and reject it
/// when that author would not be the agent (E). Re-applied identity config
/// cannot fix modes that reuse or preserve another commit's author —
/// `commit -C/-c <sha>` and `commit --amend` stamp the reused/original author
/// onto brand-new content. `--author`/`--reset-author` are already rejected in
/// [`enforce`]; this catches the reuse/amend forms that carry a human author
/// without naming one on the command line.
///
/// History-preserving replays (`rebase`, `cherry-pick`, `am`) are intentionally
/// untouched: preserving an upstream human author there is correct attribution,
/// and the push gate lets those through because the commits already exist
/// upstream (reachable from `refs/remotes/*`).
fn verify_commit_author(
    real_git: &Path,
    argv: &[String],
    ctx: &[String],
    authority: &Authority,
) -> Result<(), String> {
    let Some(sub_idx) = split_globals(argv).1 else {
        return Ok(());
    };
    if argv[sub_idx] != "commit" {
        return Ok(());
    }
    let args = &argv[sub_idx + 1..];

    // The reused/original author source, if any. `-C`/`-c <sha>` reuse that
    // commit's author; `--amend` (without a reuse flag) keeps HEAD's author.
    let reuse_sha = reuse_commit_arg(args);
    let source = if let Some(sha) = reuse_sha {
        Some(sha)
    } else if args.iter().any(|a| a == "--amend") {
        Some("HEAD".to_string())
    } else {
        None
    };
    let Some(source) = source else {
        return Ok(()); // ordinary commit — authored fresh as the agent
    };

    match commit_author_email(real_git, ctx, &source) {
        // Reused author is the agent (e.g. amending the agent's own commit, the
        // normal fixup flow) — allowed.
        Some(email) if email == authority.email => Ok(()),
        Some(email) => Err(format!(
            "buzz git wrapper: refusing this commit — it would be authored by `{email}`, not \
             your agent identity (`{}`). `commit --amend`/`-c`/`-C` preserve the original \
             commit's author on new content. Make a fresh commit (it is authored as your agent \
             identity automatically) and credit the human with `Co-authored-by`/`Signed-off-by` \
             trailers.",
            authority.email
        )),
        // Can't resolve the reuse source's author: fail closed.
        None => Err(format!(
            "buzz git wrapper: refusing this commit — could not determine the author that \
             `{source}` would stamp on it. Enforcement fails closed."
        )),
    }
}

/// The commit named by a `-C <sha>`/`-c <sha>` (or attached `-C<sha>`/`-c<sha>`)
/// author-and-message-reuse option on a `commit` invocation, if present. Unlike
/// the global `-c key=val` config flag, here `-c`/`-C` are `commit` options
/// whose value is a commit-ish; a value containing `=` is a config key, not a
/// commit, so it is ignored.
fn reuse_commit_arg(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-C" || a == "-c" {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a
            .strip_prefix("-C")
            .or_else(|| a.strip_prefix("-c"))
            .filter(|v| !v.is_empty() && !v.contains('='))
        {
            return Some(v.to_string());
        }
        i += 1;
    }
    None
}

fn commit_author_email(real_git: &Path, ctx: &[String], sha: &str) -> Option<String> {
    // Disable replacement-object interpretation for the author probe.
    // git-replace allows ordinary object reads to resolve through a replacement
    // chain, but pack transfer does NOT honour replacements.  Without this flag,
    // `git show REAL` reads the replacement-backed DECOY commit while the push
    // sends the original REAL object — the author on DECOY passes the check but
    // the wrong-authored REAL commit is what reaches the destination.
    let mut args = ctx.to_vec();
    args.push("--no-replace-objects".to_string());
    args.extend(["show", "-s", "--format=%ae", sha].map(String::from));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = capture_raw(real_git, &arg_refs)?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Whether `sha` carries a valid NIP-GS signature by the agent key. Returns
/// `Some(true)` only when git's signature-status placeholder `%G?` is `G` —
/// a good signature whose key is `TRUST_FULLY`, which `git-sign-nostr` emits
/// solely when the verified key equals the configured `user.signingkey`. So `G`
/// means both "cryptographically valid" and "by the expected agent key" in one
/// git-native, network-free probe. `Some(false)` is any other status
/// (`N` unsigned, `U`/`E` valid-but-untrusted/uncheckable, `B` bad). `None`
/// only when the probe itself fails to run — the caller fails closed on that.
///
/// The authority's signing config (`gpg.x509.program`, `user.signingkey`,
/// `nostr.keyfile`) is injected as `-c` so the probe invokes `git-sign-nostr`
/// and resolves trust against the agent key regardless of the repo's own
/// config, mirroring how [`inject_identity_args`] arms real commits.
fn commit_signature_is_agent(
    real_git: &Path,
    ctx: &[String],
    authority: &Authority,
    sha: &str,
) -> Option<bool> {
    let mut args = ctx.to_vec();
    // Disable replacement-object interpretation: the signature probe must read
    // the exact raw commit object the push sends.  git-replace makes `git show`
    // resolve `REAL` through its replacement chain to `DECOY`; the signature on
    // DECOY passes while the unsigned REAL commit reaches the destination.
    args.push("--no-replace-objects".to_string());
    for (key, value) in &authority.entries {
        args.push("-c".to_string());
        args.push(format!("{key}={value}"));
    }
    args.extend(["show", "-s", "--format=%G?", sha].map(String::from));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = capture_raw(real_git, &arg_refs)?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim() == "G")
}

/// The stable patch-id of a single commit, or `None` if it has no diff patch-id
/// (e.g. a merge commit, or `diff-tree` produced nothing). Used to recognize a
/// cherry-picked/rebased copy of an upstream commit by patch content rather than
/// SHA, which the replay rewrote.
fn commit_patch_id(real_git: &Path, ctx: &[String], sha: &str) -> Option<String> {
    let diff = {
        let mut args = ctx.to_vec();
        // Disable replacement-object interpretation: the patch content of REAL
        // must be computed from the raw object, not its replacement.
        args.push("--no-replace-objects".to_string());
        args.extend(["diff-tree", "--root", "-p", sha].map(String::from));
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = capture_raw(real_git, &arg_refs)?;
        if !out.status.success() {
            return None;
        }
        out.stdout
    };
    let ids = patch_ids_from_diff(real_git, ctx, &diff);
    ids.into_iter().next()
}

/// Patch-ids of every commit the destination holds but that is not reachable
/// from `from` — the pool a replayed upstream commit's patch-id must match.
/// Positive revs are the destination's real object ids (`remote_ids`, from
/// `git ls-remote`), NOT `refs/remotes/*`, so a forged local remote-tracking
/// ref cannot inject an attacker-chosen commit into the exemption pool. Bounded
/// to the divergence (`<remote_ids...> --not <from>`) and computed in one
/// `diff-tree | patch-id` pipeline. `--ignore-missing` drops any destination id
/// this clone lacks (a normal skew, not an error). Empty on any failure or when
/// the destination is empty, so a commit can only be *exempted* when a match is
/// positively proven (fail-closed for the gate).
fn upstream_patch_ids(
    real_git: &Path,
    ctx: &[String],
    remote_ids: &[String],
    from: &str,
) -> std::collections::HashSet<String> {
    if remote_ids.is_empty() {
        return std::collections::HashSet::new();
    }
    let mut revs_args = ctx.to_vec();
    // Disable replacement-object interpretation: the graph must reflect raw
    // objects, not replacements.  A `git replace` mapping applied to a remote
    // object SHA would misrepresent what commits the destination actually holds.
    revs_args.push("--no-replace-objects".to_string());
    revs_args.push("rev-list".to_string());
    revs_args.push("--ignore-missing".to_string());
    revs_args.extend(remote_ids.iter().cloned());
    revs_args.push("--not".to_string());
    revs_args.push(from.to_string());
    let revs_refs: Vec<&str> = revs_args.iter().map(String::as_str).collect();
    let revs = match capture_raw(real_git, &revs_refs) {
        Some(o) if o.status.success() => o.stdout,
        _ => return std::collections::HashSet::new(),
    };
    // Feed the SHA list to `diff-tree --stdin -p`, whose diff stream goes to
    // `patch-id`. Do it in two hops (diff-tree captured, then piped to
    // patch-id) to reuse the stdin helper without a shell.
    let diff = {
        let mut args = ctx.to_vec();
        // Same --no-replace-objects flag for diff-tree.
        args.push("--no-replace-objects".to_string());
        args.extend(["diff-tree", "--stdin", "--root", "-p"].map(String::from));
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        match capture_raw_with_stdin(real_git, &arg_refs, &revs) {
            Some(o) if o.status.success() => o.stdout,
            _ => return std::collections::HashSet::new(),
        }
    };
    patch_ids_from_diff(real_git, ctx, &diff)
        .into_iter()
        .collect()
}

/// Run `git patch-id --stable` over a diff stream and return each patch-id (the
/// first whitespace field of every output line).
fn patch_ids_from_diff(real_git: &Path, ctx: &[String], diff: &[u8]) -> Vec<String> {
    let mut args = ctx.to_vec();
    args.extend(["patch-id", "--stable"].map(String::from));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = match capture_raw_with_stdin(real_git, &arg_refs, diff) {
        Some(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Commits reachable from `tip` but not held by the destination. Exclusions are
/// the destination's real object ids (`remote_ids`, from `git ls-remote`), NOT
/// `refs/remotes/*` — a caller cannot forge a local remote-tracking ref to hide
/// an outgoing commit from this walk. `--ignore-missing` drops any destination
/// id absent from this clone (normal skew). `None` when `tip` does not resolve
/// (rev-list exits non-zero); when the destination holds nothing (`remote_ids`
/// empty, e.g. a brand-new branch) every commit reachable from `tip` is
/// outgoing and must be verified.
fn rev_list_outgoing(
    real_git: &Path,
    ctx: &[String],
    tip: &str,
    remote_ids: &[String],
) -> Option<Vec<String>> {
    let mut args = ctx.to_vec();
    // Disable replacement-object interpretation so the outgoing walk reflects
    // the raw object graph that pack transfer will use.  A `git replace` mapping
    // makes `rev-list TIP --not REMOTE_SHA` skip REAL when REAL is replaced by
    // DECOY: git treats REAL as already reachable through the replacement chain,
    // so it appears in the exclusion set even though the destination does NOT
    // hold REAL.  With --no-replace-objects, REAL appears correctly as outgoing.
    args.push("--no-replace-objects".to_string());
    args.push("rev-list".to_string());
    args.push(tip.to_string());
    args.push("--not".to_string());
    args.push("--ignore-missing".to_string());
    args.extend(remote_ids.iter().cloned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = capture_raw(real_git, &arg_refs)?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

/// The distinct object ids the destination currently holds, queried via the
/// **receive** service (`git ls-remote --upload-pack=git-receive-pack <dest>`).
///
/// Using the receive service rather than the default upload service (fetch) is
/// the load-bearing design choice: it binds the exclusion-set inventory to the
/// same backend that will accept the real push.  When the two services have
/// different views of the repository (honest server misconfiguration, or an
/// adversarial split via `GIT_SSH_COMMAND` routing ls-remote to a decoy while
/// routing git-receive-pack to a different target), querying upload-pack returns
/// the decoy's object set and the bypass succeeds; querying receive-pack returns
/// the real write target's object set, so no false exemption is granted.
///
/// The `--upload-pack=git-receive-pack` argument to `ls-remote` names the
/// program to invoke on the server side.  For local-path and SSH transports git
/// literally invokes `git-receive-pack <path>`, binding the inventory query to
/// the same service that will accept the real push.  For HTTP(S) the argument
/// is silently ignored: git issues `/info/refs?service=git-upload-pack`
/// regardless, so for HTTP(S) remotes the inventory reflects the fetch view,
/// not the receive view.  Accepting this best-effort ceiling for HTTP(S) is an
/// explicit design decision; the guard therefore fully closes the split-service
/// bypass only for local and SSH transports, where the flag is honoured.
///
/// Callers still call `reject_receive_pack_override` before reaching here, so
/// a caller-supplied `--receive-pack`/`--exec` argv or `remote.*.receivepack`
/// config cannot repoint this probe.  `inspect_push_config` ensures no
/// `GIT_SSH*` / `core.sshCommand` can steer this call independently of the
/// real push.
///
/// `None` on any failure (caller fails closed); an empty vec means the remote
/// has no refs (brand-new destination), which is a valid state, not an error.
fn remote_object_ids(real_git: &Path, ctx: &[String], dest: &str) -> Option<Vec<String>> {
    let mut args = ctx.to_vec();
    args.extend(["ls-remote", "--upload-pack=git-receive-pack", dest].map(String::from));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // Bounded: contacts the remote, so an unresponsive one must not hang the
    // wrapper. Timeout returns `None`, which the caller fails closed on.
    let out = capture_raw_bounded(real_git, &arg_refs, DRY_RUN_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    parse_remote_object_ids(&out.stdout)
}

/// Parse raw `git ls-remote` output as an exact remote inventory. Truly empty
/// output is the one valid empty inventory; every nonempty record must be
/// `<full lowercase-hex oid>\t<nonempty refname>` with no extra tab fields.
fn parse_remote_object_ids(stdout: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(stdout).ok()?;
    if text.is_empty() {
        return Some(Vec::new());
    }
    let mut ids = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (oid, refname) = line.split_once('\t')?;
        let valid_oid = (oid.len() == 40 || oid.len() == 64)
            && oid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid_oid || refname.is_empty() || refname.contains('\t') {
            return None;
        }
        ids.push(oid.to_string());
    }
    if ids.is_empty() {
        return None;
    }
    ids.sort_unstable();
    ids.dedup();
    Some(ids)
}

/// Run `git <ctx...> <args...>` and capture trimmed stdout when it succeeds.
/// `ctx` carries the caller's complete global set ([`caller_globals`]) so the
/// probe resolves against the same repository, config, and aliases git will use
/// for the real invocation.
fn capture(real_git: &Path, ctx: &[String], args: &[&str]) -> Option<String> {
    let mut full = ctx.to_vec();
    full.extend(args.iter().map(|s| s.to_string()));
    let arg_refs: Vec<&str> = full.iter().map(String::as_str).collect();
    let out = capture_raw(real_git, &arg_refs)?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn capture_raw(real_git: &Path, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = std::process::Command::new(real_git);
    cmd.args(args);
    scrub_env(&mut cmd);
    cmd.output().ok()
}

/// Run `git <args...>` feeding `stdin` to its standard input and capture the
/// output. Used for the `diff-tree --stdin` / `patch-id` pipeline without a
/// shell. These operate on local objects only (no network), so no timeout.
fn capture_raw_with_stdin(
    real_git: &Path,
    args: &[&str],
    stdin: &[u8],
) -> Option<std::process::Output> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = {
        let mut cmd = std::process::Command::new(real_git);
        cmd.args(args);
        scrub_env(&mut cmd);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn().ok()?
    };
    child.stdin.take()?.write_all(stdin).ok()?;
    child.wait_with_output().ok()
}

/// Hard ceiling on the push `--dry-run` probe. The probe contacts the remote to
/// resolve `old..new`, so an unresponsive remote could otherwise block the
/// wrapper — and therefore the agent's `git push` — indefinitely. A synchronous
/// unbounded subprocess in an enforcement path is a defect on its own; this
/// bounds it and the caller treats a timeout as fail-closed.
const DRY_RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Like [`capture_raw`] but killed if it runs past `timeout`. Returns `None` on
/// spawn failure OR timeout (caller fails closed). The entire process group is
/// killed and reaped on timeout so no zombie, detached network client, or
/// remote-helper grandchild survives.
///
/// Follows the `bounded_command.rs` discipline:
///
/// - **Deadlock prevention:** stdout/stderr are drained concurrently on
///   background threads. Without concurrent draining a child that writes more
///   than one pipe-buffer (~16 KB macOS / 64 KB Linux) blocks in the kernel;
///   `wait_timeout` then waits for the child to exit; deadlock. The config
///   snapshot query can emit several hundred KB on a repo with many worktrees.
///
/// - **Memory bound:** an aggregate `stdout+stderr` byte ceiling
///   ([`BOUNDED_CAPTURE_LIMIT`]) is enforced inside the drain sink, not after
///   the fact. On breach the tree is killed and `None` is returned, so a
///   hostile remote peer that streams until the deadline cannot exhaust memory.
///   The drain returns immediately on overflow rather than continuing to read,
///   which is what bounds a continuously-readable pipe.
///
/// - **Process-tree kill:** the child is spawned in its own process group
///   (`process_group(0)` on Unix). On timeout or overflow the full group is
///   signalled, so SSH sessions, `git-remote-*` helpers, and any other
///   grandchild spawned by git are also reaped.
///
/// - **Group-escaped writer:** on Unix, `kill_tree` is `killpg` on the child's
///   group and does not reach a descendant that called `setsid`/`setpgid`
///   while retaining the pipe. The drain reads are therefore made non-blocking,
///   and after the `stop` flag is set a `WouldBlock` ends the drain rather than
///   blocking the join forever.
///
/// - **Kill on success too:** a git remote helper can background a grandchild
///   that outlives the leader. `kill_tree` + `stop` run on every exit path.
const BOUNDED_CAPTURE_LIMIT: u64 = 32 << 20; // 32 MiB — headroom above any real config output

/// Poll interval while waiting for the child to exit in [`capture_raw_bounded`].
const BOUNDED_POLL: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

#[cfg(windows)]
struct BoundedJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for BoundedJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn create_bounded_job(pid: u32) -> Option<BoundedJob> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == FALSE
        {
            CloseHandle(job);
            return None;
        }
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
        if process.is_null() {
            CloseHandle(job);
            return None;
        }
        let assigned = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if assigned == FALSE {
            CloseHandle(job);
            return None;
        }
        Some(BoundedJob(job))
    }
}

#[cfg(windows)]
fn resume_bounded_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut has_entry = Thread32First(snapshot, &mut entry);
        while has_entry != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    if ResumeThread(thread) != u32::MAX {
                        resumed = true;
                    }
                    CloseHandle(thread);
                }
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            has_entry = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        resumed
    }
}

/// Idle backoff inside the nonblocking Unix drain when no bytes are available.
#[cfg(unix)]
const DRAIN_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// Set a file descriptor non-blocking so reads return `WouldBlock` immediately
/// when no bytes are available.  Returns `false` on any `fcntl` failure.
#[cfg(unix)]
fn set_nonblocking_fd<F: std::os::unix::io::AsRawFd>(f: &F) -> bool {
    let fd = f.as_raw_fd();
    // SAFETY: `F_GETFL`/`F_SETFL` read and set only the flags of `fd`;
    // `fd` is owned by `f` for the duration of this call.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return false;
        }
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == 0
    }
}

/// Drain one child stream on its own thread into a buffer bounded by the
/// shared aggregate budget.  Returns the captured bytes, or `Err` on a read
/// error other than `Interrupted`/`WouldBlock`.
///
/// On overflow the thread returns immediately (it does NOT drain to EOF), so a
/// continuously-readable pipe cannot spin the loop forever.  After teardown
/// sets `stop`, a `WouldBlock` ends a Unix drain even if a group-escaped writer
/// still holds the pipe.
fn spawn_bounded_drain<R: std::io::Read + Send + 'static>(
    mut reader: R,
    total: std::sync::Arc<std::sync::atomic::AtomicU64>,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    use std::io::ErrorKind;
    use std::sync::atomic::Ordering;
    #[cfg(windows)]
    let _ = &stop; // only used on Unix
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return Ok(buf),
                Ok(n) => {
                    let prev = total.fetch_add(n as u64, Ordering::Relaxed);
                    if prev.saturating_add(n as u64) > BOUNDED_CAPTURE_LIMIT {
                        overflow.store(true, Ordering::Relaxed);
                        // Clamp what we retain, then return immediately.
                        // Do NOT keep reading — a continuously-readable
                        // pipe would never reach WouldBlock/stop otherwise.
                        let keep =
                            BOUNDED_CAPTURE_LIMIT.saturating_sub(prev).min(n as u64) as usize;
                        buf.extend_from_slice(&chunk[..keep]);
                        return Ok(buf);
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                #[cfg(unix)]
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(buf); // teardown done; escaped writer may hold pipe
                    }
                    std::thread::sleep(DRAIN_IDLE_POLL);
                }
                Err(e) => return Err(e),
            }
        }
    })
}

fn join_bounded_drain(
    handle: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Option<Vec<u8>> {
    match handle {
        Some(h) => h.join().ok()?.ok(),
        None => Some(Vec::new()),
    }
}

fn capture_raw_bounded(
    real_git: &Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> Option<std::process::Output> {
    use std::io::ErrorKind;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    let mut cmd = std::process::Command::new(real_git);
    cmd.args(args);
    scrub_env(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Spawn in an isolated process group so kill-on-timeout reaches the full
    // tree (SSH, remote helpers, etc.), not just the direct child.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Freeze the root until a kill-on-close Job Object owns it, closing the
        // spawn-to-assign race in which a fast root could create an unowned
        // descendant before the parent assigns the job.
        cmd.creation_flags(CREATE_SUSPENDED);
    }

    let mut child = cmd.spawn().ok()?;
    let child_pid = child.id();
    #[cfg(windows)]
    let mut job = match create_bounded_job(child_pid) {
        Some(job) if resume_bounded_process(child_pid) => Some(job),
        Some(job) => {
            drop(job);
            let _ = child.wait();
            return None;
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    let stdout_pipe = child.stdout.take()?;
    let stderr_pipe = child.stderr.take()?;

    // Unix: make pipe reads non-blocking so drain threads can stop on a
    // WouldBlock after teardown, even if a group-escaped writer holds the pipe.
    #[cfg(unix)]
    {
        if !set_nonblocking_fd(&stdout_pipe) || !set_nonblocking_fd(&stderr_pipe) {
            kill_bounded_tree(
                &mut child,
                child_pid,
                #[cfg(windows)]
                &mut job,
            );
            let _ = child.wait();
            return None;
        }
    }
    let total = Arc::new(AtomicU64::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    let stdout_drain = Some(spawn_bounded_drain(
        stdout_pipe,
        total.clone(),
        overflow.clone(),
        stop.clone(),
    ));
    let stderr_drain = Some(spawn_bounded_drain(
        stderr_pipe,
        total.clone(),
        overflow.clone(),
        stop.clone(),
    ));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    kill_bounded_tree(
                        &mut child,
                        child_pid,
                        #[cfg(windows)]
                        &mut job,
                    );
                    break None;
                }
                if overflow.load(Ordering::Relaxed) {
                    kill_bounded_tree(
                        &mut child,
                        child_pid,
                        #[cfg(windows)]
                        &mut job,
                    );
                    break None;
                }
                std::thread::sleep(BOUNDED_POLL);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => {
                kill_bounded_tree(
                    &mut child,
                    child_pid,
                    #[cfg(windows)]
                    &mut job,
                );
                break None;
            }
        }
    };

    // Kill on every path (idempotent) — a successful child may have
    // backgrounded a grandchild that still holds the pipe.  Then raise `stop`
    // so the nonblocking Unix drains end on the next WouldBlock instead of
    // waiting on a group-escaped writer indefinitely.
    kill_bounded_tree(
        &mut child,
        child_pid,
        #[cfg(windows)]
        &mut job,
    );
    let _ = child.wait();
    stop.store(true, Ordering::Relaxed);

    let stdout = join_bounded_drain(stdout_drain);
    let stderr = join_bounded_drain(stderr_drain);

    let (status, stdout, stderr) = (status?, stdout?, stderr?);
    if overflow.load(Ordering::Relaxed) {
        return None;
    }
    Some(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Kill the entire process group spawned with `process_group(0)` on Unix, or
/// fall back to killing the direct child on platforms where group kill is
/// unavailable. Called on timeout, overflow, and success (idempotent).
fn kill_bounded_tree(
    _child: &mut std::process::Child,
    pid: u32,
    #[cfg(windows)] job: &mut Option<BoundedJob>,
) {
    #[cfg(unix)]
    {
        // SAFETY: `pid` equals the child's PGID (set via `process_group(0)`).
        // `killpg` sends SIGKILL to every member of the group.  ESRCH on a
        // dead/already-reaped group is intentional (idempotent).
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
        let _ = pid; // suppress unused-variable on non-unix builds
    }
    #[cfg(windows)]
    {
        let _ = pid;
        drop(job.take());
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        let _ = child.kill();
    }
}

fn scrub_env(cmd: &mut std::process::Command) {
    for var in SCRUBBED_ENV {
        cmd.env_remove(var);
    }
}

/// Locate the real `git`: the first PATH entry whose `git` does not resolve back
/// to this binary (the wrapper symlink). Canonicalization defeats the symlink so
/// we never exec ourselves.
fn find_real_git() -> Option<PathBuf> {
    let self_canon = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    let git_name = if cfg!(windows) { "git.exe" } else { "git" };

    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join(git_name);
        if !candidate.is_file() {
            continue;
        }
        let cand_canon = candidate.canonicalize().ok();
        if cand_canon.is_some() && cand_canon == self_canon {
            continue; // this is our own wrapper symlink
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = std::fs::metadata(&candidate)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if !executable {
                continue;
            }
        }
        return Some(candidate);
    }
    None
}

/// Build the real-git argv with the authoritative identity/signing config
/// injected as command-line `-c key=value` options placed immediately before
/// the subcommand — i.e. after every global option the caller passed.
///
/// Command-line `-c` is git's highest-precedence configuration channel: it wins
/// over repo/global/system config files, over the `GIT_CONFIG_*` environment,
/// over `GIT_CONFIG_PARAMETERS`, and over `-c include.path=…`/`includeIf`
/// includes (whose settings enter at the position of their own `-c`, which the
/// caller can only place *before* ours). Placing our entries last among the
/// globals therefore makes them win regardless of what config channel the agent
/// used — the whole class of "some other channel outranks the appended env"
/// bypasses — without the wrapper having to enumerate or reject those channels.
///
/// Author/committer env vars and the command-line `--author`/`--reset-author`/
/// `--no-gpg-sign`/`-c <protected>` forms outrank even command-line `-c`; those
/// are handled separately (scrubbed and rejected in [`enforce`]).
fn inject_identity_args(argv: &[String], authority: Option<&Authority>) -> Vec<String> {
    let Some(authority) = authority else {
        return argv.to_vec();
    };
    // Splice point: the subcommand index (first non-option token), or the end
    // for a bare `git`/`git --version`-style call where the position is moot.
    let at = split_globals(argv).1.unwrap_or(argv.len());
    let mut out = argv[..at].to_vec();
    for (key, value) in &authority.entries {
        out.push("-c".to_string());
        out.push(format!("{key}={value}"));
    }
    out.extend_from_slice(&argv[at..]);
    out
}

#[cfg(unix)]
fn exec_real_git(real_git: &Path, argv: &[String], authority: Option<&Authority>) -> i32 {
    use std::os::unix::process::CommandExt;
    let full = inject_identity_args(argv, authority);
    let mut cmd = std::process::Command::new(real_git);
    cmd.args(&full);
    scrub_env(&mut cmd);
    // exec replaces this process; on success it never returns. If it returns,
    // the exec itself failed.
    let err = cmd.exec();
    eprintln!("buzz git wrapper: failed to exec real git: {err}");
    127
}

#[cfg(not(unix))]
fn exec_real_git(real_git: &Path, argv: &[String], authority: Option<&Authority>) -> i32 {
    let full = inject_identity_args(argv, authority);
    let mut cmd = std::process::Command::new(real_git);
    cmd.args(&full);
    scrub_env(&mut cmd);
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("buzz git wrapper: failed to run real git: {e}");
            127
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::TestEnv;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    // ── Ambient GIT_CONFIG_* isolation ───────────────────────────────────────
    //
    // The agent harness injects up to 10 GIT_CONFIG_* env vars (including
    // `commit.gpgSign=true` and `gpg.x509.program=git-sign-nostr`) into the
    // test process env.  Tests that create real `git commit` objects via
    // subprocess must clear those vars so that the repo-level signing config
    // controls signing rather than the ambient harness config.
    //
    // The lock serialises the snapshot/clear/test/restore cycle so that
    // concurrent nextest threads cannot observe each other's mid-test env state.
    // Tests that use a self-contained stub signer should ALSO acquire this guard
    // (so the stub, not the ambient program, is used for signing).

    /// A guard that clears `GIT_CONFIG_*` and signing-identity env vars for the
    /// duration of a test.  Backed by the crate-wide `TestEnv`/`ENV_LOCK` so
    /// the entire snapshot/clear/test/restore lifetime is serialised under one
    /// mutex together with all sibling tests that use `TestEnv::lock()`.
    ///
    /// `saved` holds dynamically-discovered keys whose values are not tracked by
    /// the `TestEnv` itself (it only tracks statically-named keys).  `Drop`
    /// restores these pairs while the `TestEnv` lock is still held, then the
    /// `_env` field releases the lock.
    #[cfg(unix)]
    struct GitConfigEnvGuard {
        /// Dynamic `GIT_CONFIG_*` / signing-key pairs not tracked by `_env`.
        saved: Vec<(std::ffi::OsString, std::ffi::OsString)>,
        /// Crate-wide env lock — released AFTER `saved` is restored in `Drop`.
        _env: TestEnv,
    }

    #[cfg(unix)]
    impl Drop for GitConfigEnvGuard {
        fn drop(&mut self) {
            // Restore dynamic vars first, while `_env` (and thus ENV_LOCK)
            // is still held.  `_env` is dropped after this method returns,
            // releasing the lock only once both sets of vars are back.
            for (k, v) in self.saved.drain(..) {
                std::env::set_var(k, v);
            }
        }
    }

    /// Clear ambient `GIT_CONFIG_*` and signing-identity env vars for the
    /// duration of a test.  Acquires the crate-wide `ENV_LOCK` (via
    /// `TestEnv::lock()`) so the full snapshot/clear/test/restore cycle is
    /// serialised under one mutex with all other env-mutating tests.  Bind the
    /// returned guard to `_guard` (not `_`) so it lives for the full test scope.
    ///
    /// Cleared dynamically: `GIT_CONFIG_*` (injected git config),
    /// `BUZZ_PRIVATE_KEY` and `NOSTR_PRIVATE_KEY` (loaded by `git-sign-nostr`
    /// before `nostr.keyfile` config — without clearing these, `git-sign-nostr`
    /// uses the harness key instead of the test-vector key even when
    /// `GIT_CONFIG_*` is clean), and `BUZZ_AUTH_TAG` (a harness-signed owner
    /// attestation bound to the harness key — present with a test-vector key it
    /// makes `git-sign-nostr` abort signing with an auth-tag mismatch error).
    #[cfg(unix)]
    fn clear_git_config_env() -> GitConfigEnvGuard {
        let _env = TestEnv::lock();
        let saved: Vec<_> = std::env::vars_os()
            .filter(|(k, _)| {
                k.to_str().is_some_and(|k| {
                    k.starts_with("GIT_CONFIG_")
                        || k == "BUZZ_PRIVATE_KEY"
                        || k == "NOSTR_PRIVATE_KEY"
                        || k == "BUZZ_AUTH_TAG"
                })
            })
            .collect();
        for (k, _) in &saved {
            std::env::remove_var(k);
        }
        GitConfigEnvGuard { saved, _env }
    }

    const AGENT_EMAIL: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@relay.test";
    /// The pubkey `AGENT_EMAIL` encodes — the `user.signingkey` a valid managed
    /// manifest must name.
    const AGENT_PUBKEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// A managed-session authority with the COMPLETE signing contract — the
    /// state [`Authority::classify`] accepts as `Managed`. Used by the push
    /// tests: `verify_push` requires a signature by the agent key on every
    /// agent-authored commit, and these entries are the authority that check
    /// resolves against.
    fn managed() -> Authority {
        Authority {
            entries: vec![
                ("user.name".into(), "Agent".into()),
                ("user.email".into(), AGENT_EMAIL.into()),
                ("gpg.format".into(), "x509".into()),
                ("gpg.x509.program".into(), "git-sign-nostr".into()),
                ("commit.gpgSign".into(), "true".into()),
                ("tag.gpgSign".into(), "true".into()),
                ("user.signingkey".into(), AGENT_PUBKEY.into()),
                ("nostr.keyfile".into(), "/tmp/.nostr-key".into()),
            ],
            email: AGENT_EMAIL.into(),
        }
    }

    /// A synthetic authority with signing DISABLED, used ONLY by the exec/inject
    /// tests that create a real `git commit` (CI has no nostr signer, so signing
    /// must be off or the commit fails). This is NOT a state
    /// [`Authority::classify`] would ever return — a real manifest with
    /// `commit.gpgSign=false` is `Tampered` — it is a hand-built fixture for
    /// exercising `inject_identity_args` in isolation from the push gate.
    fn managed_nosign() -> Authority {
        Authority {
            entries: vec![
                ("user.name".into(), "Agent".into()),
                ("user.email".into(), AGENT_EMAIL.into()),
                ("commit.gpgSign".into(), "false".into()),
            ],
            email: AGENT_EMAIL.into(),
        }
    }

    // ── enforce: only enforces in a managed session ───────────────────────────

    #[test]
    fn enforce_is_a_noop_without_an_authority() {
        // An unmanaged session (no manifest) must not reject anything.
        for argv in [
            v(&["-c", "user.email=evil@x", "commit"]),
            v(&["commit", "--author=Evil <e@x>"]),
            v(&["commit", "--no-gpg-sign"]),
        ] {
            assert!(
                enforce(&argv, None).is_ok(),
                "unmanaged must allow {argv:?}"
            );
        }
    }

    // ── enforce: -c identity/signing rejection ────────────────────────────────

    #[test]
    fn rejects_dash_c_protected_keys_in_global_position() {
        let a = managed();
        for argv in [
            v(&["-c", "user.name=Evil", "commit"]),
            v(&["-c", "user.email=evil@x.com", "commit"]),
            v(&["-cuser.name=Evil", "commit"]), // attached form
            v(&["-cuser.email=e@x", "commit"]), // attached form
            v(&["-c", "USER.EMAIL=e@x", "commit"]), // case-insensitive key
            v(&["-c", "commit.gpgSign=false", "commit"]), // signing disable (F)
            v(&["-c", "user.signingkey=abc", "commit"]),
            v(&["-c", "nostr.keyfile=/tmp/evil", "commit"]),
            v(&["-c", "gpg.x509.program=/bin/false", "commit"]),
        ] {
            assert!(enforce(&argv, Some(&a)).is_err(), "must reject {argv:?}");
        }
    }

    #[test]
    fn allows_dash_c_for_unrelated_config_keys() {
        let a = managed();
        for argv in [
            v(&["-c", "core.pager=less", "log"]),
            v(&["-c", "http.proxy=x", "fetch"]),
        ] {
            assert!(enforce(&argv, Some(&a)).is_ok(), "must allow {argv:?}");
        }
    }

    #[test]
    fn commit_dash_c_reuse_message_is_not_a_config_override() {
        // `git commit -c <commit>` reuses a message; `-c` here is a commit
        // option, not the global config flag. It must not be misread as one.
        let a = managed();
        assert!(enforce(&v(&["commit", "-c", "HEAD~1"]), Some(&a)).is_ok());
        assert!(enforce(&v(&["commit", "-cuser.name=x"]), Some(&a)).is_ok());
    }

    // ── enforce: --config-env rejection ───────────────────────────────────────

    #[test]
    fn rejects_config_env_for_protected_keys() {
        let a = managed();
        assert!(enforce(&v(&["--config-env=user.name=VAR", "commit"]), Some(&a)).is_err());
        assert!(enforce(&v(&["--config-env=user.email=VAR", "commit"]), Some(&a)).is_err());
        assert!(enforce(&v(&["--config-env=commit.gpgSign=VAR", "commit"]), Some(&a)).is_err());
    }

    #[test]
    fn allows_config_env_for_unrelated_keys() {
        let a = managed();
        assert!(enforce(&v(&["--config-env=http.proxy=PROXY", "fetch"]), Some(&a)).is_ok());
    }

    // ── enforce: --author / --reset-author / --no-gpg-sign scoping ────────────

    #[test]
    fn rejects_author_overrides_on_commit_and_am() {
        let a = managed();
        for argv in [
            v(&["commit", "--author=Evil <e@x>"]),
            v(&["commit", "--author", "Evil <e@x>"]),
            v(&["commit", "--reset-author"]),
            v(&["am", "--author=Evil <e@x>"]),
        ] {
            assert!(enforce(&argv, Some(&a)).is_err(), "must reject {argv:?}");
        }
    }

    #[test]
    fn rejects_no_gpg_sign_on_signing_subcommands() {
        let a = managed();
        for argv in [
            v(&["commit", "--no-gpg-sign"]),
            v(&["tag", "-a", "v1", "--no-gpg-sign"]),
            v(&["rebase", "--no-gpg-sign", "main"]),
        ] {
            assert!(enforce(&argv, Some(&a)).is_err(), "must reject {argv:?}");
        }
    }

    #[test]
    fn allows_author_filter_on_read_side_subcommands() {
        // log/shortlog/blame --author are legitimate read filters.
        let a = managed();
        for argv in [
            v(&["log", "--author=Duncan"]),
            v(&["shortlog", "--author", "Duncan"]),
            v(&["log", "--no-gpg-sign"]), // not a signing subcommand → allowed
        ] {
            assert!(enforce(&argv, Some(&a)).is_ok(), "must allow {argv:?}");
        }
    }

    #[test]
    fn author_override_after_global_options_is_still_rejected() {
        let a = managed();
        assert!(enforce(
            &v(&["-C", "/repo", "commit", "--author=Evil <e@x>"]),
            Some(&a)
        )
        .is_err());
    }

    // ── split_globals / subcommand ────────────────────────────────────────────

    #[test]
    fn split_globals_locates_subcommand_after_value_consuming_options() {
        assert_eq!(subcommand(&v(&["commit"])).as_deref(), Some("commit"));
        assert_eq!(
            subcommand(&v(&["-C", "/repo", "-c", "core.x=y", "push"])).as_deref(),
            Some("push")
        );
        assert_eq!(
            subcommand(&v(&["--git-dir", "/g", "status"])).as_deref(),
            Some("status")
        );
        assert_eq!(subcommand(&v(&["--version"])), None);
        assert_eq!(subcommand(&[]), None);
    }

    // ── parse_porcelain_sources ───────────────────────────────────────────────

    #[test]
    fn porcelain_parse_extracts_update_sources_and_skips_deletes_and_headers() {
        let stdout = "To ../remote.git\n\
             \trefs/heads/main:refs/heads/main\t4ab76d3..c0bab62\n\
            *\trefs/heads/newbr:refs/heads/newbr\t[new branch]\n\
            =\trefs/heads/up:refs/heads/up\t[up to date]\n\
            -\t:refs/heads/tokill\t[deleted]\n\
            Done\n";
        assert_eq!(
            parse_porcelain_sources(stdout),
            vec!["refs/heads/main", "refs/heads/newbr", "refs/heads/up"]
        );
    }

    #[test]
    fn porcelain_parse_ignores_lines_without_a_refspec_field() {
        // Header/trailer and any stray non-tab lines contribute nothing.
        assert!(parse_porcelain_sources("To origin\nDone\n").is_empty());
        assert!(parse_porcelain_sources("").is_empty());
    }

    #[test]
    fn porcelain_destination_reads_the_to_header() {
        let stdout = "To ../remote.git\n\
             \trefs/heads/main:refs/heads/main\t4ab76d3..c0bab62\n\
            Done\n";
        assert_eq!(
            parse_porcelain_destination_unique(stdout).as_deref(),
            Some("../remote.git")
        );
        // A named remote resolves to a URL on the To line; whatever git prints is
        // the forge-proof handle we hand to ls-remote.
        assert_eq!(
            parse_porcelain_destination_unique("To git@example.com:o/r.git\nDone\n").as_deref(),
            Some("git@example.com:o/r.git")
        );
        // No To line (unexpected shape) → None, and the caller fails closed.
        assert!(parse_porcelain_destination_unique("Done\n").is_none());
        assert!(parse_porcelain_destination_unique("").is_none());
    }

    /// Multiple `To` headers (multiple `pushurl`s) → `None` (refused in managed mode).
    /// The A-has-HEAD/B-empty bypass requires two destinations; refusing multiple
    /// destinations closes it.
    #[test]
    fn porcelain_destination_unique_rejects_multiple_to_headers() {
        // Two To lines → None.
        let two = "To /tmp/a.git\nTo /tmp/b.git\n\
                   \trefs/heads/main:refs/heads/main\tnew-branch\n\
                   Done\n";
        assert!(
            parse_porcelain_destination_unique(two).is_none(),
            "two To headers must yield None (refused)"
        );
        assert_eq!(count_porcelain_destinations(two), 2);
    }

    /// Porcelain output with a CR in the `To` payload is refused (fail closed),
    /// not silently normalized to a different path. A trailing-space URL is
    /// preserved verbatim. Two `To` headers where one is CR-bearing must also
    /// be refused (not filtered to one clean destination — that would recreate
    /// the multi-destination bypass).
    #[test]
    fn porcelain_destination_refuses_cr_payload_and_preserves_trailing_space() {
        // CR in the To payload → None (fail closed, not normalized).
        let cr_payload = "To /tmp/path/ending/in/\r\n*\trefs/heads/main:refs/heads/main\tDone\n";
        assert!(
            parse_porcelain_destination_unique(cr_payload).is_none(),
            "CR in destination payload must be refused"
        );

        // Trailing space in the URL is preserved byte-for-byte (legitimate URL).
        let space_url = "To /tmp/a dir/repo.git\n*\trefs/heads/main:refs/heads/main\tDone\n";
        assert_eq!(
            parse_porcelain_destination_unique(space_url).as_deref(),
            Some("/tmp/a dir/repo.git"),
            "trailing-space URL must be preserved verbatim"
        );

        // Two To headers where ONE contains CR: must refuse the whole plan,
        // not filter the CR one out and accept the clean one — filtering would
        // recreate the multi-destination bypass.
        let two_to_one_cr =
            "To /tmp/a.git\r\nTo /tmp/b.git\n*\trefs/heads/main:refs/heads/main\tDone\n";
        assert!(
            parse_porcelain_destination_unique(two_to_one_cr).is_none(),
            "two To headers with one CR-bearing must refuse the whole plan, not filter to one"
        );
    }

    /// `is_receive_pack_or_exec_flag` must accept all git-recognized
    /// abbreviations of `--receive-pack` and `--exec`, and not false-positive
    /// on unrelated flags.
    #[test]
    fn is_receive_pack_or_exec_flag_covers_all_accepted_abbreviations() {
        // --receive-pack family: shortest unique prefix is --rece (not --rec,
        // which is ambiguous with --recurse-submodules and rejected by git).
        for flag in [
            "--rece",
            "--rece=cmd",
            "--recei",
            "--recei=cmd",
            "--receiv",
            "--receiv=cmd",
            "--receive",
            "--receive=cmd",
            "--receive-",
            "--receive-p",
            "--receive-p=cmd",
            "--receive-pack",
            "--receive-pack=cmd",
        ] {
            assert!(
                is_receive_pack_or_exec_flag(flag),
                "{flag:?} must be detected as receive-pack/exec"
            );
        }
        // --exec family: shortest is --e / --e=<cmd>.
        for flag in [
            "--e",
            "--e=cmd",
            "--ex",
            "--ex=cmd",
            "--exe",
            "--exe=cmd",
            "--exec",
            "--exec=cmd",
        ] {
            assert!(
                is_receive_pack_or_exec_flag(flag),
                "{flag:?} must be detected as receive-pack/exec"
            );
        }
        // Unrelated flags must not be rejected.
        for flag in [
            "--dry-run",
            "--porcelain",
            "--force",
            "--all",
            "--tags",
            "--delete",
            "--no-verify",
            "--quiet",
            "-u",
            "origin",
            "main",
            "--recurse-submodules=check",
        ] {
            assert!(
                !is_receive_pack_or_exec_flag(flag),
                "{flag:?} must NOT be detected as receive-pack/exec"
            );
        }
    }

    // ── reuse_commit_arg (E) ──────────────────────────────────────────────────

    #[test]
    fn reuse_commit_arg_detects_c_and_capital_c_forms() {
        assert_eq!(
            reuse_commit_arg(&v(&["-C", "HEAD~1"])).as_deref(),
            Some("HEAD~1")
        );
        assert_eq!(
            reuse_commit_arg(&v(&["-c", "abc123"])).as_deref(),
            Some("abc123")
        );
        assert_eq!(reuse_commit_arg(&v(&["-CHEAD"])).as_deref(), Some("HEAD"));
        // A `-c key=val` config value is not a commit reuse.
        assert_eq!(reuse_commit_arg(&v(&["-cuser.name=x"])), None);
        assert_eq!(reuse_commit_arg(&v(&["-m", "msg"])), None);
    }

    // ── caller_globals ────────────────────────────────────────────────────────

    #[test]
    fn caller_globals_captures_the_complete_global_set() {
        // Every global before the subcommand is captured — repo-context
        // (`-C`/`--git-dir`), config channels (`-c`/`--config-env`, attached +
        // split), AND repository-selection flags an allowlist would drop
        // (`--bare`) — with value tokens paired in and the subcommand excluded.
        assert_eq!(
            caller_globals(&v(&[
                "-C",
                "/repo",
                "--bare",
                "-c",
                "alias.x=push",
                "-cinclude.path=/e",
                "--config-env=alias.y=VAR",
                "--config-env",
                "alias.z=VAR2",
                "--git-dir=/g",
                "pub",
            ])),
            v(&[
                "-C",
                "/repo",
                "--bare",
                "-c",
                "alias.x=push",
                "-cinclude.path=/e",
                "--config-env=alias.y=VAR",
                "--config-env",
                "alias.z=VAR2",
                "--git-dir=/g",
            ])
        );
        // No globals before the subcommand → empty.
        assert!(caller_globals(&v(&["push"])).is_empty());
        // `-C <dir>` pairs its value; the subcommand is never captured.
        assert_eq!(
            caller_globals(&v(&["-C", "/repo", "push"])),
            v(&["-C", "/repo"])
        );
    }

    #[test]
    fn split_globals_pins_every_git_2_54_separate_value_global() {
        // `split_globals` is the single point of truth for where the subcommand
        // begins; every alias/push probe resolves under the globals it extracts.
        // A separate-value global git honors but the table omits desyncs the
        // probe from the real invocation (round-7 `--shallow-file`). Pin the
        // complete git 2.54 `handle_options()` set: each must consume its
        // following token so the *next* token is the subcommand.
        //
        // git.c v2.54.0: --git-dir, --work-tree, --namespace, --config-env,
        // --attr-source, --shallow-file are the separate-value globals; `-C` and
        // `-c` are the short-option pair. Re-audit when bumping git.
        for opt in [
            "--git-dir",
            "--work-tree",
            "--namespace",
            "--config-env",
            "--attr-source",
            "--shallow-file",
        ] {
            let (globals, sub_idx) = split_globals(&v(&[opt, "VALUE", "status"]));
            assert_eq!(
                globals,
                v(&[opt, "VALUE"]),
                "{opt} must consume its following token as a value"
            );
            assert_eq!(
                sub_idx,
                Some(2),
                "{opt} must leave `status` (index 2) as the subcommand"
            );
        }
        // The short-option pair `-c`/`-C` likewise consumes a value.
        for opt in ["-c", "-C"] {
            let (_, sub_idx) = split_globals(&v(&[opt, "VALUE", "status"]));
            assert_eq!(sub_idx, Some(2), "{opt} must consume its value token");
        }
    }

    // ── manifest round-trip ───────────────────────────────────────────────────

    #[test]
    fn authority_loads_identity_and_email_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![
            ("user.name".to_string(), "Agent".to_string()),
            ("user.email".to_string(), AGENT_EMAIL.to_string()),
            ("commit.gpgSign".to_string(), "true".to_string()),
        ];
        crate::write_identity_manifest(dir.path(), &entries).unwrap();
        let parsed = crate::read_identity_manifest(dir.path()).unwrap();
        assert_eq!(parsed, entries);
        let email = parsed
            .iter()
            .find(|(k, _)| k == "user.email")
            .map(|(_, v)| v.clone());
        assert_eq!(email.as_deref(), Some(AGENT_EMAIL));
    }

    #[test]
    fn read_identity_manifest_is_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(crate::read_identity_manifest(dir.path()).is_none());
    }

    // ── is_push_command / verify_push / verify_commit_author against real git ──

    /// Build a repo whose HEAD carries a deliberately human-authored commit and
    /// return `(tempdir, repo_path)`. No remote, so `--not --remotes` yields the
    /// full history — the commit shows as outgoing.
    fn human_authored_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Human"]);
        git(&["config", "user.email", "human@example.com"]);
        git(&["config", "commit.gpgSign", "false"]);
        git(&["config", "alias.pub", "push"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        git(&["add", "f"]);
        git(&["commit", "-qm", "human commit"]);
        (dir, repo)
    }

    fn real_git() -> PathBuf {
        PathBuf::from("git")
    }

    #[test]
    fn config_alias_resolving_to_push_is_recognized() {
        let (_d, repo) = human_authored_repo();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // `git pub` → alias.pub = push.
        assert!(matches!(
            is_push_command(
                &real_git(),
                &v(&["-C", repo.to_str().unwrap(), "pub"]),
                &ctx
            ),
            PushKind::Push
        ));
        // A non-push subcommand is not misclassified.
        assert!(matches!(
            is_push_command(
                &real_git(),
                &v(&["-C", repo.to_str().unwrap(), "status"]),
                &ctx
            ),
            PushKind::NotPush
        ));
    }

    #[test]
    fn inline_alias_resolving_to_push_is_recognized() {
        let argv = v(&["-c", "alias.pub=push", "pub"]);
        let ctx = caller_globals(&argv);
        assert!(matches!(
            is_push_command(&real_git(), &argv, &ctx),
            PushKind::Push
        ));
    }

    // ── verify_alias_safety (allowlist) ───────────────────────────────────────

    #[test]
    fn is_safe_alias_token_admits_only_trivial_bare_words() {
        // Safe: bare subcommands and plain arguments.
        assert!(is_safe_alias_token("commit"));
        assert!(is_safe_alias_token("status"));
        assert!(is_safe_alias_token("--oneline"));
        assert!(is_safe_alias_token("origin"));
        assert!(is_safe_alias_token("main"));
        // Unsafe: config channels in any spelling.
        assert!(!is_safe_alias_token("-c"));
        assert!(!is_safe_alias_token("-cuser.email=x"));
        assert!(!is_safe_alias_token("--config-env"));
        assert!(!is_safe_alias_token("--config-env=user.name=VAR"));
        // Unsafe: any quote/escape (git dequotes; we never guess).
        assert!(!is_safe_alias_token("'-c'"));
        assert!(!is_safe_alias_token("\"commit\""));
        assert!(!is_safe_alias_token("a\\b"));
        // Unsafe: a value-bearing option.
        assert!(!is_safe_alias_token("--author=Evil <e@x>"));
    }

    #[test]
    fn verify_alias_safety_rejects_config_and_quoted_aliases() {
        // Each shape carries its alias definition inline (`-c alias.<n>=…`), so
        // the probe context is the caller's own globals — exactly as `run`
        // derives it via `caller_globals`.
        let refused = |argv: Vec<String>| {
            let ctx = caller_globals(&argv);
            assert!(verify_alias_safety(&real_git(), &argv, &ctx).is_err());
        };
        // Bare `-c` config channel.
        refused(v(&["-c", "alias.hc=-c user.email=e@x commit", "hc"]));
        // `--config-env` channel.
        refused(v(&["-c", "alias.hc=--config-env=user.name=V commit", "hc"]));
        // Quoted tokens — the parser-parity bypass; refused without dequoting.
        refused(v(&["-c", "alias.q='-c' 'user.email=q@x' commit", "q"]));
    }

    #[test]
    fn verify_alias_safety_rejects_all_shell_aliases() {
        let refused = |argv: Vec<String>| {
            let ctx = caller_globals(&argv);
            assert!(verify_alias_safety(&real_git(), &argv, &ctx).is_err());
        };
        // A shell alias with no push and no config is still refused in managed mode.
        refused(v(&["-c", "alias.sh=!git status", "sh"]));
        // The commit-path shell bypass Thufir demonstrated.
        refused(v(&[
            "-c",
            "alias.sc=!f(){ git -c user.email=shell@x commit \"$@\"; }; f",
            "sc",
        ]));
    }

    #[test]
    fn verify_alias_safety_allows_bare_word_aliases() {
        // Gurney's certified working shapes must all stay allowed, and resolve to
        // their expansion so the caller can hold it to the direct-command policy.
        // The probe runs under `caller_globals(argv)`, exactly as `run` derives it.
        let expands = |argv: Vec<String>, expected: Option<Vec<String>>| {
            let ctx = caller_globals(&argv);
            assert_eq!(
                verify_alias_safety(&real_git(), &argv, &ctx).unwrap(),
                expected
            );
        };
        expands(
            v(&["-c", "alias.ci=commit", "ci"]),
            Some(v(&["-c", "alias.ci=commit", "commit"])),
        );
        expands(
            v(&["-c", "alias.st=status", "st"]),
            Some(v(&["-c", "alias.st=status", "status"])),
        );
        expands(
            v(&["-c", "alias.lg=log --oneline", "lg"]),
            Some(v(&["-c", "alias.lg=log --oneline", "log", "--oneline"])),
        );
        expands(
            v(&["-c", "alias.pub=push origin main", "pub"]),
            Some(v(&[
                "-c",
                "alias.pub=push origin main",
                "push",
                "origin",
                "main",
            ])),
        );
        // A real (non-alias) subcommand resolves immediately with no expansion.
        expands(v(&["commit", "-m", "x"]), None);
    }

    #[test]
    fn verify_alias_safety_expands_bare_word_flags_and_appends_trailing_argv() {
        // Thufir's rd-4 bypass shape: every body token is a bare word, so the
        // allowlist admits it — but the returned expansion carries the flags and
        // the caller's trailing argv, so the direct-command preflight can catch
        // `--author`/`--no-gpg-sign`. This is the unification contract.
        let human = v(&[
            "-c",
            "alias.human=commit --author Human<h@x> --no-gpg-sign",
            "human",
            "-m",
            "leak",
        ]);
        assert_eq!(
            verify_alias_safety(&real_git(), &human, &caller_globals(&human)).unwrap(),
            Some(v(&[
                "-c",
                "alias.human=commit --author Human<h@x> --no-gpg-sign",
                "commit",
                "--author",
                "Human<h@x>",
                "--no-gpg-sign",
                "-m",
                "leak",
            ]))
        );
        // A chain accumulates body tokens across hops onto the final command.
        let chain = v(&[
            "-c",
            "alias.chain=co --no-gpg-sign",
            "-c",
            "alias.co=commit",
            "chain",
        ]);
        assert_eq!(
            verify_alias_safety(&real_git(), &chain, &caller_globals(&chain)).unwrap(),
            Some(v(&[
                "-c",
                "alias.chain=co --no-gpg-sign",
                "-c",
                "alias.co=commit",
                "commit",
                "--no-gpg-sign",
            ]))
        );
    }

    #[test]
    fn verify_alias_safety_walks_bare_word_chains_and_rejects_config_at_the_end() {
        // `a` → `b` (both bare-word) → allowed.
        let ok = v(&["-c", "alias.a=b", "-c", "alias.b=commit", "a"]);
        assert!(verify_alias_safety(&real_git(), &ok, &caller_globals(&ok)).is_ok());
        // `a` → `b` where `b` introduces config → refused via the chain.
        let bad = v(&[
            "-c",
            "alias.a=b",
            "-c",
            "alias.b=-c commit.gpgSign=false commit",
            "a",
        ]);
        assert!(verify_alias_safety(&real_git(), &bad, &caller_globals(&bad)).is_err());
    }

    // ── caller-config-introduced aliases: the probe must resolve the exact
    //    alias set git will, so include.path / --config-env / case-varied `-c`
    //    definitions cannot smuggle a shell alias past the safety check or a
    //    push past outgoing-author verification. Each shape is exercised through
    //    the real wrapper functions (`real_git`), in commit and push variants.

    /// Write a git config file defining the given `alias.<name> = <body>` pairs
    /// and return its absolute path (kept alive by the returned tempdir).
    fn alias_include_file(aliases: &[(&str, &str)]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evil.cfg");
        let mut body = String::from("[alias]\n");
        for (name, def) in aliases {
            body.push_str(&format!("\t{name} = {def}\n"));
        }
        std::fs::write(&path, body).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    #[test]
    fn include_path_shell_alias_is_refused_commit_variant() {
        // `git -c include.path=<f> x` where the included file defines a shell
        // alias `x = !git … commit …`. The probe now resolves under the caller's
        // `-c include.path`, sees the `!` body, and refuses — closing the
        // round-6 smuggling path for the commit case.
        let (_d, inc) =
            alias_include_file(&[("x", "!git -c user.email=evil@x.com commit --no-gpg-sign")]);
        let argv = v(&["-c", &format!("include.path={inc}"), "x"]);
        let err = verify_alias_safety(&real_git(), &argv, &caller_globals(&argv))
            .expect_err("include.path-introduced shell alias must be refused");
        assert!(err.contains("shell (`!`) git alias"), "{err}");
    }

    #[test]
    fn include_path_alias_to_push_is_recognized_push_variant() {
        // A non-shell alias introduced via include.path that resolves to push
        // must be classified as a push so outgoing-author verification runs.
        let (_d, inc) = alias_include_file(&[("x", "push origin main")]);
        let argv = v(&["-c", &format!("include.path={inc}"), "x"]);
        assert!(matches!(
            is_push_command(&real_git(), &argv, &caller_globals(&argv)),
            PushKind::Push
        ));
    }

    #[test]
    fn config_env_shell_alias_is_refused_commit_variant() {
        // `--config-env=alias.x=VAR` sources the alias body from an env var. The
        // probe inherits the process env, so it resolves the alias git would.
        let mut env = TestEnv::lock();
        env.set(
            "BUZZ_TEST_EVIL_ALIAS",
            "!git -c user.email=evil@x.com commit",
        );
        let argv = v(&["--config-env=alias.x=BUZZ_TEST_EVIL_ALIAS", "x"]);
        let err = verify_alias_safety(&real_git(), &argv, &caller_globals(&argv))
            .expect_err("--config-env shell alias must be refused");
        assert!(err.contains("shell (`!`) git alias"), "{err}");
    }

    #[test]
    fn config_env_alias_to_push_is_recognized_push_variant() {
        let mut env = TestEnv::lock();
        env.set("BUZZ_TEST_PUSH_ALIAS", "push origin main");
        let argv = v(&["--config-env=alias.x=BUZZ_TEST_PUSH_ALIAS", "x"]);
        let kind = is_push_command(&real_git(), &argv, &caller_globals(&argv));
        assert!(matches!(kind, PushKind::Push));
    }

    #[test]
    fn case_varied_dash_c_alias_is_resolved() {
        // Git normalizes config section names, so `-c ALIAS.x=…` defines
        // `alias.x`. The old hand-rolled `strip_prefix("alias.")` matcher was
        // case-sensitive and missed this; resolving through git closes it.
        // Push variant: `-c ALIAS.x=push x` classifies as push.
        let push_argv = v(&["-c", "ALIAS.x=push", "x"]);
        assert!(matches!(
            is_push_command(&real_git(), &push_argv, &caller_globals(&push_argv)),
            PushKind::Push
        ));
        // Commit variant: a case-varied shell alias is refused.
        let shell_argv = v(&["-c", "ALIAS.x=!git commit", "x"]);
        let err = verify_alias_safety(&real_git(), &shell_argv, &caller_globals(&shell_argv))
            .expect_err("case-varied shell alias must be refused");
        assert!(err.contains("shell (`!`) git alias"), "{err}");
    }

    #[test]
    fn legitimate_repo_and_inline_aliases_still_work() {
        // Regression guard: closing the visibility gap must not break the
        // ordinary shapes. A repo-config `alias.pub = push` (set by
        // `human_authored_repo`) resolves through the `-C` context, an inline
        // `-c alias.ci=commit` still resolves, and a bare-word chain expands.
        let (_d, repo) = human_authored_repo();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        assert!(matches!(
            is_push_command(
                &real_git(),
                &v(&["-C", repo.to_str().unwrap(), "pub"]),
                &ctx
            ),
            PushKind::Push
        ));
        let inline = v(&["-c", "alias.ci=commit", "ci"]);
        assert_eq!(
            verify_alias_safety(&real_git(), &inline, &caller_globals(&inline)).unwrap(),
            Some(v(&["-c", "alias.ci=commit", "commit"]))
        );
    }

    /// Build a directory with two valid but *different* repository views (the
    /// round-7 template): `<dir>/.git` is an ordinary repo carrying NO
    /// `alias.<name>`, while `<dir>` itself holds a bare repository layout that
    /// defines `alias.<name> = <body>`. `git -C <dir> config --get alias.<name>`
    /// discovers the `.git` view and sees nothing; `git -C <dir> --bare config
    /// --get alias.<name>` treats `<dir>` as the git dir and sees the alias. So
    /// an alias probe that drops `--bare` is blind to what the real `--bare`
    /// invocation will expand. Returns the tempdir (kept alive) and `<dir>`.
    fn dir_with_bare_only_alias(name: &str, body: &str) -> (tempfile::TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        let dir_str = dir.to_str().unwrap();
        // The `.git` view: an ordinary repo with no alias.
        assert!(std::process::Command::new("git")
            .args(["-C", dir_str, "init", "-q"])
            .status()
            .unwrap()
            .success());
        // The `--bare` view: a minimal bare repository layout laid directly at
        // `<dir>` (HEAD + config + empty objects/refs is all `--bare` config
        // access needs). `--git-dir <dir>` selects it regardless of the nested
        // `.git`, so the alias is written into — and read only from — this view.
        std::fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            dir.join("config"),
            "[core]\n\trepositoryformatversion = 0\n\tbare = true\n",
        )
        .unwrap();
        std::fs::create_dir(dir.join("objects")).unwrap();
        std::fs::create_dir(dir.join("refs")).unwrap();
        assert!(std::process::Command::new("git")
            .args([
                "--git-dir",
                dir_str,
                "config",
                &format!("alias.{name}"),
                body
            ])
            .status()
            .unwrap()
            .success());
        (td, dir)
    }

    #[test]
    fn bare_view_shell_alias_is_refused_commit_variant() {
        // `git -C <dir> --bare x` where `x` is a shell alias visible ONLY in the
        // `--bare` repository view. The probe context is now the caller's
        // complete global set (including `--bare`), so the probe resolves the
        // same view git will and sees the `!` body — refused. Dropping `--bare`
        // from the probe (the round-6 allowlist) made this alias invisible.
        let (_td, dir) = dir_with_bare_only_alias("x", "!git -c user.email=evil@x.com commit");
        // Sanity: in the non-bare (`.git`) view the alias does not exist, so the
        // probe finds no alias and treats `x` as a real command — proving the
        // two views genuinely diverge and this is not a trivially-present alias.
        assert_eq!(
            verify_alias_safety(&real_git(), &v(&["-C", dir.to_str().unwrap(), "x"]), &[]).unwrap(),
            None,
            "non-bare view must not resolve the bare-only alias"
        );
        let argv = v(&["-C", dir.to_str().unwrap(), "--bare", "x"]);
        let ctx = caller_globals(&argv);
        let err = verify_alias_safety(&real_git(), &argv, &ctx)
            .expect_err("bare-view shell alias must be refused");
        assert!(err.contains("shell (`!`) git alias"), "{err}");
    }

    #[test]
    fn bare_view_alias_to_push_is_recognized_push_variant() {
        // A `--bare`-only alias resolving to push must classify as push so
        // outgoing-author/signature verification runs — otherwise `git -C <dir>
        // --bare p` (p = push …) would reach the real push unverified.
        let (_td, dir) = dir_with_bare_only_alias("p", "push --no-verify origin main");
        let argv = v(&["-C", dir.to_str().unwrap(), "--bare", "p"]);
        let ctx = caller_globals(&argv);
        assert!(matches!(
            is_push_command(&real_git(), &argv, &ctx),
            PushKind::Push
        ));
        // Without `--bare` in the probe context the alias is invisible and the
        // command misclassifies as NotPush — the exact round-7 bypass.
        let blind_ctx = vec!["-C".to_string(), dir.to_string_lossy().into_owned()];
        assert!(matches!(
            is_push_command(&real_git(), &argv, &blind_ctx),
            PushKind::NotPush
        ));
    }

    #[test]
    fn shallow_file_value_shape_shell_alias_is_refused_commit_variant() {
        // Round-7 grammar desync: `git -C <repo> --shallow-file -c x`. Git 2.54
        // consumes `-c` as the `--shallow-file` VALUE and dispatches alias `x`.
        // If `split_globals` omitted `--shallow-file` it would treat `x` as the
        // value of `-c`, find no subcommand, and skip every preflight — letting
        // the shell alias through. With `--shallow-file` in the table the
        // subcommand is located at `x` and the probe (under the same globals)
        // resolves the alias and refuses it.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "alias.x", "!git -c user.email=evil@x.com commit"]);
        let repo_str = repo.to_str().unwrap();
        let argv = v(&["-C", repo_str, "--shallow-file", "-c", "x"]);
        // The subcommand must be located at `x` (index 4), proving the value
        // token `-c` was consumed by `--shallow-file`.
        assert_eq!(subcommand(&argv).as_deref(), Some("x"));
        let ctx = caller_globals(&argv);
        let err = verify_alias_safety(&real_git(), &argv, &ctx)
            .expect_err("shallow-file-shape shell alias must be refused");
        assert!(err.contains("shell (`!`) git alias"), "{err}");
    }

    #[test]
    fn shallow_file_value_shape_alias_to_push_is_recognized_push_variant() {
        // Same desync shape, push variant: `--shallow-file -c p` where `p`
        // resolves to push must classify as push so the outgoing-author gate
        // runs. A blind table (no `--shallow-file`) finds no subcommand and
        // returns NotPush — the round-7 signature-gate bypass.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "alias.p", "push --no-verify origin main"]);
        let repo_str = repo.to_str().unwrap();
        let argv = v(&["-C", repo_str, "--shallow-file", "-c", "p"]);
        let ctx = caller_globals(&argv);
        assert!(matches!(
            is_push_command(&real_git(), &argv, &ctx),
            PushKind::Push
        ));
    }

    #[test]
    fn verify_push_rejects_human_commit_via_git_resolved_plan() {
        // No remote configured: the dry-run to a bogus remote fails, so the
        // push fails closed. Point a real remote at a fresh bare repo so the
        // plan resolves and HEAD (human-authored) shows as an offender.
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        run(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        run(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let err = verify_push(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &ctx,
            &managed(),
        )
        .expect_err("human-authored HEAD must be refused");
        assert!(err.contains("not authored by your agent identity"), "{err}");
    }

    // ── Wes (5055999359) P1 regressions ─────────────────────────────────────

    /// (Wes P1) Direct `--receive-pack` flag on `push` is refused.
    /// Mutation: removing the `--receive-pack` check from
    /// `reject_receive_pack_override` must flip this test to `Ok` (precondition
    /// asserted inline).
    #[test]
    fn verify_push_rejects_receive_pack_flag() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                remote.path().to_str().unwrap(),
            ])
            .status()
            .unwrap();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // Direct --receive-pack=<cmd> form.
        let argv_rp = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--receive-pack=/bin/false",
            "origin",
            "main",
        ]);
        let err = verify_push(&real_git(), &argv_rp, &argv_rp, &ctx, &managed())
            .expect_err("--receive-pack must be refused in managed mode");
        assert!(err.contains("receive-pack"), "{err}");

        // Separate-value --receive-pack form.
        let argv_sep = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--receive-pack",
            "/bin/false",
            "origin",
            "main",
        ]);
        let err2 = verify_push(&real_git(), &argv_sep, &argv_sep, &ctx, &managed())
            .expect_err("--receive-pack (separate value) must be refused");
        assert!(err2.contains("receive-pack"), "{err2}");

        // --exec= form (alias for --receive-pack).
        let argv_exec = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--exec=/bin/false",
            "origin",
            "main",
        ]);
        let err3 = verify_push(&real_git(), &argv_exec, &argv_exec, &ctx, &managed())
            .expect_err("--exec must be refused in managed mode");
        assert!(
            err3.contains("receive-pack") || err3.contains("exec"),
            "{err3}"
        );
    }

    /// (Wes P1) An alias that expands to `push --exec evil` is refused.
    /// `verify_push` receives the EXPANDED argv as `effective_argv`, so the
    /// `--exec` token is visible even though the literal typed command is `p`.
    ///
    /// Mutation: changing `verify_push` to scan `argv` instead of `effective_argv`
    /// for receive-pack flags lets this test pass (turns `Ok`) — confirming the
    /// scan must be against the expanded form.
    #[test]
    fn verify_push_rejects_exec_carried_by_alias_via_expanded_argv() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                remote.path().to_str().unwrap(),
            ])
            .status()
            .unwrap();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // Literal argv: `git -C <repo> p` (alias name, no flags visible).
        let literal_argv = v(&["-C", repo.to_str().unwrap(), "p"]);
        // Effective (expanded) argv: what alias.p = push --exec=/bin/false expands to.
        let expanded_argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--exec=/bin/false",
            "origin",
            "main",
        ]);
        // Precondition: scanning literal argv alone would NOT catch --exec.
        assert!(
            reject_receive_pack_override(&literal_argv).is_ok(),
            "literal argv must not trigger the guard (--exec not present)"
        );
        // The guard must fire on the expanded argv.
        let err = verify_push(&real_git(), &literal_argv, &expanded_argv, &ctx, &managed())
            .expect_err("alias-carried --exec must be refused via expanded argv");
        assert!(
            err.contains("receive-pack") || err.contains("exec"),
            "{err}"
        );
    }

    /// (Wes P1 / Carl review) `remote.<name>.receivepack` in config is refused.
    ///
    /// **Positive-control redirect regression**: proves the bypass is executable,
    /// not just that the guard fires. Setup:
    ///   - `remote` = the nominal push destination (what `ls-remote` would read).
    ///     Seeded with the first commit so its OID appears "already remote" and
    ///     `partition_outgoing` would exempt it.
    ///   - `redirect` = an empty bare repo. Configured as the receive-pack
    ///     endpoint via `remote.origin.receivepack`, so the real push lands here
    ///     instead of `remote`.
    ///   - A new commit is made so its SHA is not yet on `remote`.
    ///   - A direct `git push` (bypassing `verify_push`) is issued. Git's
    ///     `ls-remote --upload-pack=git-receive-pack origin` reads `remote`
    ///     (returns the first commit's OID), but the real receive-pack is the
    ///     custom program pointing at `redirect` — so the new commit lands in
    ///     `redirect`, not `remote`. This proves the redirect is real.
    ///
    /// Mutation evidence: removing the `remote.*.receivepack` policy arm from
    /// `inspect_push_config` removes the guard; the push would then proceed and
    /// land at `redirect`, not `remote`, as the positive-control block proves.
    #[test]
    fn verify_push_rejects_receivepack_config() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let redirect = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&["init", "-q", "--bare", redirect.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);

        // Seed `remote` with the first commit so `ls-remote` would return its OID
        // (making `partition_outgoing` exempt it as "already remote").
        let first_sha = {
            let out = std::process::Command::new("git")
                .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "-q",
            "--no-verify",
            remote.path().to_str().unwrap(),
            &format!("{first_sha}:refs/heads/seed"),
        ]);

        // Configure receivepack to redirect the real push to `redirect`.
        // Git invokes the receivepack program as a shell command, passing the
        // remote-dir as an additional argument.  A wrapper script that ignores
        // the passed dir and unconditionally calls `git-receive-pack <redirect>`
        // makes every push via `origin` land in `redirect` instead of `remote`.
        let rp_script = redirect.path().join("fake_rp.sh");
        std::fs::write(
            &rp_script,
            format!(
                "#!/bin/sh\nexec git-receive-pack '{}'\n",
                redirect.path().display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&rp_script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "remote.origin.receivepack",
            rp_script.to_str().unwrap(),
        ]);

        // Positive-control: make a new commit (not yet on `remote` or `redirect`)
        // and direct-push it bypassing `verify_push`. This proves the redirect is
        // executable — the commit lands in `redirect`, not `remote`.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-q",
            "--no-verify",
            "-m",
            "redirect-test",
            "--author",
            "Human Author <human@example.com>",
        ]);
        let second_sha = {
            let out = std::process::Command::new("git")
                .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let direct_push = std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "push",
                "--no-verify",
                "-q",
                "origin",
                &format!("{second_sha}:refs/heads/redirected"),
            ])
            .status()
            .unwrap();
        assert!(
            direct_push.success(),
            "direct push with custom receivepack must succeed (redirect is live)"
        );
        // `redirect` must contain the new commit — proving the receive-pack redirect fired.
        let redirect_refs: Vec<String> = {
            let out = std::process::Command::new("git")
                .args(["ls-remote", redirect.path().to_str().unwrap()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| {
                    let mut p = l.split_whitespace();
                    let sha = p.next()?;
                    let r = p.next()?;
                    Some(format!("{sha} {r}"))
                })
                .collect()
        };
        assert!(
            redirect_refs
                .iter()
                .any(|r| r.contains("refs/heads/redirected")),
            "positive-control: new commit must have landed in redirect, not remote; \
             redirect_refs={redirect_refs:?}"
        );
        // `remote` must NOT contain the new commit (it was redirected away).
        let remote_refs: Vec<String> = {
            let out = std::process::Command::new("git")
                .args(["ls-remote", remote.path().to_str().unwrap()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| {
                    let mut p = l.split_whitespace();
                    let sha = p.next()?;
                    let r = p.next()?;
                    Some(format!("{sha} {r}"))
                })
                .collect()
        };
        assert!(
            !remote_refs
                .iter()
                .any(|r| r.contains("refs/heads/redirected")),
            "positive-control: redirected commit must NOT appear in remote (the nominal \
             ls-remote target); remote_refs={remote_refs:?}"
        );

        // Production guard: `verify_push` must refuse before any push runs.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("remote.origin.receivepack must be refused");
        assert!(
            err.contains("receivepack") || err.contains("receive-pack"),
            "expected receivepack refusal; got: {err}"
        );
    }

    /// Case-varied regression: `remote.Origin.receivePack` (capital section and
    /// variable) must be caught by the same config guard as `remote.origin.receivepack`.
    /// Proves that normalization after parsing handles case-varied output and
    /// that the ERE character-class filter captures the key.
    #[test]
    fn verify_push_rejects_case_varied_receivepack_config() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                remote.path().to_str().unwrap(),
            ])
            .status()
            .unwrap();

        // Write the key with capital casing directly into the config file so git
        // does not normalize it on write.
        let config_path = repo.join(".git/config");
        let mut cfg = std::fs::read_to_string(&config_path).unwrap();
        cfg.push_str("\n[Remote \"origin\"]\n    receivePack = evil-rp\n");
        std::fs::write(&config_path, cfg).unwrap();

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("Remote.origin.receivePack must be refused");
        assert!(
            err.contains("receivepack") || err.contains("receive-pack"),
            "expected receivepack refusal for case-varied key; got: {err}"
        );
    }

    /// Case-varied regression: `Core.sshCommand` (capital section name) must be
    /// caught by the same guard as `core.sshcommand`.
    #[test]
    fn verify_push_rejects_case_varied_sshcommand_config() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                remote.path().to_str().unwrap(),
            ])
            .status()
            .unwrap();

        // Write [Core] section with capital casing directly.
        let config_path = repo.join(".git/config");
        let mut cfg = std::fs::read_to_string(&config_path).unwrap();
        cfg.push_str("\n[Core]\n    sshCommand = /evil/ssh\n");
        std::fs::write(&config_path, cfg).unwrap();

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("Core.sshCommand must be refused");
        assert!(
            err.contains("sshcommand") || err.contains("sshCommand") || err.contains("SSH"),
            "expected sshCommand refusal for case-varied key; got: {err}"
        );
    }

    /// (Wes P1) Multiple `pushurl`s produce multiple `To` headers in
    /// and `verify_push` fails closed with the multiple-destination error.
    ///
    /// Mutation-sensitivity: this test seeds A with the offending commit (so
    /// A's object-id set would exclude HEAD if A were used as the exclusion
    /// source), leaves B empty (so B would flag HEAD as outgoing), then proves:
    ///
    /// 1. The REAL guard (multi-dest rejection) returns `Err` — the push is
    ///    refused before any object-id lookup.
    /// 2. If `parse_porcelain_destination_unique` were changed to first-only
    ///    (i.e. to return the first `To` line regardless of count), A's IDs would
    ///    exclude HEAD and the push would return `Ok` — the A-has-HEAD/B-empty
    ///    bypass is real. This is asserted by calling `partition_outgoing`
    ///    directly with A's remote ids and confirming zero offenders.
    #[test]
    fn verify_push_rejects_multiple_pushurls() {
        let (_d, repo) = human_authored_repo();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", a.path().to_str().unwrap()]);
        g(&["init", "-q", "--bare", b.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            a.path().to_str().unwrap(),
        ]);
        // Add BOTH A and B as explicit pushurls.  Once any pushurl exists, git
        // ignores `remote.origin.url` for pushes and uses only pushurls — so a
        // single `--add B` would leave only B as the push destination.  Adding A
        // first then B gives exactly two push destinations, producing two `To`
        // headers in the porcelain output.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            a.path().to_str().unwrap(),
        ]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            b.path().to_str().unwrap(),
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];

        // Precondition: the dry-run does produce 2 To headers.
        let dry_argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--dry-run",
            "--porcelain",
            "--no-verify",
            "origin",
            "main",
        ]);
        let dry_out = {
            let arg_refs: Vec<&str> = dry_argv.iter().map(String::as_str).collect();
            std::process::Command::new("git")
                .args(&arg_refs)
                .output()
                .unwrap()
        };
        let dry_stdout = String::from_utf8_lossy(&dry_out.stdout);
        assert!(
            count_porcelain_destinations(&dry_stdout) >= 2,
            "precondition: 2 pushurls must produce ≥2 To headers; got: {dry_stdout:?}"
        );

        // Seed A by pushing HEAD (the human-authored commit) to it directly
        // using real git, so A's object-id set includes HEAD.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "-q",
            a.path().to_str().unwrap(),
            "HEAD:refs/heads/main",
        ]);

        // Verify A's IDs now include HEAD — the bypass precondition.
        let a_ids = remote_object_ids(&real_git(), &ctx, a.path().to_str().unwrap())
            .expect("ls-remote A must succeed");
        let head_sha = {
            let out = std::process::Command::new("git")
                .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert!(
            a_ids.contains(&head_sha),
            "precondition: A must hold HEAD so its ID set would exclude it; head={head_sha}"
        );

        // Mutation proof: if we used A's IDs as the exclusion source (the
        // bypass), partition_outgoing would report zero offenders — HEAD is
        // "already on A" so it appears as not-outgoing.
        let (bypass_offenders, _) = partition_outgoing(
            &real_git(),
            &ctx,
            AGENT_EMAIL,
            &["HEAD".to_string()],
            &a_ids,
        )
        .expect("partition must succeed");
        assert!(
            bypass_offenders.is_empty(),
            "bypass proof: with A's IDs as exclusion, HEAD appears already-pushed \
             (zero offenders) — this is the A-has-HEAD/B-empty bypass; \
             got offenders: {bypass_offenders:?}"
        );

        // Real guard: the multi-dest check must refuse the push.
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("multiple pushurls must be refused");
        assert!(
            err.contains("multiple") || err.contains("destination"),
            "expected multiple-destination refusal; {err}"
        );

        // Executable mutation evidence: without the guard, a direct `git push
        // origin main` (bypassing verify_push) pushes to BOTH A and B.  B starts
        // empty; after the direct push B contains HEAD.  This confirms that the
        // guard is the only thing preventing the A-has-HEAD/B-empty bypass —
        // the underlying push mechanism really does populate B.
        let b_before: Vec<String> = {
            let out = std::process::Command::new("git")
                .args(["ls-remote", b.path().to_str().unwrap()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        assert!(
            b_before.is_empty(),
            "mutation precondition: B must be empty before direct push; \
             got: {b_before:?}"
        );
        // Direct push — bypasses verify_push, exercises the raw git mechanism.
        let direct_push = std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "push",
                "--no-verify",
                "-q",
                "origin",
                "main",
            ])
            .status()
            .unwrap();
        assert!(
            direct_push.success(),
            "direct push must succeed (both pushurls accept the refs)"
        );
        let b_after: Vec<String> = {
            let out = std::process::Command::new("git")
                .args(["ls-remote", b.path().to_str().unwrap()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        assert!(
            b_after.contains(&head_sha),
            "mutation evidence: B received HEAD ({head_sha}) on direct push — \
             verify_push's multi-dest guard is the sole barrier; b_after={b_after:?}"
        );
    }

    #[test]
    fn parse_remote_object_ids_accepts_empty_and_exact_records() {
        assert_eq!(parse_remote_object_ids(b""), Some(Vec::new()));
        let oid = "a".repeat(40);
        assert_eq!(
            parse_remote_object_ids(format!("{oid}\trefs/heads/main\n").as_bytes()),
            Some(vec![oid])
        );
    }

    #[test]
    fn parse_remote_object_ids_rejects_malformed_nonempty_output() {
        let oid = "a".repeat(40);
        for malformed in [
            vec![0xff],
            b"not-an-oid\trefs/heads/main\n".to_vec(),
            b"deadbeef\trefs/heads/main\n".to_vec(),
            format!("{oid} refs/heads/main\n").into_bytes(),
            b"   \n  ".to_vec(),
            format!("{oid}\t\n").into_bytes(),
            format!("{oid}\trefs/heads/main\textra\n").into_bytes(),
        ] {
            assert_eq!(
                parse_remote_object_ids(&malformed),
                None,
                "malformed inventory must fail closed: {malformed:?}"
            );
        }
    }

    #[test]
    fn remote_object_ids_accepts_empty_bare_repo() {
        let bare = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(real_git())
            .args(["init", "-q", "--bare"])
            .current_dir(bare.path())
            .status()
            .unwrap();
        assert!(status.success());
        let ctx: Vec<String> = vec![];
        let result = remote_object_ids(&real_git(), &ctx, bare.path().to_str().unwrap())
            .expect("empty bare repo ls-remote must succeed");
        assert!(result.is_empty(), "empty repo must return empty ID list");
    }

    /// (Wes P1) Chained `pushInsteadOf`→`insteadOf` rewrite: the `To` header
    /// names endpoint A (the push destination after `pushInsteadOf`), but
    /// `ls-remote --get-url A` rewrites via `insteadOf` to B.
    /// `verify_destination_stable` detects the mismatch and fails closed.
    ///
    /// Setup:
    ///   - `origin` URL → `orig` (placeholder; never contacted directly)
    ///   - `url.A.pushInsteadOf = orig` → pushes to `orig` are redirected to A
    ///   - `url.B.insteadOf = A` → `ls-remote A` (and any fetch to A) reads B
    ///   - B is seeded with offending HEAD; A starts empty
    ///
    /// Bypass shape (WITHOUT the guard):
    ///   1. `resolve_push_sources` dry-run produces `To A` (pushInsteadOf).
    ///   2. `remote_object_ids` calls `git ls-remote A`; git rewrites A → B
    ///      (insteadOf) and reads B's objects → HEAD's IDs → HEAD exempt.
    ///   3. Real push goes to A (empty) → A receives HEAD.
    ///
    /// With guard: `verify_destination_stable` calls `ls-remote --get-url A`
    ///   → returns B ≠ A → error before any object-id lookup → A stays empty.
    ///
    /// Mutation-sensitive: removing `verify_destination_stable` from
    /// `verify_push` lets the bypass succeed; A becomes populated.
    #[test]
    fn verify_push_rejects_insteadof_rewrite_replay() {
        let (_d, repo) = human_authored_repo();
        let orig = tempfile::tempdir().unwrap(); // placeholder origin URL
        let a = tempfile::tempdir().unwrap(); // push destination (pushInsteadOf)
        let b = tempfile::tempdir().unwrap(); // ls-remote destination (insteadOf)
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        // Only A and B need to be bare repos; orig is just a URL placeholder.
        g(&["init", "-q", "--bare", a.path().to_str().unwrap()]);
        g(&["init", "-q", "--bare", b.path().to_str().unwrap()]);

        let orig_url = orig.path().to_str().unwrap();
        let a_url = a.path().to_str().unwrap();
        let b_url = b.path().to_str().unwrap();
        let repo_str = repo.to_str().unwrap();

        // Wire origin → orig (the placeholder URL).
        g(&["-C", repo_str, "remote", "add", "origin", orig_url]);

        // pushInsteadOf: pushes nominally targeting `orig` are redirected to A.
        // The porcelain `To` header will show A.
        g(&[
            "-C",
            repo_str,
            "config",
            &format!("url.{a_url}.pushInsteadOf"),
            orig_url,
        ]);

        // insteadOf: any fetch/ls-remote on A is redirected to B.
        // `verify_destination_stable` will call `ls-remote --get-url A` and get B.
        g(&[
            "-C",
            repo_str,
            "config",
            &format!("url.{b_url}.insteadOf"),
            a_url,
        ]);

        // Seed B with HEAD (the offending human commit).
        // Without the guard, `ls-remote A` → B returns HEAD's IDs → exempt.
        g(&["-C", repo_str, "push", "-q", b_url, "HEAD:refs/heads/main"]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo_str, "push", "origin", "main"]);

        // Precondition 1: `ls-remote --get-url A` must return B (insteadOf applied).
        let get_url_out = {
            let mut args = ctx.clone();
            args.extend(["ls-remote", "--get-url", a_url].map(String::from));
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            std::process::Command::new("git")
                .args(&refs)
                .output()
                .unwrap()
        };
        let resolved = String::from_utf8_lossy(&get_url_out.stdout)
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        assert_eq!(
            resolved, b_url,
            "precondition: insteadOf must rewrite A → B; got {resolved:?}"
        );

        // Precondition 2: dry-run `To` header must say A (pushInsteadOf applied).
        let dry_out = {
            let dry_argv = v(&[
                "-C",
                repo_str,
                "push",
                "--dry-run",
                "--porcelain",
                "--no-verify",
                "origin",
                "main",
            ]);
            let refs: Vec<&str> = dry_argv.iter().map(String::as_str).collect();
            std::process::Command::new("git")
                .args(&refs)
                .output()
                .unwrap()
        };
        let dry_stdout = String::from_utf8_lossy(&dry_out.stdout);
        assert!(
            dry_stdout.contains(a_url),
            "precondition: porcelain To header must contain A; got {dry_stdout:?}"
        );

        // The push gate must detect the rewrite and refuse.
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("insteadOf-chained destination rewrite must be refused");
        assert!(
            err.contains("rewritten") || err.contains("insteadOf") || err.contains("destination"),
            "expected destination-rewrite refusal; {err}"
        );

        // A must be empty — refused before any object-id lookup or real push.
        let show_ref = std::process::Command::new("git")
            .args(["-C", a_url, "show-ref", "--verify", "refs/heads/main"])
            .output()
            .unwrap();
        assert!(
            !show_ref.status.success(),
            "A must be empty after refused push; show-ref found: {}",
            String::from_utf8_lossy(&show_ref.stdout),
        );
    }
    /// A forged local remote-tracking ref must not be used to hide an
    /// outgoing commit from the push gate. An agent can `git update-ref
    /// refs/remotes/forged/main HEAD` so `rev-list --not --remotes` reports zero
    /// outgoing and both verification loops accept without inspecting HEAD.
    /// Exclusions must come from the destination's real object ids (`ls-remote`),
    /// so the forged ref is irrelevant and the human-authored HEAD is still an
    /// offender.
    #[test]
    fn verify_push_ignores_forged_remote_tracking_ref() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        run(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        run(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // Forge a remote-tracking ref at HEAD. The real bare remote is empty, so
        // HEAD is genuinely outgoing — only this local ref pretends otherwise.
        run(&[
            "-C",
            repo.to_str().unwrap(),
            "update-ref",
            "refs/remotes/forged/main",
            "HEAD",
        ]);
        // Sanity: the OLD forgeable predicate would report nothing outgoing.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let hidden = {
            let mut a = ctx.clone();
            a.extend(["rev-list", "HEAD", "--not", "--remotes"].map(String::from));
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let o = capture_raw(&real_git(), &refs).unwrap();
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        assert!(
            hidden.is_empty(),
            "precondition: the forged ref must hide HEAD from `--not --remotes`; got {hidden:?}"
        );
        // The gate must still refuse: exclusions come from the real remote.
        let err = verify_push(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &ctx,
            &managed(),
        )
        .expect_err("a forged remote-tracking ref must not bypass the author gate");
        assert!(err.contains("not authored by your agent identity"), "{err}");
    }

    #[test]
    fn verify_push_fails_closed_when_remote_unreachable() {
        let (_d, repo) = human_authored_repo();
        // origin points at a nonexistent path → dry-run fails → fail closed.
        std::process::Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "/no/such/remote.git",
            ])
            .status()
            .unwrap();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let err = verify_push(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &ctx,
            &managed(),
        )
        .expect_err("unreachable remote must fail closed");
        assert!(err.contains("could not verify outgoing commits"), "{err}");
    }

    #[test]
    fn verify_commit_author_rejects_reuse_of_human_author() {
        // `commit -C <human HEAD>` would stamp the human author on new content.
        let (_d, repo) = human_authored_repo();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let err = verify_commit_author(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "commit", "-C", "HEAD"]),
            &ctx,
            &managed(),
        )
        .expect_err("reusing a human author must be refused");
        assert!(err.contains("not"), "{err}");
    }

    #[test]
    fn verify_commit_author_allows_ordinary_and_agent_amend() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, repo) = human_authored_repo();
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // Ordinary commit (no reuse/amend) is authored fresh as the agent.
        assert!(verify_commit_author(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "commit", "-m", "x"]),
            &ctx,
            &managed(),
        )
        .is_ok());
        // Amending a commit already authored by the agent is the normal fixup
        // flow and must be allowed.
        let agent = managed();
        let agent_repo = tempfile::tempdir().unwrap();
        let ar = agent_repo.path();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(ar)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap();
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.name", "Agent"]);
        g(&["config", "user.email", AGENT_EMAIL]);
        g(&["config", "commit.gpgSign", "false"]);
        std::fs::write(ar.join("f"), "x").unwrap();
        g(&["add", "f"]);
        g(&["commit", "-qm", "agent commit"]);
        let ctx2 = vec!["-C".to_string(), ar.to_string_lossy().into_owned()];
        assert!(verify_commit_author(
            &real_git(),
            &v(&["-C", ar.to_str().unwrap(), "commit", "--amend", "--no-edit"]),
            &ctx2,
            &agent,
        )
        .is_ok());
    }

    // ── helpers for exec-level identity/push tests ─────────────────────────────

    /// Run `git` in `repo` with the given argv and hermetic global/system config.
    fn git_in(repo: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap()
    }

    /// Author email of `rev` in `repo`, trimmed.
    fn author_email(repo: &Path, rev: &str) -> String {
        let out = git_in(repo, &["show", "-s", "--format=%ae", rev]);
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// The destination's real object ids for a repo wired with `origin`, read
    /// the same forge-proof way `verify_push` does (`git ls-remote origin`).
    /// Mirrors [`remote_object_ids`] so `partition_outgoing` tests exercise the
    /// authoritative exclusion set, not local `refs/remotes/*`.
    fn origin_ids(ctx: &[String]) -> Vec<String> {
        remote_object_ids(&real_git(), ctx, "origin").expect("ls-remote origin must succeed")
    }

    // ── C1: injected `-c` identity outranks every other config channel ─────────

    /// The wrapper's re-applied command-line `-c user.email=…` must dominate the
    /// author even when the agent tries to smuggle a human identity in through a
    /// lower-precedence channel: `GIT_CONFIG_PARAMETERS`, a `-c include.path`
    /// include, and repo-file config. Exercised through `inject_identity_args`
    /// (what `exec_real_git` splices) plus a real `git commit`.
    #[test]
    fn injected_identity_outranks_config_parameters_and_include_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git_in(repo, &["init", "-q", "-b", "main"]);
        // Repo-file config claims a human identity (a legitimate lower channel).
        git_in(repo, &["config", "user.name", "Human"]);
        git_in(repo, &["config", "user.email", "human@example.com"]);
        git_in(repo, &["config", "commit.gpgSign", "false"]);

        // An include file that also tries to set a human identity.
        let inc = repo.join("evil.inc");
        std::fs::write(&inc, "[user]\n\temail = include@evil.com\n").unwrap();

        std::fs::write(repo.join("f"), "x").unwrap();
        git_in(repo, &["add", "f"]);

        // Caller argv smuggles identity via a `-c include.path` global. The
        // wrapper splices its authoritative `-c user.email=<agent>` AFTER this,
        // so command-line precedence (last `-c` wins) must make the agent win.
        let caller = v(&[
            "-c",
            &format!("include.path={}", inc.display()),
            "commit",
            "-qm",
            "smuggled",
        ]);
        let full = inject_identity_args(&caller, Some(&managed_nosign()));
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();

        // Also arm the env channel the wrapper re-append is meant to defeat.
        let params = "'user.email=params@evil.com'";
        let out = std::process::Command::new("git")
            .args(&refs)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_PARAMETERS", params)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            author_email(repo, "HEAD"),
            AGENT_EMAIL,
            "injected -c identity must beat include.path, GIT_CONFIG_PARAMETERS, and repo config"
        );
    }

    // ── C2: shell (`!`) aliases are refused outright in a managed session ─────

    /// `verify_alias_safety` must refuse a `!`-shell alias (here one whose body
    /// would push) without executing it. A sentinel file proves the body never
    /// runs during classification — refusal is by source inspection only.
    #[test]
    fn shell_alias_is_refused_without_executing_its_body() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git_in(repo, &["init", "-q", "-b", "main"]);
        let sentinel = repo.join("ran");
        let body = format!("!touch {} && git push", sentinel.display());
        git_in(repo, &["config", "alias.deploy", &body]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let err = verify_alias_safety(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "deploy"]),
            &ctx,
        );
        assert!(err.is_err(), "shell alias must be refused");
        assert!(
            err.unwrap_err().contains("shell (`!`) git alias"),
            "expected the shell-alias rejection message"
        );
        assert!(
            !sentinel.exists(),
            "classification must not execute the shell alias body"
        );
    }

    /// A non-push `!`-shell alias is refused too — the ruling rejects ALL shell
    /// aliases in a managed session, not only push-bearing ones.
    #[test]
    fn shell_alias_without_push_is_also_refused() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        git_in(repo, &["init", "-q", "-b", "main"]);
        git_in(repo, &["config", "alias.st", "!git status"]);
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        assert!(
            verify_alias_safety(&real_git(), &v(&["-C", repo.to_str().unwrap(), "st"]), &ctx)
                .is_err()
        );
    }

    // ── I3: cherry-picked / rebased upstream human commits are exempt ──────────

    /// Build `(dir, local, remote)` where `remote` (a real bare repo wired as
    /// `origin` and fetched) carries a human-authored commit, and `local` is on
    /// a branch forked from the shared base. Returns paths for building the two
    /// rebase/cherry-pick shapes on top.
    fn repo_with_upstream_human_commit() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.git");
        let local = dir.path().join("local");
        // Seed remote via a scratch working clone, then discard it.
        git_in(
            dir.path(),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        let seed = dir.path().join("seed");
        git_in(
            dir.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                seed.to_str().unwrap(),
            ],
        );
        git_in(&seed, &["config", "user.name", "Human"]);
        git_in(&seed, &["config", "user.email", "human@example.com"]);
        git_in(&seed, &["config", "commit.gpgSign", "false"]);
        std::fs::write(seed.join("base"), "b").unwrap();
        git_in(&seed, &["add", "base"]);
        git_in(&seed, &["commit", "-qm", "base"]);
        std::fs::write(seed.join("human"), "h").unwrap();
        git_in(&seed, &["add", "human"]);
        git_in(&seed, &["commit", "-qm", "human work"]);
        git_in(&seed, &["push", "-q", "origin", "HEAD:main"]);

        // Local clone forked from the shared BASE (not the human tip).
        git_in(
            dir.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                local.to_str().unwrap(),
            ],
        );
        git_in(&local, &["config", "user.name", "Agent"]);
        git_in(&local, &["config", "user.email", AGENT_EMAIL]);
        git_in(&local, &["config", "commit.gpgSign", "false"]);
        (dir, local, remote)
    }

    /// I3 shape B (the shape that exercises the fix): the agent rebases/rewrites
    /// the UPSTREAM human commit onto a new base, giving it a fresh SHA. That
    /// new SHA is not reachable from `refs/remotes/*`, so the naive
    /// `rev-list --not --remotes` flags it — but its patch-id matches the
    /// upstream original, so the exemption must let the push through.
    #[test]
    fn verify_push_exempts_rebased_upstream_human_commit_by_patch_id() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, local, _remote) = repo_with_upstream_human_commit();
        // Reset local to the shared base, then cherry-pick the upstream human
        // commit — a replay that rewrites its SHA but preserves its patch and
        // its human author. This is the correct-attribution case the gate must
        // NOT refuse.
        git_in(&local, &["reset", "-q", "--hard", "origin/main~1"]);
        let human_sha =
            String::from_utf8_lossy(&git_in(&local, &["rev-parse", "origin/main"]).stdout)
                .trim()
                .to_string();
        // Add an agent commit first, then replay the human commit on top so the
        // outgoing range is {agent, replayed-human} — both must be allowed.
        std::fs::write(local.join("agent"), "a").unwrap();
        git_in(&local, &["add", "agent"]);
        git_in(&local, &["commit", "-qm", "agent work"]);
        let cp = git_in(&local, &["cherry-pick", &human_sha]);
        assert!(
            cp.status.success(),
            "cherry-pick failed: {}",
            String::from_utf8_lossy(&cp.stderr)
        );

        let ctx = vec!["-C".to_string(), local.to_string_lossy().into_owned()];
        // Authorship only (the signature check is exercised by the real-signer
        // integration tests); `HEAD` is the push source ref.
        let (offenders, _agent) = partition_outgoing(
            &real_git(),
            &ctx,
            AGENT_EMAIL,
            &[String::from("HEAD")],
            &origin_ids(&ctx),
        )
        .expect("partition must succeed");
        assert!(
            offenders.is_empty(),
            "replayed upstream human commit must be exempt by patch-id, got: {offenders:?}"
        );
    }

    /// I3 shape A (agent-onto-human, the ordinary rebase): the agent's own
    /// commit sits on top of the upstream human tip. Only the agent commit is
    /// outgoing; the human commit is already reachable from `refs/remotes/*`.
    /// The push must be allowed, and it exercises the no-exemption-needed path.
    #[test]
    fn verify_push_allows_agent_commit_atop_upstream_human_tip() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, local, _remote) = repo_with_upstream_human_commit();
        std::fs::write(local.join("agent"), "a").unwrap();
        git_in(&local, &["add", "agent"]);
        git_in(&local, &["commit", "-qm", "agent work"]);
        let ctx = vec!["-C".to_string(), local.to_string_lossy().into_owned()];
        let (offenders, _agent) = partition_outgoing(
            &real_git(),
            &ctx,
            AGENT_EMAIL,
            &[String::from("HEAD")],
            &origin_ids(&ctx),
        )
        .expect("partition must succeed");
        assert!(
            offenders.is_empty(),
            "agent commit atop upstream human tip must be allowed: {offenders:?}"
        );
    }

    /// A genuinely NEW human-authored commit (no upstream patch-id match) is
    /// still refused — the patch-id exemption must not become a blanket pass.
    #[test]
    fn verify_push_still_rejects_new_human_commit_without_upstream_match() {
        let (_d, local, _remote) = repo_with_upstream_human_commit();
        // A fresh human-authored commit that exists nowhere upstream.
        std::fs::write(local.join("new"), "n").unwrap();
        git_in(&local, &["add", "new"]);
        git_in(
            &local,
            &[
                "-c",
                "user.name=Human",
                "-c",
                "user.email=human@example.com",
                "commit",
                "-qm",
                "brand-new human work",
            ],
        );
        let ctx = vec!["-C".to_string(), local.to_string_lossy().into_owned()];
        let (offenders, _agent) = partition_outgoing(
            &real_git(),
            &ctx,
            AGENT_EMAIL,
            &[String::from("HEAD")],
            &origin_ids(&ctx),
        )
        .expect("partition must succeed");
        assert!(
            offenders.iter().any(|(_, e)| e == "human@example.com"),
            "a brand-new human commit must be an offender; got {offenders:?}"
        );
    }

    // ── L3b: signing enforcement in the push gate ─────────────────────────────

    /// `Authority::classify` accepts a manifest ONLY when it carries the
    /// complete signing contract and a `user.signingkey` matching the pubkey in
    /// `user.email`. Every weaker/inconsistent state is `Tampered`, so no
    /// surviving `Authority` can silently skip or misdirect the signature gate.
    #[test]
    fn classify_accepts_only_the_complete_and_consistent_signing_contract() {
        let complete = || {
            vec![
                ("user.name".to_string(), "Agent".to_string()),
                ("user.email".to_string(), AGENT_EMAIL.to_string()),
                ("gpg.format".to_string(), "x509".to_string()),
                ("gpg.x509.program".to_string(), "git-sign-nostr".to_string()),
                ("commit.gpgSign".to_string(), "true".to_string()),
                ("tag.gpgSign".to_string(), "true".to_string()),
                ("user.signingkey".to_string(), AGENT_PUBKEY.to_string()),
                ("nostr.keyfile".to_string(), "/tmp/.nostr-key".to_string()),
            ]
        };

        // The canonical install manifest is accepted.
        assert!(matches!(
            Authority::classify(complete()),
            AuthorityState::Managed(_)
        ));

        // No usable identity.
        assert!(matches!(
            Authority::classify(vec![("user.name".into(), "Agent".into())]),
            AuthorityState::Tampered
        ));

        // `commit.gpgSign` absent → the signature gate would never fire.
        let mut no_sign = complete();
        no_sign.retain(|(k, _)| k != "commit.gpgSign");
        assert!(matches!(
            Authority::classify(no_sign),
            AuthorityState::Tampered
        ));

        // `commit.gpgSign=false` → same silent-disable defect, spelled out.
        let mut false_sign = complete();
        false_sign
            .iter_mut()
            .find(|(k, _)| k == "commit.gpgSign")
            .unwrap()
            .1 = "false".into();
        assert!(matches!(
            Authority::classify(false_sign),
            AuthorityState::Tampered
        ));

        // `user.signingkey` naming a DIFFERENT key than the author email encodes
        // → the probe would trust the wrong key.
        let mut wrong_key = complete();
        wrong_key
            .iter_mut()
            .find(|(k, _)| k == "user.signingkey")
            .unwrap()
            .1 = "b".repeat(64);
        assert!(matches!(
            Authority::classify(wrong_key),
            AuthorityState::Tampered
        ));

        // A tampered verifier program cannot pose as managed.
        let mut wrong_program = complete();
        wrong_program
            .iter_mut()
            .find(|(k, _)| k == "gpg.x509.program")
            .unwrap()
            .1 = "/bin/true".into();
        assert!(matches!(
            Authority::classify(wrong_program),
            AuthorityState::Tampered
        ));

        // A verifier program differing ONLY in case — on a case-sensitive host
        // `GIT-SIGN-NOSTR` resolves past the managed install to a later PATH
        // entry, so fixed values must match byte for byte, not case-insensitively.
        let mut cased_program = complete();
        cased_program
            .iter_mut()
            .find(|(k, _)| k == "gpg.x509.program")
            .unwrap()
            .1 = "GIT-SIGN-NOSTR".into();
        assert!(matches!(
            Authority::classify(cased_program),
            AuthorityState::Tampered
        ));

        // A DUPLICATE later `user.signingkey` — git config is last-value-wins,
        // so a first canonical value cannot launder an appended override.
        let mut dup_key = complete();
        dup_key.push(("user.signingkey".into(), "b".repeat(64)));
        assert!(matches!(
            Authority::classify(dup_key),
            AuthorityState::Tampered
        ));

        // Any UNKNOWN key (e.g. an `include.path` pulling in another key file)
        // is rejected — accepted entries are injected verbatim as `-c`.
        let mut extra_key = complete();
        extra_key.push(("include.path".into(), "/tmp/evil.inc".into()));
        assert!(matches!(
            Authority::classify(extra_key),
            AuthorityState::Tampered
        ));

        // A missing canonical key (here `tag.gpgSign`) is incomplete → tampered.
        let mut missing_tag = complete();
        missing_tag.retain(|(k, _)| k != "tag.gpgSign");
        assert!(matches!(
            Authority::classify(missing_tag),
            AuthorityState::Tampered
        ));
    }

    /// When the session enforces signing, an agent-authored but UNSIGNED
    /// outgoing commit is refused. This is the `merge`/`pull`/`commit-tree`
    /// class the flag-based `enforce` cannot catch: the commit is correctly
    /// agent-authored, so only the signature check rejects it. An unsigned
    /// commit yields `%G?` = `N`, so no signer binary is needed to drive this.
    #[test]
    fn verify_push_rejects_unsigned_agent_commit_when_signing_enforced() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, repo) = agent_authored_unsigned_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let err = verify_push(
            &real_git(),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]),
            &ctx,
            &managed(), // signing enforced
        )
        .expect_err("an unsigned agent commit must be refused when signing is enforced");
        assert!(
            err.contains("no valid signature by your agent key"),
            "expected the unsigned-commit rejection; {err}"
        );
    }

    /// A repo with one agent-authored, unsigned commit and no remote.
    fn agent_authored_unsigned_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let g = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.name", "Agent"]);
        g(&["config", "user.email", AGENT_EMAIL]);
        g(&["config", "commit.gpgSign", "false"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        g(&["add", "f"]);
        g(&["commit", "-qm", "agent commit"]);
        (dir, repo)
    }

    // ── I6: the dry-run probe is bounded and a timeout fails closed ────────────

    /// `verify_push_rejects_dotted_scheme_in_url_rewrite_key`: a
    /// `url.foo.https://bar.insteadOf` rewrite key redirects any URL starting
    /// with the insteadOf value through git's remote-helper dispatch, which
    /// would invoke `git-remote-foo.https` (a helper with a dot in its scheme).
    /// The production guard must refuse before any helper is invoked.
    ///
    /// **Positive-control precondition**: this test also proves that with the
    /// guard bypassed (a direct git subprocess with the sentinel on PATH) git
    /// DOES invoke the helper, establishing that the bypass is executable —
    /// not just that the guard fires (which would be a vacuous test if git
    /// never invoked the helper regardless).
    #[test]
    fn verify_push_rejects_dotted_scheme_in_url_rewrite_key() {
        let (_d, repo) = human_authored_repo();
        let sentinel_dir = tempfile::tempdir().unwrap();
        let marker = sentinel_dir.path().join("dotted_scheme_invoked");
        // The helper name git would look for is `git-remote-foo.https`.
        let sentinel = sentinel_dir.path().join("git-remote-foo.https");
        std::fs::write(
            &sentinel,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };

        // Write a `url.foo.https://bar.insteadOf` rewrite key.  The subsection
        // is `foo.https://bar`; git treats `foo.https` as the scheme and would
        // invoke `git-remote-foo.https` for any endpoint that starts with `bar`.
        // Point origin at `http://safe/` — the insteadOf value — so the rewrite
        // FIRES when git resolves it.  An origin pointing at a local path would
        // never match the rewrite, making the bypass precondition vacuous.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            "http://safe/",
        ]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "url.foo.https://bar.insteadOf",
            "http://safe/",
        ]);

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let mut sentinel_path = std::ffi::OsString::from(sentinel_dir.path());
        sentinel_path.push(":");
        sentinel_path.push(&original_path);

        // Positive-control precondition: prove that with the guard bypassed
        // (direct git subprocess, sentinel on PATH) git DOES invoke the helper.
        // `git ls-remote origin` with the rewrite active and `foo.https` on PATH
        // invokes `git-remote-foo.https`, which touches the marker and exits 1.
        // We use a subprocess so PATH manipulation is process-local and safe.
        let probe_status = std::process::Command::new("git")
            .args(["-C", repo.to_str().unwrap(), "ls-remote", "origin"])
            .env("PATH", &sentinel_path)
            .status()
            .unwrap();
        assert!(
            marker.exists() || !probe_status.success(),
            "positive-control precondition: direct git ls-remote must invoke the \
             foo.https sentinel (marker={}, status={:?})",
            marker.exists(),
            probe_status.code()
        );
        // Reset marker for the production-guard assertion below.
        let _ = std::fs::remove_file(&marker);

        // Production guard: verify_push must refuse the key without PATH mutation.
        // The guard fires in inspect_push_config before any git subprocess runs,
        // so PATH does not need to contain the sentinel here.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let result = verify_push(&real_git(), &argv, &argv, &ctx, &managed());

        let err = result.expect_err("dotted-scheme url rewrite key must be refused");
        assert!(
            err.contains("foo.https")
                || err.contains("unknown URL scheme")
                || err.contains("subsection"),
            "expected dotted-scheme refusal; got: {err}"
        );
        assert!(
            !marker.exists(),
            "git-remote-foo.https sentinel was invoked — the dotted-scheme guard did not fire"
        );
    }

    // ── I6: the dry-run probe is bounded and a timeout fails closed ────────────

    /// `capture_raw_bounded` must kill and report failure (`None`) when the
    /// child outlives the timeout, so a hung remote probe cannot block the
    /// wrapper indefinitely. Uses a tiny timeout against a sleep to prove the
    /// bound fires without depending on real network latency.
    #[test]
    fn capture_raw_bounded_times_out_and_fails_closed() {
        // `sleep` via any binary on PATH would do; use the shell so the timeout
        // is deterministic regardless of installed git. We invoke `sh -c sleep`
        // as the "real git" stand-in — capture_raw_bounded only cares that the
        // child runs longer than the timeout.
        let start = std::time::Instant::now();
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", "sleep 5"],
            std::time::Duration::from_millis(200),
        );
        assert!(
            out.is_none(),
            "a child exceeding the timeout must yield None"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "the bound must fire well before the child would finish"
        );
    }

    /// A child that completes within the timeout returns its output normally.
    #[test]
    fn capture_raw_bounded_returns_output_within_timeout() {
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", "printf ok"],
            std::time::Duration::from_secs(5),
        )
        .expect("fast child must produce output");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "ok");
    }

    /// A child that emits large output (well under the cap) returns it in full.
    /// This proves the concurrent drain delivers all bytes, not just a partial
    /// chunk — the pipe-buffer deadlock would truncate or hang instead.
    #[cfg(unix)]
    #[test]
    fn capture_raw_bounded_returns_large_output() {
        // 256 KiB — well above one pipe buffer (~16 KB macOS, ~64 KB Linux)
        // and well under BOUNDED_CAPTURE_LIMIT.
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", "dd if=/dev/zero bs=1024 count=256 2>/dev/null"],
            std::time::Duration::from_secs(10),
        )
        .expect("large-output child must produce output");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 256 * 1024, "all 256 KiB must be returned");
    }

    /// A child that emits output beyond BOUNDED_CAPTURE_LIMIT fails closed.
    /// Proves the overflow path fires before the timeout deadline.
    #[cfg(unix)]
    #[test]
    fn capture_raw_bounded_fails_closed_on_overflow() {
        // Use a very long timeout so a return well before it proves the cap — not
        // the timeout — ended the call.
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", "exec cat /dev/zero"],
            std::time::Duration::from_secs(60),
        );
        assert!(
            out.is_none(),
            "an unbounded producer must fail closed on the capture cap before the deadline"
        );
    }

    /// A child that exits cleanly but backgrounds a grandchild holding the pipe
    /// must not block the drain joins.  Proves kill-on-success + stop-flag
    /// work together.
    #[cfg(unix)]
    #[test]
    fn capture_raw_bounded_returns_when_descendant_holds_pipe() {
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let pid_path = pid_file.path().to_str().unwrap().to_string();
        // Background a `sleep` that retains stdout; record its PID; leader exits.
        let script = format!(
            "sleep 30 & echo $! > '{pid_path}'; \
             until [ -s '{pid_path}' ]; do :; done; echo done; exit 0"
        );
        let start = std::time::Instant::now();
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", &script],
            std::time::Duration::from_secs(10),
        )
        .expect("leader exits, so output must be returned");
        assert!(out.status.success());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(8),
            "must return well before the timeout; elapsed={:?}",
            start.elapsed()
        );
    }

    /// A child that backgrounds a grandchild calling `setsid()` (escaping the
    /// process group) that then holds the pipe must still return promptly.
    /// This is the exact group-escape case; the nonblocking drain stops on
    /// `WouldBlock` after teardown rather than blocking on the escaped writer.
    #[cfg(unix)]
    #[test]
    fn capture_raw_bounded_returns_when_group_escaped_descendant_holds_pipe() {
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let pid_path = pid_file.path().to_str().unwrap().to_string();
        // Perl descendant calls setsid(), records PID, writes a bit, then sleeps.
        let script = format!(
            "perl -MPOSIX -e 'POSIX::setsid(); open(my $f,\">\",$ARGV[0]) or die; \
             print $f $$; close $f; print \"x\" x 1024; sleep 30;' '{pid_path}' & \
             until [ -s '{pid_path}' ]; do :; done; while :; do sleep 1; done"
        );
        let start = std::time::Instant::now();
        let out = capture_raw_bounded(
            Path::new("sh"),
            &["-c", &script],
            std::time::Duration::from_millis(300),
        );
        assert!(
            out.is_none(),
            "a timed-out probe with a group-escaped writer must fail closed"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "must return after the timeout, not hang; elapsed={:?}",
            start.elapsed()
        );
        // Clean up the escaped descendant.
        if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }

    /// (Thufir P1) A push URL supplied inline in argv containing a literal `\n`
    /// is caught by `inspect_push_config` (Surface 1) before porcelain is
    /// parsed.  This test exercises the **argv** surface:
    /// `git push <newline-bearing-path> main` with the path given inline.
    ///
    /// The guard fires on the CR/LF in argv; `verify_push` returns an error
    /// before any porcelain output is produced.  The destination must remain
    /// empty.
    ///
    /// Mutation evidence: removing the newline guard from `inspect_push_config`
    /// makes `verify_push` proceed past the newline check; execution continues
    /// to a later refusal (destination-inventory or author check) with a
    /// different error class — so the assertion that the error contains "CR or
    /// LF" catches the mutation.
    #[test]
    fn verify_push_rejects_newline_in_argv_url() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, repo) = human_authored_repo();
        let parent = tempfile::tempdir().unwrap();

        // actual: the real target — path with embedded newline.  Starts empty.
        let actual_path = parent.path().join("target\nsuffix");
        std::fs::create_dir_all(&actual_path).expect("create newline-path dir");
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", actual_path.to_str().unwrap()]);

        // With the guard, verify_push refuses before reaching porcelain.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            actual_path.to_str().unwrap(),
            "main",
        ]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("newline in inline push URL must be refused");
        assert!(
            err.contains("CR or LF") || err.contains("newline") || err.contains("LF"),
            "expected newline-argv refusal; got: {err}"
        );
        // actual must remain empty — the guard fired before any write.
        let ls_actual = std::process::Command::new("git")
            .args(["ls-remote", actual_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls_actual.stdout).trim().is_empty(),
            "actual repo must remain empty after guard refusal"
        );
    }

    /// (Thufir P1) A `remote.origin.url` config value containing a literal `\n`
    /// is caught by `inspect_push_config` (Surface 2) via the single
    /// `--null --get-regexp` config snapshot.  Plain `--get-regexp` would split
    /// the value into two lines; the NUL probe reads the complete raw value bytes.
    ///
    /// This test exercises the **config** surface: the newline is in
    /// `remote.origin.url` written by `git remote add`, not in inline argv.
    ///
    /// Mutation evidence: `actual` is an empty bare repo.  With the guard,
    /// `verify_push` refuses on the config snapshot before reaching porcelain.
    /// With the guard removed, execution proceeds past the newline-specific
    /// refusal to a later error class — the assertion that the error contains
    /// "CR or LF" still catches the mutation.
    #[test]
    fn verify_push_rejects_newline_in_config_url() {
        #[cfg(unix)]
        let _guard = clear_git_config_env();
        let (_d, repo) = human_authored_repo();
        let parent = tempfile::tempdir().unwrap();

        // actual: the real target — path with embedded newline.  Starts empty.
        let actual_path = parent.path().join("target\nsuffix");
        std::fs::create_dir_all(&actual_path).expect("create newline-path dir");
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", actual_path.to_str().unwrap()]);

        // Wire origin → actual via `remote add` (writes newline into config).
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            actual_path.to_str().unwrap(),
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("newline in config URL must be refused");
        assert!(
            err.contains("CR or LF") || err.contains("newline") || err.contains("LF"),
            "expected newline-endpoint config refusal; got: {err}"
        );
        // actual must remain empty — the guard fired before any write.
        let ls_actual = std::process::Command::new("git")
            .args(["ls-remote", actual_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls_actual.stdout).trim().is_empty(),
            "actual repo must remain empty after guard refusal"
        );
    }

    /// (Thufir P1) `GIT_SSH_COMMAND` set in the environment is caught by
    /// `inspect_push_config` (Surface 0) before any porcelain is run.
    #[test]
    fn verify_push_rejects_git_ssh_command_env() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        // The guard reads std::env at call time. The crate-wide harness holds
        // the shared lock through child execution and restores the exact prior
        // `OsString` even if the test panics.
        let mut env = TestEnv::lock();
        env.set("GIT_SSH_COMMAND", "/evil/ssh");
        let result = verify_push(&real_git(), &argv, &argv, &ctx, &managed());

        let err = result.expect_err("GIT_SSH_COMMAND set must be refused");
        assert!(
            err.contains("GIT_SSH_COMMAND") || err.contains("transport"),
            "expected transport-override refusal; got: {err}"
        );
    }

    /// (Thufir P1) `core.sshCommand` in effective config is caught by
    /// `inspect_push_config` (Surface 2) via the single `--null --get-regexp`
    /// config snapshot.
    #[test]
    fn verify_push_rejects_core_ssh_command_config() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // Inject core.sshCommand into the repo config.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "core.sshCommand",
            "/evil/ssh",
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("core.sshCommand in config must be refused");
        assert!(
            err.contains("core.sshCommand") || err.contains("transport"),
            "expected transport-override config refusal; got: {err}"
        );
    }

    /// Snapshot/parser test: the production `git config --null --get-regexp`
    /// query must see one key from each of the four namespaces, using the
    /// case-insensitive character-class pattern.  This test proves:
    ///
    /// 1. The ERE character-class pattern is accepted by the local git binary
    ///    (no exit 1).
    /// 2. Keys from every protected namespace are returned.
    /// 3. Case-varied section headers (`[Remote]`, `[Core]`) produce output
    ///    that the pattern matches (git lowercases before matching at ≥ 2.28;
    ///    the character classes defend against older versions).
    /// 4. A mutation reverting the pattern to BRE (`^\(remote\|...\)`) would
    ///    cause exit 1 + empty output, failing the assertions here.
    ///
    /// The exact pattern used is the one `inspect_push_config` builds:
    ///   `r"^([Rr][Ee][Mm][Oo][Tt][Ee]|[Uu][Rr][Ll]|[Bb][Rr][Aa][Nn][Cc][Hh]|[Cc][Oo][Rr][Ee])\."`
    #[test]
    fn inspect_push_config_query_sees_all_namespaces() {
        let (_d, repo) = human_authored_repo();
        let repo_str = repo.to_str().unwrap();
        let g = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };

        // Write one key from each of the four protected namespaces.
        // remote.*: a receivepack override (refused by policy, but present for query)
        g(&["-C", repo_str, "remote", "add", "origin", "/tmp/dummy"]);
        g(&[
            "-C",
            repo_str,
            "config",
            "remote.origin.receivepack",
            "evil-rp",
        ]);
        // branch.*: a pushremote setting
        g(&["-C", repo_str, "config", "branch.main.pushremote", "origin"]);
        // url.*: a pushinsteadof rewrite (key name uses url. prefix)
        g(&[
            "-C",
            repo_str,
            "config",
            "url.https://safe/.pushinsteadOf",
            "http://old/",
        ]);
        // core.*: a sshcommand override
        g(&["-C", repo_str, "config", "core.sshCommand", "/evil/ssh"]);

        const PROD_PATTERN: &str =
            r"^([Rr][Ee][Mm][Oo][Tt][Ee]|[Uu][Rr][Ll]|[Bb][Rr][Aa][Nn][Cc][Hh]|[Cc][Oo][Rr][Ee])\.";

        // Run the exact query `inspect_push_config` uses — same args, same pattern.
        let out = std::process::Command::new("git")
            .args([
                "-C",
                repo_str,
                "config",
                "--null",
                "--get-regexp",
                PROD_PATTERN,
            ])
            .output()
            .unwrap();

        assert!(
            out.status.success() || out.status.code() == Some(0),
            "git config --get-regexp must succeed (exit 0) with ERE character-class pattern; \
             exit={:?}; stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );

        let stdout = String::from_utf8_lossy(&out.stdout);
        // NUL-delimited records: `<key>\n<value>\0`.  Split on NUL to get records.
        let keys: Vec<&str> = stdout
            .split('\0')
            .filter_map(|rec| rec.split('\n').next())
            .filter(|k| !k.is_empty())
            .collect();

        assert!(
            keys.iter().any(|k| k.starts_with("remote.")),
            "remote.* namespace must appear in query output; keys={keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.starts_with("branch.")),
            "branch.* namespace must appear in query output; keys={keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.starts_with("url.")),
            "url.* namespace must appear in query output; keys={keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.starts_with("core.")),
            "core.* namespace must appear in query output; keys={keys:?}"
        );

        // Mutation proof: the old BRE pattern produces exit 1 + empty output.
        let bre_out = std::process::Command::new("git")
            .args([
                "-C",
                repo_str,
                "config",
                "--null",
                "--get-regexp",
                r"^\(remote\|url\|branch\|core\)\.",
            ])
            .output()
            .unwrap();
        let bre_stdout = String::from_utf8_lossy(&bre_out.stdout);
        assert!(
            bre_out.status.code() == Some(1) && bre_stdout.trim().is_empty(),
            "BRE pattern must exit 1 + empty stdout on this git binary — \
             if it matches, the BRE mutation would be undetectable; \
             exit={:?}, stdout={bre_stdout:?}",
            bre_out.status.code()
        );

        // Case-varied regression: write keys with capital section/variable names
        // and assert the production pattern still sees them.  Git 2.54 lowercases
        // before matching; older versions may not — the character classes defend
        // against that.
        //
        // Write keys using raw config file injection so git does not normalize
        // the casing (git-config write always lowercases; file write preserves it).
        let config_path = repo.join(".git/config");
        let mut config_content = std::fs::read_to_string(&config_path).unwrap();
        config_content.push_str(
            "\n[Remote \"caps\"]\n    receivePack = evil-caps\n\
             [Core]\n    sshCommand = /evil/ssh2\n",
        );
        std::fs::write(&config_path, &config_content).unwrap();

        // The production pattern must still return these case-varied keys.
        let caps_out = std::process::Command::new("git")
            .args([
                "-C",
                repo_str,
                "config",
                "--null",
                "--get-regexp",
                PROD_PATTERN,
            ])
            .output()
            .unwrap();
        let caps_stdout = String::from_utf8_lossy(&caps_out.stdout);
        let caps_keys: Vec<&str> = caps_stdout
            .split('\0')
            .filter_map(|rec| rec.split('\n').next())
            .filter(|k| !k.is_empty())
            .collect();
        // The key appears lowercased in output (git normalizes before emitting).
        assert!(
            caps_keys
                .iter()
                .any(|k| k.starts_with("remote.") && k.ends_with(".receivepack")),
            "case-varied [Remote] section must produce a remote.*.receivepack key; \
             caps_keys={caps_keys:?}"
        );
        assert!(
            caps_keys.contains(&"core.sshcommand"),
            "case-varied [Core] section must produce core.sshcommand key; \
             caps_keys={caps_keys:?}"
        );
    }

    /// (Thufir P1) An `evil://` endpoint in push argv invokes a PATH-resident
    /// `git-remote-evil` helper.  `inspect_push_config` (Surface 1) must refuse
    /// the push before any helper is executed.
    ///
    /// Sentinel setup: a script named `git-remote-evil` is written to a temp
    /// directory and placed first on PATH.  If the guard fires, the script is
    /// never executed.  If the guard were removed, `git ls-remote evil://x`
    /// would exec the sentinel — the test confirms the sentinel was NOT invoked
    /// by checking that verify_push returns Err with an "unknown URL scheme"
    /// message.
    #[test]
    fn verify_push_rejects_evil_scheme_in_argv() {
        let (_d, repo) = human_authored_repo();
        let sentinel_dir = tempfile::tempdir().unwrap();
        // Write a sentinel that creates a marker file if invoked.  The sentinel
        // directory is NOT added to PATH; if the guard fires (expected), git is
        // never spawned with evil://, so the helper is never looked up.  If the
        // guard were removed, git would search the current PATH for
        // `git-remote-evil`; it would not find it (sentinel_dir is not on PATH),
        // producing a "helper not found" error rather than an "unknown URL scheme"
        // error — the message assertion catches the mutation.  The marker check
        // adds defence-in-depth: if someone adds sentinel_dir to PATH in future,
        // an invocation is still detected.
        let marker = sentinel_dir.path().join("evil_invoked");
        let sentinel = sentinel_dir.path().join("git-remote-evil");
        std::fs::write(
            &sentinel,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // evil:// directly in argv — the unknown scheme the guard must catch.
        let argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "evil://payload",
            "main",
        ]);

        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("evil:// in argv must be refused");
        assert!(
            err.contains("unknown URL scheme") || err.contains("evil"),
            "expected unknown-scheme refusal; got: {err}"
        );
        // The sentinel must NOT have been executed — the guard fired before git ran.
        assert!(
            !marker.exists(),
            "git-remote-evil sentinel was invoked — the evil:// guard did not fire \
             before the helper was executed"
        );
    }

    /// (Thufir P1) An `evil://` URL in `remote.origin.url` config is caught by
    /// `inspect_push_config` (Surface 2) before any push probe runs.
    #[test]
    fn verify_push_rejects_evil_scheme_in_config_url() {
        let (_d, repo) = human_authored_repo();
        let sentinel_dir = tempfile::tempdir().unwrap();
        let marker = sentinel_dir.path().join("evil_invoked");
        let sentinel = sentinel_dir.path().join("git-remote-evil");
        std::fs::write(
            &sentinel,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        // Wire origin to evil:// — the config surface.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            "evil://payload",
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed())
            .expect_err("evil:// in config URL must be refused");
        assert!(
            err.contains("unknown URL scheme") || err.contains("evil"),
            "expected unknown-scheme config refusal; got: {err}"
        );
        assert!(
            !marker.exists(),
            "git-remote-evil sentinel was invoked — the config evil:// guard did not fire"
        );
    }

    // ── Carl R8 P1: --no-dry-run negation ─────────────────────────────────────
    //
    // `resolve_push_sources` injects `--dry-run --porcelain --no-verify` after
    // the push subcommand.  If `--no-dry-run` is present in the caller's argv,
    // git's option parser (which evaluates flags left-to-right and lets the last
    // occurrence win for negatable bit options) clears the dry-run bit — turning
    // the supposedly read-only probe into a real push before authorship/signature
    // checks run.
    //
    // The guard in `reject_receive_pack_override` must fire BEFORE any probe runs.

    /// (Carl R8 P1) `--no-dry-run` in push argv is refused before the probe runs.
    ///
    /// Mutation evidence: removing the `--no-dry-run` check from
    /// `reject_receive_pack_override` makes this test pass (`Ok`) instead of
    /// `Err` — confirming the guard, not the subsequent dry-run logic, is what
    /// catches the flag.
    #[test]
    fn verify_push_rejects_no_dry_run_flag() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];

        // Full spelling.
        let argv_full = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--no-dry-run",
            "origin",
            "main",
        ]);
        let err = verify_push(&real_git(), &argv_full, &argv_full, &ctx, &managed())
            .expect_err("--no-dry-run must be refused in managed mode");
        assert!(
            err.contains("--no-dry-run") || err.contains("dry-run"),
            "expected dry-run-negation refusal; got: {err}"
        );

        // Unique prefix abbreviation `--no-dry` — also must be refused.
        let argv_abbr = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--no-dry",
            "origin",
            "main",
        ]);
        let err2 = verify_push(&real_git(), &argv_abbr, &argv_abbr, &ctx, &managed())
            .expect_err("abbreviated --no-dry must be refused in managed mode");
        assert!(
            err2.contains("--no-dry") || err2.contains("dry-run"),
            "expected dry-run-negation refusal (abbreviated form); got: {err2}"
        );

        // Minimal unique prefix `--no-dr` — git accepts this as the shortest
        // unambiguous abbreviation of `--no-dry-run` (cf. `--no-d` is ambiguous
        // between `--no-delete` and `--no-dry-run`; `--no-dr` resolves uniquely).
        // Before the guard was extended to `starts_with("--no-dr")`, this form
        // slipped through and turned the dry-run probe into a real push.
        let argv_min = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "--no-dr",
            "origin",
            "main",
        ]);
        let err3 = verify_push(&real_git(), &argv_min, &argv_min, &ctx, &managed())
            .expect_err("--no-dr must be refused — minimal unique prefix of --no-dry-run");
        assert!(
            err3.contains("--no-dr") || err3.contains("dry-run"),
            "expected dry-run-negation refusal for --no-dr; got: {err3}"
        );
        // Mutation precondition at the minimal prefix: reject_receive_pack_override
        // must return Err for --no-dr too — changing starts_with("--no-dr") to
        // starts_with("--no-dry") would make this Ok and reopen the bypass.
        let rp_min = reject_receive_pack_override(&argv_min);
        assert!(
            rp_min.is_err(),
            "reject_receive_pack_override must reject --no-dr; \
             reverting to starts_with(\"--no-dry\") would make this Ok"
        );

        // Mutation precondition: removing the guard must flip this to Ok.
        // Verified by directly calling reject_receive_pack_override, which is
        // the function that contains the new check:
        //   WITHOUT the guard: reject_receive_pack_override(&argv_full) == Ok(())
        //   WITH the guard (current state): it returns Err containing "no-dry-run".
        let rp_result = reject_receive_pack_override(&argv_full);
        assert!(
            rp_result.is_err(),
            "mutation target: reject_receive_pack_override must return Err for --no-dry-run; \
             removing the check here would make this Ok and let --no-dry-run slip through"
        );
        let rp_err = rp_result.unwrap_err();
        assert!(
            rp_err.contains("--no-dry-run") || rp_err.contains("dry-run"),
            "reject_receive_pack_override must name the flag; got: {rp_err}"
        );

        // The destination must be empty — the guard fires before any probe reaches
        // the remote.
        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty — the --no-dry-run guard must fire before any push \
             probe reaches the remote; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    // ── Carl R8 P1: alias dispatch uses effective_argv ────────────────────────
    //
    // Before this fix, `is_push_command` was called with the original `argv`
    // (not `effective_argv`).  For `alias.pub = -p push`:
    //   - The body `-p push` is safe (no config channel, no quote).
    //   - `verify_alias_safety` expands it and returns `effective_argv` whose
    //     subcommand is `push`.
    //   - But `is_push_command(&real_git, &argv, &ctx)` takes the original
    //     typed name `pub`, looks up `alias.pub = -p push`, gets `-p` as the
    //     next command word (first token of the body), looks up `alias.-p`
    //     (absent), and returns `NotPush` — bypassing all push verification.
    //
    // The same bypass applies to the exact ten-hop boundary: a chain
    // a1→a2→…→a10 is the maximum alias expansion.  `verify_alias_safety`
    // validates the entire chain and returns a correct `effective_argv` whose
    // subcommand is `push`.  `is_push_command` with the original argv re-walks
    // the chain from scratch, fails at the tenth alias (returns `NotPush` after
    // 10 iterations), and skips push verification.
    //
    // The fix: pass `effective_argv` to `is_push_command`.

    /// (Carl R8 P1) `-p push` alias body bypassed push verification before fix.
    ///
    /// Mutation evidence: reverting `is_push_command(&real_git, effective_argv, …)`
    /// back to `is_push_command(&real_git, &argv, …)` makes this test pass (Ok)
    /// instead of Err — confirming the fix is the use of effective_argv.
    #[test]
    fn verify_push_rejects_commit_via_dash_p_push_alias() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // Set `alias.pub = -p push`; `-p` is a safe bare-word token (no `-c`,
        // no `=`, no quote), so `verify_alias_safety` expands it to
        // `["-C", repo, "push", "origin", "main"]`.  The human-authored HEAD
        // is the outgoing commit; the remote is empty.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "alias.pub",
            "-p push",
        ]);

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        // The alias-expanded effective_argv has "push" as the subcommand.
        // With the old argv-based is_push_command, "pub" → alias body "-p push"
        // → first word "-p" → no alias → NotPush — verification skipped.
        let argv = v(&["-C", repo.to_str().unwrap(), "pub", "origin", "main"]);
        let effective = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        // Mutation precondition: is_push_command with the ORIGINAL argv returns NotPush.
        assert!(
            matches!(is_push_command(&real_git(), &argv, &ctx), PushKind::NotPush),
            "mutation evidence: is_push_command(original argv) must return NotPush for \
             `alias.pub = -p push` — reverting the fix recreates the bypass"
        );
        // The fix: is_push_command with effective_argv returns Push.
        assert!(
            matches!(
                is_push_command(&real_git(), &effective, &ctx),
                PushKind::Push
            ),
            "is_push_command(effective_argv) must return Push after alias expansion"
        );

        // End-to-end: verify_push with effective_argv must reject the human commit.
        let err = verify_push(&real_git(), &argv, &effective, &ctx, &managed()).expect_err(
            "human-authored HEAD via -p push alias must be refused by the push gate; \
                 with the old argv-based is_push_command the alias was a NotPush — bypassed",
        );
        assert!(err.contains("not authored by your agent identity"), "{err}");

        // Destination must be empty — the human commit must not have reached it.
        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty after rejection; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    /// (Carl R8 P1) Exact ten-hop alias chain ending in `push` is still
    /// classified as a push via `effective_argv` and the human commit is refused.
    ///
    /// A chain of exactly 10 aliases is the maximum `verify_alias_safety` will
    /// expand.  With the old `is_push_command(&argv, …)` the function re-walks
    /// the chain from scratch, exits the loop after 10 iterations (the limit)
    /// without finding `push` as the resolved command, and returns `NotPush`.
    /// With `effective_argv`, classification of the already-expanded command is
    /// immediate.
    ///
    /// Mutation evidence: reverting to `argv`-based classification makes this
    /// test pass (Ok) instead of Err.
    #[test]
    fn verify_push_rejects_commit_via_ten_hop_push_alias_chain() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // Build: alias.a1 = a2, alias.a2 = a3, …, alias.a9 = a10, alias.a10 = push.
        // Ten hops total; `verify_alias_safety` expands this successfully because
        // every body token is a safe bare word.
        for (from, to) in [
            ("alias.a1", "a2"),
            ("alias.a2", "a3"),
            ("alias.a3", "a4"),
            ("alias.a4", "a5"),
            ("alias.a5", "a6"),
            ("alias.a6", "a7"),
            ("alias.a7", "a8"),
            ("alias.a8", "a9"),
            ("alias.a9", "a10"),
            ("alias.a10", "push"),
        ] {
            g(&["-C", repo.to_str().unwrap(), "config", from, to]);
        }

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&["-C", repo.to_str().unwrap(), "a1", "origin", "main"]);
        let effective = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);

        // Mutation precondition: is_push_command with original argv returns NotPush
        // after exhausting the 10-hop limit.
        assert!(
            matches!(is_push_command(&real_git(), &argv, &ctx), PushKind::NotPush),
            "mutation evidence: is_push_command(original argv) must return NotPush after \
             10 hops — reverting the fix recreates the bypass for exactly-ten-hop chains"
        );
        // The fix: effective_argv has push as the subcommand.
        assert!(
            matches!(
                is_push_command(&real_git(), &effective, &ctx),
                PushKind::Push
            ),
            "is_push_command(effective_argv) must return Push"
        );

        // End-to-end: human commit via 10-hop alias must be refused.
        let err = verify_push(&real_git(), &argv, &effective, &ctx, &managed()).expect_err(
            "human commit via 10-hop push alias must be refused; \
                 with argv-based classification the 10-hop limit makes it NotPush — bypassed",
        );
        assert!(err.contains("not authored by your agent identity"), "{err}");

        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    // ── Carl R9 P1-b / P5 P1: builtin-shadowing alias suppresses verification ──
    //
    // P1-b (R9): `verify_alias_safety` previously looked up `alias.<name>` for
    // EVERY command word, including builtin subcommands like `push`.  Git's own
    // dispatch (git.c `handle_builtin`) checks builtins BEFORE alias config on
    // non-deprecated commands, so an `alias.push=status` setting is silently
    // ignored at exec time.  The wrapper's expansion loop, however, DID follow
    // it: with `-c alias.push=status`, it produced effective_argv with subcommand
    // `status` → `is_push_command` returned `NotPush` → verification skipped →
    // `exec_real_git` ran the original `push` argv → git ignored the alias →
    // real push happened.
    //
    // P1-b fix (R9): query `git --list-cmds=builtins` once at the top of the
    // expansion loop and break immediately when the current name is in the set,
    // treating it as a real subcommand (no alias lookup).
    //
    // P5 P1 (Thufir pass 5, this round): on binaries where `--list-cmds=deprecated`
    // succeeds, deprecated builtins (`whatchanged`, `pack-redundant`) are
    // alias-first: `run_argv()` calls `handle_alias()` BEFORE `handle_builtin()`
    // for those.  The unconditional builtin short-circuit from R9 therefore
    // created another bypass: `verify_alias_safety` would break at `whatchanged`
    // (it is in builtins), classify as NotPush, skip verification — while real
    // git on such binaries expands the alias and pushes.
    //
    // P5 fix: query `git --list-cmds=deprecated` for the same binary.  A name
    // that is in BOTH sets is alias-first on this binary; skip the builtin
    // short-circuit and continue alias lookup.  When `--list-cmds=deprecated`
    // exits non-zero, the deprecated set is empty and ALL builtins are treated
    // as builtin-first — matching that binary's actual dispatch.
    //
    // Regression tests use `run_inner` (the extracted inner body of `run()`)
    // rather than `verify_push` directly, because tests that hand-construct
    // `effective_argv` cannot catch a bug in `verify_alias_safety` itself.
    // `run_inner` exercises the full verify_alias_safety → is_push_command →
    // verify_push pipeline end-to-end.

    /// (Carl R9 P1-b) `git push` with `alias.push=status` is still refused.
    ///
    /// Mutation evidence: removing the builtin-precedence check from
    /// `verify_alias_safety` (reverting to the unconditional alias lookup) makes
    /// this test pass (exit 0) instead of failing — the expansion would replace
    /// `push` with `status`, classify as `NotPush`, and skip verification.
    #[test]
    fn verify_push_builtin_alias_shadow_does_not_bypass_verification() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // Set alias.push=status — a builtin-shadowing alias git silently ignores.
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "alias.push",
            "status",
        ]);

        // `run_inner` exercises the full verify_alias_safety → is_push_command →
        // verify_push path with a managed authority.  The human-authored HEAD
        // must be refused; destination must remain empty.
        let argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        let exit = run_inner(argv, Some(managed()));
        assert_ne!(
            exit, 0,
            "run_inner must refuse human push even when alias.push=status is set"
        );

        // Mutation precondition: without the builtin check, verify_alias_safety
        // would expand push→status (NotPush); with it, push is treated as a
        // real subcommand and effective_argv retains push as the command word.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let check_argv = v(&["-C", repo.to_str().unwrap(), "push", "origin", "main"]);
        // verify_alias_safety must return Ok(None) for the builtin `push` name
        // even when alias.push is set — no expansion should occur.
        let expansion = verify_alias_safety(&real_git(), &check_argv, &ctx)
            .expect("verify_alias_safety must not return Err for alias.push=status");
        assert!(
            expansion.is_none(),
            "verify_alias_safety must return Ok(None) for builtin `push` (no expansion); \
             got Some(expanded) — removing the builtin check recreates the bypass"
        );

        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    /// (Carl R9 P1-b) `git pub` with `alias.pub=push` and `alias.push=status`
    /// is still refused — the alias chain terminates at the builtin `push` and
    /// does not follow `alias.push=status` to `status`.
    ///
    /// Mutation evidence: removing the builtin-precedence check makes
    /// `verify_alias_safety` continue beyond `push` to `status`, classify as
    /// `NotPush`, and skip verification, allowing a real push.
    #[test]
    fn verify_push_alias_chain_terminating_at_builtin_is_refused() {
        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);
        // alias.pub=push: user command → builtin push.
        // alias.push=status: would further redirect if builtin precedence were ignored.
        g(&["-C", repo.to_str().unwrap(), "config", "alias.pub", "push"]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "alias.push",
            "status",
        ]);

        let argv = v(&["-C", repo.to_str().unwrap(), "pub", "origin", "main"]);
        let exit = run_inner(argv, Some(managed()));
        assert_ne!(
            exit, 0,
            "run_inner must refuse human push via pub→push even with alias.push=status set"
        );

        // Mutation precondition: verify_alias_safety must expand pub→push and
        // stop there (push is builtin) — returning Some(effective_argv) with
        // push as the subcommand, not None and not Some([..., "status", ...]).
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let check_argv = v(&["-C", repo.to_str().unwrap(), "pub", "origin", "main"]);
        let expansion = verify_alias_safety(&real_git(), &check_argv, &ctx)
            .expect("verify_alias_safety must not Err for pub→push chain");
        let expanded = expansion
            .expect("verify_alias_safety must expand non-builtin `pub` alias to Some(argv)");
        let sub = subcommand(&expanded).expect("expanded argv must have a subcommand");
        assert_eq!(
            sub, "push",
            "expansion must terminate at builtin `push`, not follow alias.push=status; \
             removing the builtin check would expand further to `status`"
        );

        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    /// (Pass 5 P1) A deprecated builtin with `alias.<name> = push …` on a binary
    /// where `--list-cmds=deprecated` succeeds is still refused — the wrapper must
    /// NOT short-circuit at the deprecated builtin name and must expand the alias,
    /// classify the result as Push, and enforce policy.
    ///
    /// Background: on binaries where `--list-cmds=deprecated` succeeds, deprecated
    /// builtins (`whatchanged`, `pack-redundant`) are alias-first: `run_argv()`
    /// calls `handle_alias()` before `handle_builtin()` for those names.  The R9
    /// fix short-circuited at every builtin, treating ALL of them as builtin-first;
    /// that left `alias.whatchanged = push origin main` reachable as a bypass.
    ///
    /// Fix: when a name is in BOTH `--list-cmds=builtins` AND
    /// `--list-cmds=deprecated`, it is alias-first on this binary.  Skip the
    /// short-circuit and follow the alias — which then hits `push`, is classified
    /// as Push, and is refused.
    ///
    /// Dispatch gate: skips cleanly on binaries where `--list-cmds=deprecated`
    /// exits non-zero or does not list `whatchanged` — those binaries do not have
    /// the alias-first deprecated dispatch exception.
    ///
    /// Mutation preconditions:
    /// 1. `verify_alias_safety` returns `Ok(None)` when the deprecated set is
    ///    artificially emptied (simulating the old unconditional short-circuit) —
    ///    the wrapper sees NotPush, skips verification, and the destination
    ///    accumulates refs.
    /// 2. With the fix, `verify_alias_safety` expands `whatchanged` to `push …`
    ///    (returns `Some(expanded)`) and the destination stays empty.
    #[test]
    fn verify_push_deprecated_builtin_alias_is_expanded_and_refused() {
        // Skip cleanly when the installed git does not support --list-cmds=deprecated
        // (those binaries also lack the deprecated-builtin dispatch exception, so
        // there is nothing to test).
        let deprecated_set = git_deprecated_commands(&real_git());
        if deprecated_set.is_empty() {
            return;
        }
        // Confirm `whatchanged` is in the deprecated set on this git binary.
        // If git ever removes it from the deprecated list, skip rather than panic —
        // the test is only meaningful while the deprecated exception applies.
        if !deprecated_set.contains("whatchanged") {
            return;
        }
        // `whatchanged` must also appear in the builtins list for the test to be
        // exercising the exact overlap condition.
        let builtin_set = git_builtin_commands(&real_git());
        assert!(
            builtin_set.contains("whatchanged"),
            "`whatchanged` must be in --list-cmds=builtins for the deprecated-overlap test \
             to be meaningful; git binary: {:?}",
            real_git()
        );

        let (_d, repo) = human_authored_repo();
        let remote = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .status()
                .unwrap();
        };
        g(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]);

        // Wire alias.whatchanged = push origin main.
        // On binaries where --list-cmds=deprecated succeeds, git expands this
        // alias even though `whatchanged` is in --list-cmds=builtins, because
        // it is also deprecated (alias-first on such binaries).
        g(&[
            "-C",
            repo.to_str().unwrap(),
            "config",
            "alias.whatchanged",
            "push origin main",
        ]);

        // Mutation precondition part 1: with the OLD unconditional builtin short-
        // circuit (builtins set checked, deprecated set ignored), verify_alias_safety
        // would return Ok(None) for `whatchanged` — treating it as a real non-alias
        // builtin and producing no expansion.  We simulate this by confirming that
        // an implementation which treats the full builtins set as unconditionally
        // dominant would short-circuit: `whatchanged` IS in builtins, so the old
        // code would break without consulting alias config.
        // Direct evidence: if we call verify_alias_safety and the deprecated-aware
        // path is removed (builtin-only check), whatchanged is in builtins → Ok(None).
        // We verify the OPPOSITE holds with the fix: Some(expanded) containing push.
        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let check_argv = v(&["-C", repo.to_str().unwrap(), "whatchanged"]);
        let expansion = verify_alias_safety(&real_git(), &check_argv, &ctx)
            .expect("verify_alias_safety must not Err for alias.whatchanged=push");
        let expanded = expansion.expect(
            "verify_alias_safety must expand deprecated builtin `whatchanged` alias to \
             Some(argv); Ok(None) means the deprecated-builtin exception was not handled \
             and the bypass is live",
        );
        let sub = subcommand(&expanded).expect("expanded argv must have a subcommand");
        assert_eq!(
            sub, "push",
            "expansion must resolve `whatchanged` → `push origin main`; \
             got `{sub}` — deprecated-builtin alias not expanded correctly"
        );

        // End-to-end: run_inner must refuse the human-authored commit that would
        // reach `origin` via the alias.  On binaries where --list-cmds=deprecated
        // succeeds, real git expands the alias to `push origin main` and sends the
        // commit; the wrapper must intercept it via the expanded classification.
        let argv = v(&["-C", repo.to_str().unwrap(), "whatchanged"]);
        let exit = run_inner(argv, Some(managed()));
        assert_ne!(
            exit, 0,
            "run_inner must refuse human push routed through alias.whatchanged=push; \
             exit 0 means the deprecated builtin was treated as builtin-first and \
             verification was skipped"
        );

        // Destination must be empty — the guard must fire before any commit reaches
        // the remote.
        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty after deprecated-builtin alias refusal; \
             remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    // ── Carl R8 P1: replacement refs ──────────────────────────────────────────
    //
    // `git replace REAL DECOY` makes ordinary object reads (used by `git show`,
    // `git rev-list`, `git diff-tree`) resolve REAL through its replacement chain
    // to DECOY.  Pack transfer does NOT honour replacements: the destination
    // receives the original REAL object.  Without `--no-replace-objects` on all
    // verification probes, the wrapper inspects DECOY while sending REAL.
    //
    // Two classes of bypass:
    //   - Wrong-author replacement: REAL has a human author; DECOY has the agent
    //     email.  Without the fix, `commit_author_email(REAL)` returns DECOY's
    //     author (agent) — passing the author check.
    //   - Unsigned replacement: REAL is unsigned; DECOY carries a valid agent
    //     signature.  Without the fix, `commit_signature_is_agent(REAL)` reads
    //     DECOY's signature status — `G` — passing the signature check.

    /// (Carl R8 P1) A wrong-author commit with a `git replace` mapping to an
    /// agent-authored decoy is refused by the author gate.
    ///
    /// Mutation evidence: removing `--no-replace-objects` from
    /// `commit_author_email` makes this test pass (Ok) instead of Err — the
    /// commit_author_email call would then read DECOY's author (agent email),
    /// incorrectly passing the check.
    #[test]
    fn verify_push_rejects_wrong_author_via_replacement_ref() {
        // Build a repo with:
        //   REAL = wrong-author (human@example.com) commit
        //   DECOY = agent-authored unsigned commit with the same tree
        // then `git replace REAL DECOY` so that ordinary object reads see DECOY
        // when asked about REAL.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let g = |args: &[&str]| -> bool {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap()
                .success()
        };
        assert!(g(&["init", "-q", "-b", "main"]));
        assert!(g(&["config", "user.name", "Agent"]));
        assert!(g(&["config", "user.email", AGENT_EMAIL]));
        assert!(g(&["config", "commit.gpgSign", "false"]));

        // DECOY: agent-authored commit (what the replacement chain presents).
        // Use explicit --author to ensure the commit has the expected email
        // regardless of any ambient git identity in the environment.
        std::fs::write(repo.join("f"), "x").unwrap();
        assert!(g(&["add", "f"]));
        assert!(g(&[
            "commit",
            "-qm",
            "decoy",
            "--author",
            &format!("Agent <{AGENT_EMAIL}>"),
        ]));
        let decoy_sha = {
            let out = std::process::Command::new("git")
                .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        // REAL: wrong-author commit on a separate branch.
        // Use a distinct file content so REAL has a different OID than DECOY.
        assert!(g(&["checkout", "-qb", "real-branch"]));
        std::fs::write(repo.join("f"), "y").unwrap();
        assert!(g(&["add", "f"]));
        assert!(g(&[
            "commit",
            "-qm",
            "real",
            "--author",
            "Human <human@example.com>",
        ]));
        let real_sha = {
            let out = std::process::Command::new("git")
                .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        // Wire: REAL → DECOY replacement.
        assert!(
            g(&["replace", &real_sha, &decoy_sha]),
            "git replace must succeed"
        );

        // Sanity: without --no-replace-objects, git show REAL reports DECOY's author.
        // This proves the bypass is executable: ordinary `git show` follows the
        // replacement chain and reads DECOY's agent email instead of REAL's human email.
        let replaced_email = {
            let out = std::process::Command::new("git")
                .args([
                    "-C",
                    repo.to_str().unwrap(),
                    "show",
                    "-s",
                    "--format=%ae",
                    &real_sha,
                ])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(
            replaced_email, AGENT_EMAIL,
            "precondition: without --no-replace-objects, git show REAL reports DECOY's \
             author ({AGENT_EMAIL}) — the replacement bypass is executable"
        );

        // Sanity: with --no-replace-objects, git show reports REAL's raw author.
        let raw_email = {
            let out = std::process::Command::new("git")
                .args([
                    "-C",
                    repo.to_str().unwrap(),
                    "--no-replace-objects",
                    "show",
                    "-s",
                    "--format=%ae",
                    &real_sha,
                ])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(
            raw_email, "human@example.com",
            "precondition: --no-replace-objects must reveal REAL's true author"
        );

        // Set up a bare remote and push REAL to it.
        let remote = tempfile::tempdir().unwrap();
        assert!(g(&[
            "init",
            "-q",
            "--bare",
            remote.path().to_str().unwrap()
        ]));
        assert!(g(&[
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap()
        ]));

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];
        let argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "origin",
            &format!("{real_sha}:refs/heads/main"),
        ]);

        let err = verify_push(&real_git(), &argv, &argv, &ctx, &managed()).expect_err(
            "wrong-author REAL commit must be refused even with REAL→DECOY replacement; \
                 without --no-replace-objects commit_author_email would read DECOY's agent \
                 author and incorrectly pass the check",
        );
        assert!(
            err.contains("not authored by your agent identity"),
            "expected author-mismatch refusal; got: {err}"
        );

        // Destination must be empty.
        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty; remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    /// Locate the `git-sign-nostr` binary built alongside this test binary in
    /// the Cargo target directory.  Returns `None` when the binary is absent.
    /// Tests that require this binary must call `panic!` rather than skip on
    /// `None` — the test-unit recipe guarantees `git-sign-nostr` is built before
    /// the `buzz-git-identity` nextest run so absence always means a build error.
    #[cfg(unix)]
    fn find_git_sign_nostr() -> Option<std::path::PathBuf> {
        // current_exe = target/<profile>/deps/<test_binary>-<hash>
        // parent      = target/<profile>/deps
        // parent²     = target/<profile>
        let exe = std::env::current_exe().ok()?;
        let profile_dir = exe.parent()?.parent()?;
        let name = if cfg!(windows) {
            "git-sign-nostr.exe"
        } else {
            "git-sign-nostr"
        };
        let p = profile_dir.join(name);
        p.exists().then_some(p)
    }

    /// (Carl R8 P1) An unsigned agent-authored commit with a `git replace`
    /// mapping to a signed decoy is refused by the signature gate.
    ///
    /// The bypass: `git replace REAL DECOY` makes `commit_signature_is_agent`
    /// read DECOY's signature status (`G`) instead of REAL's (unsigned → `N`),
    /// incorrectly passing the signature check while REAL (unsigned) reaches the
    /// destination.
    ///
    /// Mutation evidence: removing `--no-replace-objects` from
    /// `commit_signature_is_agent` makes the end-to-end `verify_push` return Ok
    /// (the signature probe reads DECOY's `G` status, passing the check while
    /// the unsigned REAL reaches the destination).  With the fix, the probe
    /// reads REAL's raw unsigned state → `N` → refused and destination stays
    /// empty.
    ///
    /// Self-contained: uses a secp256k1 spec test vector keypair with the
    /// `git-sign-nostr` binary (guaranteed built by `just test-unit` before
    /// this nextest run) as the signing program.  Binary absence panics — it
    /// indicates a build failure, not a normal skip condition.  The ambient
    /// `GIT_CONFIG_*` env (harness-injected signing identity) is cleared via
    /// `GitConfigEnvGuard` so the test keypair is the only identity in effect.
    #[cfg(unix)]
    #[test]
    fn verify_push_rejects_unsigned_commit_via_replacement_ref() {
        // Acquire the env guard: clears all ambient GIT_CONFIG_* so the
        // test-vector keypair (not the harness's agent identity) controls signing.
        let _guard = clear_git_config_env();

        // Locate git-sign-nostr built alongside this test binary.
        // `just test-unit` builds it before running buzz-git-identity tests.
        let sign_nostr_bin = find_git_sign_nostr().unwrap_or_else(|| {
            panic!(
                "git-sign-nostr binary not found in Cargo target dir — \
                 run `cargo build -p git-sign-nostr` or `just test-unit`"
            )
        });

        // Test keypair: secp256k1 spec test vector (secret scalar = 3).
        // The public key is a known valid x-only BIP-340 point.
        const TEST_SECRET_HEX: &str =
            "0000000000000000000000000000000000000000000000000000000000000003";
        const TEST_PUBKEY_HEX: &str =
            "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
        let test_email = format!("{TEST_PUBKEY_HEX}@relay.test");

        // Write a 0600 keyfile with the test secret key.
        let key_dir = tempfile::tempdir().unwrap();
        let keyfile = key_dir.path().join(".nostr-key");
        std::fs::write(&keyfile, TEST_SECRET_HEX).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&keyfile, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();

        // Helper: run git in the repo with file-level global/system config
        // isolated.  The process GIT_CONFIG_* env is already clear (guard above).
        // PATH includes sign_nostr_bin's parent so the binary is on PATH when
        // repo config names it without a full path.
        let g = |args: &[&str]| -> bool {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("PATH", {
                    let parent = sign_nostr_bin.parent().unwrap();
                    let orig = std::env::var_os("PATH").unwrap_or_default();
                    let mut p = std::ffi::OsString::from(parent);
                    p.push(":");
                    p.push(orig);
                    p
                })
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        // Capture helper — same env isolation.
        let gc = |args: &[&str]| -> String {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("PATH", {
                    let parent = sign_nostr_bin.parent().unwrap();
                    let orig = std::env::var_os("PATH").unwrap_or_default();
                    let mut p = std::ffi::OsString::from(parent);
                    p.push(":");
                    p.push(orig);
                    p
                })
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        assert!(g(&["init", "-q", "-b", "main"]));
        assert!(g(&["config", "user.name", "Agent"]));
        assert!(g(&["config", "user.email", &test_email]));
        assert!(g(&["config", "gpg.format", "x509"]));
        assert!(g(&[
            "config",
            "gpg.x509.program",
            sign_nostr_bin.to_str().unwrap(),
        ]));
        assert!(g(&["config", "user.signingkey", TEST_PUBKEY_HEX]));
        assert!(g(&["config", "nostr.keyfile", keyfile.to_str().unwrap()]));
        assert!(g(&["config", "commit.gpgSign", "true"]));

        // Build the test authority — matches the repo-level signing config.
        // Authority fields are private but accessible here (same module).
        let test_authority = Authority {
            entries: vec![
                ("user.name".into(), "Agent".into()),
                ("user.email".into(), test_email.clone()),
                ("gpg.format".into(), "x509".into()),
                (
                    "gpg.x509.program".into(),
                    sign_nostr_bin.to_string_lossy().into_owned(),
                ),
                ("commit.gpgSign".into(), "true".into()),
                ("tag.gpgSign".into(), "true".into()),
                ("user.signingkey".into(), TEST_PUBKEY_HEX.into()),
                (
                    "nostr.keyfile".into(),
                    keyfile.to_string_lossy().into_owned(),
                ),
            ],
            email: test_email.clone(),
        };

        // DECOY: agent-authored, genuinely signed with the test keypair.
        // Must have %G? = "G" — the bypass precondition.
        std::fs::write(repo.join("f"), "decoy").unwrap();
        assert!(g(&["add", "f"]));
        let decoy_ok = g(&[
            "commit",
            "-qm",
            "decoy",
            "--author",
            &format!("Agent <{test_email}>"),
        ]);
        assert!(
            decoy_ok,
            "DECOY commit must succeed (git-sign-nostr must sign with the test keyfile)"
        );
        let decoy_sha = gc(&["rev-parse", "HEAD"]);

        // Sanity: DECOY has %G? = "G" (genuinely signed with the test keypair).
        let decoy_gq = gc(&["show", "-s", "--format=%G?", &decoy_sha]);
        assert_eq!(
            decoy_gq, "G",
            "DECOY must be a valid agent-signed commit (%G?=G); \
             got {decoy_gq:?} — git-sign-nostr may not be signing correctly"
        );

        // REAL: agent-authored but UNSIGNED (different content → different SHA).
        // --no-gpg-sign beats all config so REAL has no signature regardless of
        // what commit.gpgSign says in repo config.
        std::fs::write(repo.join("f"), "real").unwrap();
        assert!(g(&["add", "f"]));
        assert!(g(&[
            "commit",
            "--no-gpg-sign",
            "-qm",
            "real",
            "--author",
            &format!("Agent <{test_email}>"),
        ]));
        let real_sha = gc(&["rev-parse", "HEAD"]);

        // Wire: REAL → DECOY replacement.
        assert!(g(&["replace", &real_sha, &decoy_sha]));

        let ctx = vec!["-C".to_string(), repo.to_string_lossy().into_owned()];

        // Bypass precondition: without --no-replace-objects, git show REAL
        // follows the replacement and reports DECOY's %G? (G).
        let without_flag = gc(&["show", "-s", "--format=%G?", &real_sha]);
        assert_eq!(
            without_flag, "G",
            "bypass precondition: without --no-replace-objects, git show REAL \
             must report DECOY's %G? (G) — replacement chain is not active \
             or DECOY is not signed"
        );

        // The production `commit_signature_is_agent` (WITH --no-replace-objects)
        // must return Some(false): REAL is unsigned → %G? = "N".
        //
        // Mutation guard: removing --no-replace-objects from
        // commit_signature_is_agent makes the probe follow the replacement chain,
        // read DECOY's G status, and return Some(true) — flipping this assertion.
        let sig_result = commit_signature_is_agent(&real_git(), &ctx, &test_authority, &real_sha);
        assert_eq!(
            sig_result,
            Some(false),
            "commit_signature_is_agent must return Some(false) for REAL (unsigned) \
             with --no-replace-objects: got {sig_result:?}. \
             Some(true) means --no-replace-objects is missing and the bypass is live."
        );

        // End-to-end: verify_push must refuse REAL (unsigned) even with
        // the REAL → DECOY replacement active.
        let remote = tempfile::tempdir().unwrap();
        assert!(g(&[
            "init",
            "-q",
            "--bare",
            remote.path().to_str().unwrap(),
        ]));
        assert!(g(&[
            "remote",
            "add",
            "origin",
            remote.path().to_str().unwrap(),
        ]));

        let argv = v(&[
            "-C",
            repo.to_str().unwrap(),
            "push",
            "origin",
            &format!("{real_sha}:refs/heads/main"),
        ]);
        let err = verify_push(&real_git(), &argv, &argv, &ctx, &test_authority).expect_err(
            "REAL (unsigned agent commit) must be refused even with REAL→DECOY \
                 replacement; without --no-replace-objects the signature probe reads \
                 DECOY's G status and incorrectly passes",
        );
        assert!(
            err.contains("no valid signature"),
            "expected signature refusal; got: {err}"
        );

        // Destination must not have acquired REAL.
        let ls = std::process::Command::new("git")
            .args(["ls-remote", remote.path().to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ls.stdout).trim().is_empty(),
            "destination must remain empty after unsigned-via-replacement refusal; \
             remote_refs={:?}",
            String::from_utf8_lossy(&ls.stdout)
        );
    }

    // ── Carl R8 P2: transport parsing — inline config and --repo= ─────────────
    //
    // The `://` scheme check in `inspect_push_config` Surface 1 must inspect the
    // SCHEME FIELD of a URL, not the entire token before `://`.  For tokens like
    // `remote.origin.url=https://host/repo.git` (a `-c` value) or
    // `--repo=https://host/repo.git`, the `=` sign separates a key/option name
    // from the URL value.  Treating `remote.origin.url=https` as the scheme
    // (everything before `://`) would incorrectly refuse a valid builtin
    // transport.

    /// (Carl R8 P2) Inline `-c remote.origin.url=https://…` config with a valid
    /// builtin scheme is accepted by `inspect_push_config`.
    ///
    /// Mutation evidence: reverting the scheme extraction to use the entire token
    /// before `://` (`&tok.as_bytes()[..sep]`) makes this test return Err
    /// (false rejection of `remote.origin.url=https` as an unknown scheme)
    /// instead of allowing the push to proceed to the author/signature gate.
    #[test]
    fn verify_push_allows_inline_config_url_with_builtin_scheme() {
        // Use a file:// URL to trigger the `://` scheme check on a `-c` value token.
        // With the OLD extraction (no `=` stripping), the scheme is
        // `remote.origin.url=file` — not builtin → refused.
        // With the NEW extraction (strip up to last `=`), the scheme is `file` → allowed.
        let file_url = "file:///tmp/some/repo";
        let inline_file_tok = format!("remote.origin.url={file_url}");

        // Old extraction: scheme = "remote.origin.url=file" (everything before ://).
        let old_scheme = inline_file_tok
            .as_bytes()
            .windows(3)
            .position(|w| w == b"://")
            .map(|sep| std::str::from_utf8(&inline_file_tok.as_bytes()[..sep]).unwrap())
            .unwrap_or("");
        assert_ne!(
            old_scheme, "file",
            "mutation evidence: old extraction gets `{old_scheme}` (not `file`) for \
             `{inline_file_tok}` — it would be refused as an unknown scheme, confirming \
             the = stripping is the fix"
        );
        assert!(
            !is_builtin_url_scheme(old_scheme.as_bytes()),
            "old scheme `{old_scheme}` must NOT be in the builtin list — confirms the \
             false-rejection the fix corrects"
        );

        // New extraction: scheme starts after the last `=` before `://`.
        let new_scheme = {
            let tok = &inline_file_tok;
            let sep = tok.as_bytes().windows(3).position(|w| w == b"://").unwrap();
            let scheme_start = tok.as_bytes()[..sep]
                .iter()
                .rposition(|&b| b == b'=')
                .map(|i| i + 1)
                .unwrap_or(0);
            &tok[scheme_start..sep]
        };
        assert_eq!(
            new_scheme, "file",
            "new extraction must yield `file` from `{inline_file_tok}`"
        );
        assert!(
            is_builtin_url_scheme(new_scheme.as_bytes()),
            "new scheme `file` must be in the builtin list — confirms the fix allows it"
        );

        // inspect_push_config Surface 1 must NOT refuse the -c value token.
        // We pass it as an effective_argv token that appears after the subcommand.
        // The function scans all tokens; the `://` scheme check must pass for
        // `remote.origin.url=file://…`.
        //
        // We do not need a reachable remote here — if the function returns Ok or
        // fails for a reason OTHER than "unknown URL scheme", the transport check passed.
        let ctx: Vec<String> = vec![];
        let argv = v(&["push", "-c", &inline_file_tok, "origin", "main"]);
        let result = inspect_push_config(&real_git(), &argv, &ctx);
        if let Err(ref msg) = result {
            assert!(
                !msg.contains("unknown URL scheme"),
                "must NOT refuse `{inline_file_tok}` as unknown scheme — the fix extracts \
                 the scheme after `=`; got: {msg}"
            );
        }

        // Also verify with https:// in a -c value token.
        let https_tok = "remote.origin.url=https://host/repo.git";
        let argv_https = v(&["push", "-c", https_tok, "origin", "main"]);
        let result_https = inspect_push_config(&real_git(), &argv_https, &ctx);
        if let Err(ref msg) = result_https {
            assert!(
                !msg.contains("unknown URL scheme"),
                "must NOT refuse https:// in -c value token; got: {msg}"
            );
        }
    }

    /// (Carl R8 P2) `--repo=https://…` with a valid builtin scheme in argv is
    /// accepted by `inspect_push_config` (scheme extracted after the `=`).
    ///
    /// Mutation evidence: reverting to the full-token scheme extraction
    /// (`&tok.as_bytes()[..sep]`) would return `--repo=https` as the scheme,
    /// which is not builtin — causing a false rejection.
    #[test]
    fn verify_push_allows_attached_repo_flag_with_builtin_scheme() {
        let ctx: Vec<String> = vec![];
        // `--repo=https://host/repo.git` — a bare URL token with an option prefix.
        let repo_tok = "--repo=https://host/repo.git";

        // Old extraction: scheme = "--repo=https" — not builtin, would be refused.
        let old_scheme = repo_tok
            .as_bytes()
            .windows(3)
            .position(|w| w == b"://")
            .map(|sep| std::str::from_utf8(&repo_tok.as_bytes()[..sep]).unwrap())
            .unwrap_or("");
        assert_ne!(
            old_scheme, "https",
            "mutation evidence: old extraction yields `{old_scheme}` (not `https`) — \
             the false rejection is confirmed, so the = stripping is the load-bearing fix"
        );

        // New extraction: scheme starts after the last `=` before `://`.
        let new_scheme = {
            let tok = repo_tok;
            let sep = tok.as_bytes().windows(3).position(|w| w == b"://").unwrap();
            let scheme_start = tok.as_bytes()[..sep]
                .iter()
                .rposition(|&b| b == b'=')
                .map(|i| i + 1)
                .unwrap_or(0);
            &tok[scheme_start..sep]
        };
        assert_eq!(
            new_scheme, "https",
            "new extraction must yield `https` from `{repo_tok}`"
        );

        // inspect_push_config Surface 1 must not refuse this token.
        // Build a minimal effective_argv carrying this token.
        let argv = v(&["push", "--repo=https://host/repo.git", "main"]);
        // inspect_push_config will fail because there's no real remote / config,
        // but the error must NOT be "unknown URL scheme".
        let result = inspect_push_config(&real_git(), &argv, &ctx);
        if let Err(ref msg) = result {
            assert!(
                !msg.contains("unknown URL scheme"),
                "must NOT refuse --repo=https://… as unknown scheme; got: {msg}"
            );
        }
        // (If it returns Ok, that's fine too — the point is no scheme rejection.)
    }

    // ── Windows Job Object tree-ownership regressions ─────────────────────────
    //
    // These tests exercise the `capture_raw_bounded` Windows path directly on a
    // real `windows-latest` CI runner.  Each test:
    //   1. Spawns a PowerShell root that launches a hidden detached descendant.
    //   2. Lets the descendant record its own PID (proving it existed inside the
    //      job before teardown).
    //   3. Asserts the descendant is dead after the helper returns.
    //
    // The `#[ignore]` tag is paired with an explicit `--ignored` step in the
    // Windows CI job so these tests actually execute; they are not dead-letter.
    //
    // Mutation: removing `BoundedJob` / making `kill_bounded_tree` a no-op on
    // Windows leaves the descendant alive and the PID-alive assert fails.

    /// Probe whether a Windows process is still running by opening a handle
    /// with `PROCESS_QUERY_LIMITED_INFORMATION` and checking its exit code.
    /// `STILL_ACTIVE` (259) means running; any other exit code (or a failed
    /// open because the PID is gone) means dead.
    #[cfg(windows)]
    fn windows_pid_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut code);
            CloseHandle(handle);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }

    /// Read a PID that a PowerShell payload wrote to `path`, retrying briefly
    /// since the descendant records it asynchronously.  Returns the PID or panics
    /// with a diagnostic message after a deadline.
    #[cfg(windows)]
    fn read_windows_pid(path: &str) -> u32 {
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    return pid;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("descendant never recorded its PID at {path}");
    }

    /// On the success path, `capture_raw_bounded` must reap a descendant that
    /// the root backgrounds before exiting.  The Job Object assigns the root
    /// before any code runs, so the descendant is born inside it; closing the
    /// job on the success path kills the whole tree even after the root exits.
    ///
    /// Non-vacuous: the descendant's PID is asserted dead.  Without the Job
    /// Object (`Child::kill()` only), `kill_bounded_tree` kills the root but
    /// not the descendant, which remains alive and the assert fails.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows host; executed by the 'Test (buzz-git-identity)' CI step with --ignored"]
    fn capture_raw_bounded_reaps_descendant_on_success_windows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_path = dir.path().join("descendant.pid");
        let child_ps1 = dir.path().join("child.ps1");
        let root_ps1 = dir.path().join("root.ps1");
        let pid_s = pid_path.to_str().expect("utf-8 pid path");
        let child_s = child_ps1.to_str().expect("utf-8 child path");

        // Descendant: record own PID, sleep 30 s.
        std::fs::write(
            &child_ps1,
            format!(
                "$PID | Set-Content -Encoding ascii -Path '{pid_s}'\nStart-Sleep -Seconds 30\n"
            ),
        )
        .expect("write child ps1");
        // Root: launch descendant, wait until PID recorded, then exit 0.
        std::fs::write(
            &root_ps1,
            format!(
                "Start-Process -FilePath 'powershell' -WindowStyle Hidden \
                 -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{child_s}'\n\
                 $d = (Get-Date).AddSeconds(15)\n\
                 while ((-not (Test-Path '{pid_s}') -or \
                 (Get-Item '{pid_s}').Length -eq 0) -and (Get-Date) -lt $d) \
                 {{ Start-Sleep -Milliseconds 50 }}\n\
                 exit 0\n"
            ),
        )
        .expect("write root ps1");

        let root_s = root_ps1.to_str().expect("utf-8 root path");
        let out = capture_raw_bounded(
            Path::new("powershell"),
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", root_s],
            std::time::Duration::from_secs(20),
        )
        .expect("root exits 0, so capture_raw_bounded must return Some");
        assert!(out.status.success(), "root must exit 0");

        let descendant_pid = read_windows_pid(pid_s);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !windows_pid_alive(descendant_pid),
            "descendant {descendant_pid} must be reaped by the kill-on-close job on \
             the success path, but it is still alive"
        );
    }

    /// On the timeout path, `capture_raw_bounded` must reap a descendant that
    /// the root backgrounds.  The root itself loops forever, triggering the
    /// deadline; closing the job reaps both the root and the descendant.
    ///
    /// Non-vacuous: the descendant's PID is asserted dead after the helper
    /// returns `None`.  Without the Job Object (`Child::kill()` only), the
    /// descendant survives the direct-child kill and the assert fails.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows host; executed by the 'Test (buzz-git-identity)' CI step with --ignored"]
    fn capture_raw_bounded_reaps_descendant_on_timeout_windows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_path = dir.path().join("descendant.pid");
        let child_ps1 = dir.path().join("child.ps1");
        let root_ps1 = dir.path().join("root.ps1");
        let pid_s = pid_path.to_str().expect("utf-8 pid path");
        let child_s = child_ps1.to_str().expect("utf-8 child path");

        std::fs::write(
            &child_ps1,
            format!(
                "$PID | Set-Content -Encoding ascii -Path '{pid_s}'\nStart-Sleep -Seconds 300\n"
            ),
        )
        .expect("write child ps1");
        // Root: launch descendant, wait until PID recorded, then loop forever
        // so the helper's deadline fires inside the root.
        std::fs::write(
            &root_ps1,
            format!(
                "Start-Process -FilePath 'powershell' -WindowStyle Hidden \
                 -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{child_s}'\n\
                 $d = (Get-Date).AddSeconds(15)\n\
                 while ((-not (Test-Path '{pid_s}') -or \
                 (Get-Item '{pid_s}').Length -eq 0) -and (Get-Date) -lt $d) \
                 {{ Start-Sleep -Milliseconds 50 }}\n\
                 Start-Sleep -Seconds 300\n"
            ),
        )
        .expect("write root ps1");

        let root_s = root_ps1.to_str().expect("utf-8 root path");
        // Short timeout so the test is fast; the watchdog is the outer wall-clock.
        let result = capture_raw_bounded(
            Path::new("powershell"),
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", root_s],
            std::time::Duration::from_secs(5),
        );
        assert!(result.is_none(), "a timed-out tree must yield None");

        let descendant_pid = read_windows_pid(pid_s);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !windows_pid_alive(descendant_pid),
            "descendant {descendant_pid} must be job-killed on timeout, but it is still alive"
        );
    }
}
