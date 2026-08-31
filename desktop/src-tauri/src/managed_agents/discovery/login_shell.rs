//! Login-shell PATH discovery and nvm fallback.
//!
//! Extracted verbatim from `discovery.rs` to keep that file under the
//! file-size ratchet. Covers login-shell candidate selection, the cached
//! login-shell PATH probe, and nvm default-bin resolution.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::is_executable_file;

/// Per-candidate wall-clock bound for a login-shell spawn. Matches the auth
/// probe's 10s discipline: long enough for a healthy interactive shell to
/// source its rc files, short enough that a wedged shell can't stall discovery.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// Test-only spawn counter lives beside `discovery.rs`; import it here so the
/// spawn-record call site stays byte-identical to the pre-extraction source.
#[cfg(test)]
use super::login_shell_spawn_probe;

/// Collect login shell candidates for the current platform.
///
/// On Unix: `/bin/zsh`, `/bin/bash` (the historical defaults).
/// On Windows: Git Bash via `resolve_bash_path` — skips `BUZZ_SHELL` because
/// login-shell callers use bash-only `-l -c` syntax.
pub(crate) fn login_shell_candidates() -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/bin/zsh"), PathBuf::from("/bin/bash")]
    }
    #[cfg(windows)]
    {
        super::super::git_bash::resolve_bash_path()
            .into_iter()
            .collect()
    }
}

/// Run a command in a login shell (tries zsh then bash on Unix, Git Bash on Windows).
/// Returns trimmed stdout if the command succeeds with non-empty output.
///
/// Each candidate shell is bounded by [`LOGIN_SHELL_TIMEOUT`]: a shell whose
/// startup blocks (an interactive prompt in `.zshrc`, a stalled network mount,
/// a credential helper waiting on input) is killed and treated as a miss so the
/// loop falls through to the next candidate rather than hanging the whole
/// discovery. Without this bound a single slow login shell froze the forced
/// pipeline indefinitely, which is what left "Check again" spinning forever.
fn run_in_login_shell(args: &[&str]) -> Option<String> {
    #[cfg(test)]
    login_shell_spawn_probe::record();
    for shell in login_shell_candidates() {
        let mut cmd = Command::new(&shell);
        cmd.args(args);
        // Window suppression is owned by `output_with_timeout`'s spawn
        // (`BOUNDED_CREATION_FLAGS` carries `CREATE_NO_WINDOW`); a
        // `configure_no_window` call here would be clobbered by that later
        // `creation_flags` set, so it is deliberately omitted.
        let Some(output) = super::bounded_command::output_with_timeout(cmd, LOGIN_SHELL_TIMEOUT)
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            return Some(stdout);
        }
    }
    None
}

pub(crate) fn find_via_login_shell(command: &str) -> Option<PathBuf> {
    let stdout = run_in_login_shell(&["-l", "-c", r#"command -v -- "$1""#, "_", command])?;
    let resolved = stdout.lines().rfind(|line| !line.trim().is_empty())?;
    let path = PathBuf::from(resolved.trim());
    (path.is_absolute() && is_executable_file(&path)).then_some(path)
}

/// Three-state backing store for the login-shell PATH cache.
#[derive(Clone)]
enum LoginShellPath {
    /// Cache has never been populated; the next call will spawn a login shell.
    Uninit,
    /// A login shell was invoked; the inner value is the PATH it returned
    /// (`None` when the shell produced no output).
    Probed(Option<String>),
}

/// Cache plus a monotonic generation counter. `refresh_login_shell_path` bumps
/// the generation and resets the state together; a probe records the generation
/// it started under and may only publish its result while that generation is
/// still current. This stops a slow, pre-refresh probe from committing a stale
/// (often false-negative) PATH over the fresh value a post-refresh probe wrote.
struct PathCache {
    generation: u64,
    state: LoginShellPath,
}

fn path_cache() -> &'static std::sync::Mutex<PathCache> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<PathCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(PathCache {
            generation: 0,
            state: LoginShellPath::Uninit,
        })
    })
}

fn fetch_login_shell_path_inner() -> Option<String> {
    // On Windows, Git Bash's `echo $PATH` returns POSIX colon-delimited paths
    // (`/mingw64/bin:/c/Users/...`) which poison native Windows children that
    // split on `;`. login_shell_path() feeds agent_models, runtime, and
    // cli_probe — all native processes. Return None so they inherit the real
    // Windows PATH instead.
    #[cfg(windows)]
    {
        return None;
    }

    #[cfg(not(windows))]
    {
        let stdout = run_in_login_shell(&["-l", "-c", "echo $PATH"])?;
        let last_line = stdout.lines().rfind(|l| !l.trim().is_empty())?;
        Some(last_line.trim().to_string())
    }
}

/// Return the user's full PATH from a login shell.
///
/// The result is cached after the first call. Call [`refresh_login_shell_path`]
/// to invalidate the cache so the next call re-fetches — e.g. after the user
/// installs Node.js mid-session and clicks Retry.
///
/// The lock is never held while the login shell spawns: we read the cached
/// value and the current generation, release the lock, run the shell, then
/// re-lock to publish. Publication is generation-guarded so a probe that
/// started before a [`refresh_login_shell_path`] can never overwrite the fresh
/// value: if the generation moved while the probe ran, its result is discarded.
/// Within one generation two callers may both probe; a failure/timeout result
/// (`None`) never clobbers an already-committed success, so a slow timeout can't
/// undo a peer's fresh PATH.
///
/// The caller never returns its own local probe result: after publishing it
/// returns the value now in the cache. This closes two divergences where a
/// caller's own result contradicted the authoritative cache:
///   - same-generation timeout-vs-success — a peer committed a success while
///     our probe timed out (`None`); we return the peer's success, not `None`;
///   - a pre-refresh probe whose writeback was generation-rejected — its local
///     value is stale, so we re-probe under the new generation instead.
pub fn login_shell_path() -> Option<String> {
    loop {
        // Fast path: return the cached result and capture the generation the
        // probe will run under, all under a single lock.
        let generation = {
            let guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
            if let LoginShellPath::Probed(ref result) = guard.state {
                return result.clone();
            }
            guard.generation
        };

        // Slow path: spawn shell outside any lock.
        let result = probe_login_shell_path();

        // Publish under our generation, then return whatever value is now
        // authoritative. `None` means a refresh invalidated our generation
        // mid-probe and no fresh value is cached yet, so our `result` is stale
        // by definition — discard it and re-probe under the new generation.
        //
        // Termination: another lap requires another [`refresh_login_shell_path`]
        // to land during a probe. Refreshes come only from discrete human
        // actions (install/retry/Doctor re-run) and one-shot boot warm, so the
        // loop cannot spin unbounded.
        if let Some(committed) = publish_probe_result(generation, result) {
            return committed;
        }
    }
}

/// Real login-shell probe. A `cfg(test)` seam lets the race tests inject
/// deterministic probe results (and side effects) without spawning shells.
#[cfg(not(test))]
fn probe_login_shell_path() -> Option<String> {
    fetch_login_shell_path_inner()
}

#[cfg(test)]
fn probe_login_shell_path() -> Option<String> {
    match path_cache_race_tests::take_injected_probe() {
        Some(injected) => injected(),
        None => fetch_login_shell_path_inner(),
    }
}

/// Commit a probe's `result` under the generation it started with, then report
/// the value the caller should return.
///
/// A probe whose generation is stale (a [`refresh_login_shell_path`] ran while
/// it was probing) does not commit. Within a live generation a failure/timeout
/// (`None`) never overwrites an already-committed success. This is the sole
/// writer of a probed value, so the two race outcomes are decided here.
///
/// Returns `Some(v)` — the now-cached probed value the caller must return
/// (its own commit, or a peer's success that superseded it) — or `None` when
/// the cache is `Uninit` because a refresh landed mid-probe, signalling the
/// caller to re-probe under the new generation. Commit and re-read happen under
/// one lock so no refresh can slip between them.
fn publish_probe_result(generation: u64, result: Option<String>) -> Option<Option<String>> {
    let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
    if guard.generation == generation {
        let keep_committed_success =
            result.is_none() && matches!(guard.state, LoginShellPath::Probed(Some(_)));
        if !keep_committed_success {
            guard.state = LoginShellPath::Probed(result);
        }
    }
    match guard.state {
        LoginShellPath::Probed(ref v) => Some(v.clone()),
        LoginShellPath::Uninit => None,
    }
}

/// Invalidate the login-shell PATH cache so the next [`login_shell_path`] call
/// re-fetches from a fresh login shell.
///
/// Called before every install/retry operation and on Doctor Re-run so a
/// newly-installed tool becomes visible without restarting the app. Bumping the
/// generation revokes any in-flight probe's writeback, so a shell that started
/// before this refresh cannot recache its now-stale result.
pub(crate) fn refresh_login_shell_path() {
    let mut guard = path_cache().lock().unwrap_or_else(|e| e.into_inner());
    guard.generation = guard.generation.wrapping_add(1);
    guard.state = LoginShellPath::Uninit;
}

#[cfg(test)]
pub(crate) fn is_login_shell_path_uninit() -> bool {
    matches!(
        path_cache().lock().unwrap_or_else(|e| e.into_inner()).state,
        LoginShellPath::Uninit
    )
}

/// Return `true` when `tag` is a safe nvm alias/version tag that can be joined
/// onto a `PathBuf` without escaping the nvm root.
///
/// nvm uses tags like `v22.1.0` or `lts/hydrogen`. We allow ASCII alphanumeric
/// plus `. - / _` and require that no path component is `..` and that the tag
/// does not start with `/` (which would replace the base in `PathBuf::join`).
pub(crate) fn is_safe_nvm_tag(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    // An absolute path in the alias file would let PathBuf::join silently
    // replace the nvm root with an attacker-controlled path.
    if tag.starts_with('/') {
        return false;
    }
    // Reject any .. component to prevent upward traversal.
    for component in tag.split('/') {
        if component == ".." {
            return false;
        }
    }
    // Allow only the characters nvm uses in real tag names.
    tag.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '/' | '_'))
}

/// Locate the `bin` directory for nvm's default Node.js version.
///
/// Reads `~/.nvm/alias/default`; resolves at most one alias hop to handle
/// nvm alias chains; falls back to the highest-semver directory under
/// `~/.nvm/versions/node/`. Returns the `bin` subdirectory only when it exists.
///
/// Cheap: at most two file reads or one `read_dir`. Never cached — computed
/// fresh per call so a mid-session `nvm install` is visible at the next spawn.
pub fn find_nvm_default_bin(home: &Path) -> Option<PathBuf> {
    let nvm_root = home.join(".nvm");
    let versions_root = nvm_root.join("versions").join("node");

    // 1. Try alias/default, with at most one hop.
    let default_alias = nvm_root.join("alias").join("default");
    if let Ok(content) = std::fs::read_to_string(&default_alias) {
        let tag = content.trim().to_string();
        if is_safe_nvm_tag(&tag) {
            let candidate = versions_root.join(&tag).join("bin");
            if candidate.is_dir() {
                return Some(candidate);
            }
            // One alias hop: ~/.nvm/alias/<tag>
            let hop_file = nvm_root.join("alias").join(&tag);
            if let Ok(hop_content) = std::fs::read_to_string(&hop_file) {
                let hop_tag = hop_content.trim().to_string();
                if is_safe_nvm_tag(&hop_tag) {
                    let hop_candidate = versions_root.join(&hop_tag).join("bin");
                    if hop_candidate.is_dir() {
                        return Some(hop_candidate);
                    }
                }
            }
        }
    }

    // 2. Fall back to highest-semver directory under ~/.nvm/versions/node/.
    let entries = std::fs::read_dir(&versions_root).ok()?;
    let best = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy().into_owned();
            parse_semver_tag(&s).map(|v| (v, s))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b));

    let (_, tag) = best?;
    let bin = versions_root.join(&tag).join("bin");
    bin.is_dir().then_some(bin)
}

/// Parse a `vMAJ.MIN.PATCH` (or `vMAJ.MIN.PATCH-extra`) tag into a numeric
/// triple for semver comparison.
pub(crate) fn parse_semver_tag(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v')?;
    let mut parts = s.splitn(3, '.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_str = parts.next()?;
    let patch = patch_str.split('-').next()?.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod path_cache_race_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};

    /// A deterministic stand-in for one login-shell spawn. Returning it lets a
    /// test drive `login_shell_path`'s slow path without a real shell, and run
    /// side effects (a peer commit, a mid-probe refresh) at the exact moment a
    /// probe would be executing.
    pub(super) type InjectedProbe = Box<dyn FnOnce() -> Option<String> + Send>;

    fn probe_queue() -> &'static Mutex<VecDeque<InjectedProbe>> {
        static Q: OnceLock<Mutex<VecDeque<InjectedProbe>>> = OnceLock::new();
        Q.get_or_init(|| Mutex::new(VecDeque::new()))
    }

    /// Consumed by the `cfg(test)` `probe_login_shell_path` seam: each slow-path
    /// probe pops the next injected result, falling back to the real shell when
    /// the queue is empty (so unrelated cache tests still exercise real probing).
    pub(super) fn take_injected_probe() -> Option<InjectedProbe> {
        probe_queue()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
    }

    fn inject_probes(probes: Vec<InjectedProbe>) {
        let mut q = probe_queue().lock().unwrap_or_else(|e| e.into_inner());
        q.clear();
        q.extend(probes);
    }

    fn cached_probe() -> Option<Option<String>> {
        match path_cache().lock().unwrap_or_else(|e| e.into_inner()).state {
            LoginShellPath::Uninit => None,
            LoginShellPath::Probed(ref v) => Some(v.clone()),
        }
    }

    fn generation() -> u64 {
        path_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
    }

    /// A probe that started before a refresh must not recache its stale result.
    /// Models the P1 interleaving: probe A captures generation G; a forced
    /// refresh bumps to G+1 and (via probe B) commits a fresh PATH; then A
    /// finishes late and tries to publish. A publishes a non-empty *success*
    /// (`/stale/bin`), which the same-generation `None`-over-`Some` rule would
    /// accept — so only the generation guard can reject it. This keeps the test
    /// non-vacuous: delete the generation comparison and stale overwrites fresh.
    #[test]
    fn stale_probe_cannot_commit_after_refresh() {
        let _guard = crate::managed_agents::lock_path_mutex();
        refresh_login_shell_path();

        // Probe A starts here.
        let gen_a = generation();

        // A forced refresh invalidates the cache; probe B (new generation) then
        // commits a fresh PATH.
        refresh_login_shell_path();
        let gen_b = generation();
        assert_ne!(gen_a, gen_b, "refresh must bump the generation");
        publish_probe_result(gen_b, Some("/fresh/bin".to_string()));

        // Probe A finishes late and tries to publish a *stale success* under
        // its old generation. Only the generation guard can reject this — the
        // same-generation success-retention rule would let a `Some` through.
        publish_probe_result(gen_a, Some("/stale/bin".to_string()));

        assert_eq!(
            cached_probe(),
            Some(Some("/fresh/bin".to_string())),
            "a pre-refresh probe must not overwrite the post-refresh fresh PATH"
        );

        // Restore the shared cache so sibling tests re-probe a real PATH rather
        // than reading this fixture value.
        refresh_login_shell_path();
    }

    /// Within one generation a slow failure/timeout must not clobber a peer's
    /// already-committed success. Two cold callers race under generation G: the
    /// success lands first, the timeout (`None`) lands second and is dropped.
    #[test]
    fn timeout_does_not_clobber_committed_success() {
        let _guard = crate::managed_agents::lock_path_mutex();
        refresh_login_shell_path();
        let gen = generation();

        // Caller 1 succeeds.
        publish_probe_result(gen, Some("/usr/local/bin".to_string()));
        // Caller 2 times out later in the same generation.
        publish_probe_result(gen, None);

        assert_eq!(
            cached_probe(),
            Some(Some("/usr/local/bin".to_string())),
            "a same-generation timeout must not overwrite a committed success"
        );

        // Restore the shared cache so sibling tests re-probe a real PATH rather
        // than reading this fixture value.
        refresh_login_shell_path();
    }

    /// P1 #2, divergence (a): same-generation timeout-vs-success. A caller
    /// whose own probe times out (`None`) must still return the success a peer
    /// committed under the same generation — never its own `None`, which would
    /// let a forced discovery on this thread settle a PATH-missing UI while the
    /// authoritative cache holds the peer's success.
    ///
    /// Injected probe: commit the peer's `/peer/bin` success, then return `None`
    /// (this caller's timeout). Non-vacuous for the "return authoritative value"
    /// rule: return the local result instead and this yields `None`.
    #[test]
    fn caller_returns_peer_success_not_own_timeout() {
        let _guard = crate::managed_agents::lock_path_mutex();
        refresh_login_shell_path();
        let gen = generation();

        inject_probes(vec![Box::new(move || {
            // A peer probe finishes first and commits a success under gen.
            publish_probe_result(gen, Some("/peer/bin".to_string()));
            // Our probe then times out.
            None
        })]);

        assert_eq!(
            login_shell_path(),
            Some("/peer/bin".to_string()),
            "a timed-out caller must return the peer's committed success, not its own None"
        );

        refresh_login_shell_path();
    }

    /// P1 #2, divergence (b): a pre-refresh probe whose writeback is
    /// generation-rejected must not return its stale local value; the caller
    /// re-probes under the new generation and returns the fresh result.
    ///
    /// First injected probe refreshes mid-flight (bumping the generation) and
    /// returns a stale `/stale/bin`; publication is rejected, so the caller
    /// loops and the second probe returns the fresh `/fresh/bin`. Non-vacuous
    /// for the re-probe rule: return the stale local value on a rejected commit
    /// instead and this yields `/stale/bin`.
    #[test]
    fn caller_reprobes_after_midprobe_refresh() {
        let _guard = crate::managed_agents::lock_path_mutex();
        refresh_login_shell_path();

        inject_probes(vec![
            Box::new(|| {
                // A forced refresh lands while this probe runs, invalidating the
                // generation it started under; its result is stale by definition.
                refresh_login_shell_path();
                Some("/stale/bin".to_string())
            }),
            Box::new(|| Some("/fresh/bin".to_string())),
        ]);

        assert_eq!(
            login_shell_path(),
            Some("/fresh/bin".to_string()),
            "a generation-rejected probe must re-probe, never return its stale local value"
        );

        refresh_login_shell_path();
    }
}
