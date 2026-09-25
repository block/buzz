//! Process-level, mutation-sensitive regression for the deterministic
//! agent-git-identity enforcement wrapper. Unlike the unit tests in
//! `buzz-git-identity`, this exercises the REAL multicall binary: `buzz-acp`
//! symlinked as `git`, invoked exactly as an agent's shell would invoke it,
//! with a `.git-identity` manifest beside the symlink (the harness-owned
//! authority) and the real `git` reachable later on PATH.
//!
//! Each test targets one enforcement layer and is designed to go RED if that
//! layer is deleted:
//!   * `enforce`            — flag-based identity/signing override is rejected.
//!   * `verify_push`        — a human-authored outgoing commit cannot be pushed.
//!   * `apply_authority_env`— the agent identity is re-applied over caller/repo
//!     config (the env-var override vector), so commits land agent-authored
//!     even when repo-local config names a human.
//!
//! The whole suite is unix-only: enforcement installs the wrapper as a PATH
//! symlink and every test wires a real `git-sign-nostr` signer via
//! [`signed_shim_env`], both of which need unix symlinks. buzz-acp's tests do
//! not run on Windows CI; this gate keeps `cargo check --all-targets` there
//! from compiling helpers it can never exercise.
#![cfg(unix)]

use nostr::ToBech32;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const ALIAS_HOP_LIMIT: usize = 10;

/// A named manifest mutation: a label and a fn that rewrites the manifest body.
/// Aliased to keep the tampered-manifest table under `clippy::type_complexity`.
type ManifestMutation = (&'static str, fn(&str) -> String);

/// Directory of the first real `git` on PATH; the wrapper is installed ahead
/// of it so `find_real_git` skips our shim symlink and reaches this one.
fn real_git_dir() -> PathBuf {
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let cand = dir.join("git");
        if cand.is_file() {
            return dir;
        }
    }
    panic!("no real git on PATH");
}

/// Repository-local env git exports into hook processes. Under a pre-push hook
/// in a linked worktree `GIT_DIR` is absolute, so an inherited value escapes
/// `current_dir`/`-C` and fixture commands would rewrite the real repository.
const INHERITED_GIT_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];

/// `Command::new(program)` with [`INHERITED_GIT_ENV`] removed. Every test spawn
/// that runs git (directly, via the wrapper, or via a child that runs git) goes
/// through this so the fixture only ever touches its own tempdir repos.
fn hermetic_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    for var in INHERITED_GIT_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// Verdict returned by the isolated capability probe helpers.
///
/// Mirrors the production `SubsectionSupport` discrimination:
///   - `Supported`:   exit 0 + "git version" stdout
///   - `Unsupported`: exit 1 + empty stdout + stderr starting with
///     `git: '<sentinel>' is not a git command`
///   - `Failure`:     setup error, spawn failure, timeout, output overflow, or
///     unclassifiable output — callers `panic!` (fail the test)
#[derive(Debug, PartialEq, Eq)]
enum ProbeVerdict {
    Supported,
    Unsupported,
    Failure,
}

/// Probe timeout — same order of magnitude as the production `PROBE_TIMEOUT`.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Classify a raw probe `Output` for alias `sentinel` using the same rules as
/// the production `git_supports_subsection_alias` classifier.
fn classify_probe_output(out: &std::process::Output, sentinel: &str) -> ProbeVerdict {
    let unknown_cmd_diag = format!("git: '{sentinel}' is not a git command");
    if out.status.success() && out.stdout.starts_with(b"git version") {
        ProbeVerdict::Supported
    } else if out.status.code() == Some(1)
        && out.stdout.is_empty()
        && out.stderr.starts_with(unknown_cmd_diag.as_bytes())
    {
        ProbeVerdict::Unsupported
    } else {
        ProbeVerdict::Failure
    }
}

/// Isolated capability probe for `alias.<name>.command` form aliases.
///
/// Matches the production `git_supports_subsection_alias` isolation model:
///   - Probe-only private tempdir with a controlled `git` symlink to the real
///     binary (prevents sibling helpers from intercepting the probe)
///   - Both PATH and GIT_EXEC_PATH set to the private dir only
///   - GIT_CONFIG_NOSYSTEM, scratch HOME, XDG_CONFIG_HOME/GIT_DIR removed
///   - LC_ALL=C so the unknown-command diagnostic is ASCII-stable
///   - Bounded execution via production `run_bounded`: timeout + output cap
///     over the whole process group, including pipe draining and teardown
///
/// Callers `panic!` on `Failure` — setup, spawn, timeout, and unclassifiable
/// output indicate a broken test environment, not a capability answer.
fn isolated_subsection_probe() -> ProbeVerdict {
    run_isolated_probe_for(
        &real_git_dir().join("git"),
        "alias._probe_.command=version",
        "_probe_",
    )
}

/// Isolated empty-subsection dispatch probe for `alias..<name>` form aliases.
///
/// Same isolation model as `isolated_subsection_probe`; same tri-state return.
fn isolated_empty_subsection_probe() -> ProbeVerdict {
    run_isolated_probe_for(&real_git_dir().join("git"), "alias..pub=version", "pub")
}

/// Shared implementation for the isolated probes: runs
/// `git -c <config> <sentinel>` against `git_binary`.
fn run_isolated_probe_for(git_binary: &Path, config: &str, sentinel: &str) -> ProbeVerdict {
    let git_binary = git_binary
        .canonicalize()
        .unwrap_or_else(|_| git_binary.to_path_buf());

    // Private probe dir: only the controlled git symlink — no adjacent helpers.
    let probe_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return ProbeVerdict::Failure,
    };
    let git_link = probe_dir.path().join("git");
    if std::os::unix::fs::symlink(&git_binary, &git_link).is_err() {
        return ProbeVerdict::Failure;
    }

    let scratch = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return ProbeVerdict::Failure,
    };
    let probe_path = probe_dir.path().as_os_str().to_owned();
    let out = match buzz_git_identity::git_wrapper::run_bounded(
        Command::new(&git_link)
            .args(["-c", config, sentinel])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", scratch.path())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_SYSTEM")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_NAMESPACE")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env("PATH", &probe_path)
            .env("GIT_EXEC_PATH", probe_dir.path())
            .env("LC_ALL", "C")
            .current_dir(scratch.path()),
        PROBE_TIMEOUT,
    ) {
        Some(o) => o,
        None => return ProbeVerdict::Failure,
    };
    classify_probe_output(&out, sentinel)
}

/// The test probe helpers must report setup/process/classification failure as
/// `Failure` — never as `Unsupported`, which callers treat as a legitimate skip.
#[test]
fn probe_helpers_report_timeout_and_unrecognized_exit1_as_failure() {
    let mut sleeper = Command::new("sh");
    sleeper.args(["-c", "sleep 30"]);
    assert!(
        buzz_git_identity::git_wrapper::run_bounded(
            &mut sleeper,
            std::time::Duration::from_millis(200)
        )
        .is_none(),
        "a probe exceeding its deadline must yield no output"
    );

    let unrelated = Command::new("sh")
        .args(["-c", "echo 'fatal: something else' >&2; exit 1"])
        .output()
        .unwrap();
    assert_eq!(
        classify_probe_output(&unrelated, "_probe_"),
        ProbeVerdict::Failure
    );

    let recognized = Command::new("sh")
        .args([
            "-c",
            "echo \"git: '_probe_' is not a git command. See 'git --help'.\" >&2; exit 1",
        ])
        .output()
        .unwrap();
    assert_eq!(
        classify_probe_output(&recognized, "_probe_"),
        ProbeVerdict::Unsupported
    );

    let missing = Path::new("/nonexistent/buzz-probe-test/git");
    assert_eq!(
        run_isolated_probe_for(missing, "alias._probe_.command=version", "_probe_"),
        ProbeVerdict::Failure
    );
}

/// A git repo with one human-authored commit and human-named local config.
fn human_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let g = |args: &[&str]| {
        let ok = hermetic_command("git")
            .args(args)
            .current_dir(p)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    };
    g(&["init", "-q", "-b", "main"]);
    g(&["config", "user.name", "Human Dev"]);
    g(&["config", "user.email", "human@example.com"]);
    g(&["config", "commit.gpgSign", "false"]);
    std::fs::write(p.join("f"), "one").unwrap();
    g(&["add", "f"]);
    g(&["commit", "-qm", "human commit"]);
    d
}

/// A fresh repo with a staged file but no commit object.
fn unborn_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let g = |args: &[&str]| {
        let ok = hermetic_command("git")
            .args(args)
            .current_dir(p)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    };
    g(&["init", "-q", "-b", "main"]);
    g(&["config", "user.name", "Human Dev"]);
    g(&["config", "user.email", "human@example.com"]);
    g(&["config", "commit.gpgSign", "false"]);
    std::fs::write(p.join("f"), "staged").unwrap();
    g(&["add", "f"]);
    d
}

/// Number of commit objects in `repo`, including unreachable objects.
fn commit_object_count(repo: &Path) -> usize {
    let out = hermetic_command("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "cat-file",
            "--batch-all-objects",
            "--batch-check",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "enumerating git objects failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.split_whitespace().nth(1) == Some("commit"))
        .count()
}

/// Invoke the wrapper (`git` on the shim PATH) with `args`, in `cwd`.
///
/// `NOSTR_PRIVATE_KEY`/`BUZZ_PRIVATE_KEY` are scrubbed so `git-sign-nostr` signs
/// from the manifest's `nostr.keyfile` — the real agent-runtime child has the
/// private key env removed, and leaving the runner's ambient key set would make
/// the signer load the wrong identity (a non-hermetic test). `BUZZ_AUTH_TAG` is
/// scrubbed so the signer skips NIP-OA owner attestation (no relay to verify
/// against offline); signing itself needs no network.
fn wrapper(path: &str, cwd: &Path, args: &[&str]) -> std::process::Output {
    hermetic_command("git")
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("run wrapper git")
}

/// Like `wrapper`, but bounded: the whole process tree is killed after
/// `timeout` and the test panics. Used for nested-agent tests where a recursion
/// regression would otherwise hang CI indefinitely.
#[cfg(unix)]
fn wrapper_bounded(
    path: &str,
    cwd: &Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> std::process::Output {
    let mut cmd = hermetic_command("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG");
    run_tree_bounded(&mut cmd, timeout)
}

/// Run `cmd` via `run_bounded`, which drains pipes and tears down the full
/// process group, so a descendant holding stdout cannot outlive the deadline.
#[cfg(unix)]
fn run_tree_bounded(cmd: &mut Command, timeout: std::time::Duration) -> std::process::Output {
    buzz_git_identity::git_wrapper::run_bounded(cmd, timeout).unwrap_or_else(|| {
        panic!(
            "bounded run of {cmd:?} timed out after {timeout:?} or failed to spawn \
             — likely a recursion or hang regression"
        )
    })
}

/// The bounded helper must not wait on, or leave alive, a backgrounded
/// descendant that keeps the child's stdout open after the child exits.
#[cfg(unix)]
#[test]
fn bounded_helper_kills_descendant_holding_stdout() {
    let timeout = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    let out = run_tree_bounded(
        Command::new("sh").args(["-c", "sleep 30 & echo $!"]),
        timeout,
    );
    assert!(
        start.elapsed() < timeout,
        "helper waited {:?} for a pipe-holding descendant",
        start.elapsed()
    );
    assert!(out.status.success());
    let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!pid.is_empty(), "descendant pid missing from stdout");
    // The orphaned sleeper is reaped asynchronously by init; allow a moment.
    let alive = || {
        Command::new("kill")
            .args(["-0", &pid])
            .status()
            .unwrap()
            .success()
    };
    let reap_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while alive() && std::time::Instant::now() < reap_deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !alive(),
        "descendant sleeper {pid} survived the bounded run"
    );
}

/// Assert `remote` enumerates refs successfully and holds none.
fn assert_remote_empty(remote: &Path) {
    let refs = hermetic_command("git")
        .args(["-C", remote.to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.status.success(),
        "for-each-ref failed: {:?}",
        refs.status
    );
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty; refs={}",
        String::from_utf8_lossy(&refs.stdout)
    );
}

/// The current `HEAD` commit SHA of `repo`, via real git (empty if unborn).
fn head_sha(repo: &Path) -> String {
    let out = hermetic_command("git")
        .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    if !out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn wrapper_rejects_flag_based_identity_override() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    std::fs::write(repo.path().join("f"), "two").unwrap();
    wrapper(&path, repo.path(), &["add", "f"]);

    let out = wrapper(
        &path,
        repo.path(),
        &["-c", "user.email=evil@example.com", "commit", "-m", "x"],
    );
    assert!(
        !out.status.success(),
        "override commit should be rejected; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("machine-managed"),
        "expected the loud enforce message; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn wrapper_refuses_to_push_human_authored_commit() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    // A reachable bare remote so the dry-run plan resolves and HEAD (human
    // authored) is examined as an offender.
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );

    let out = wrapper(&path, repo.path(), &["push", "origin", "main"]);
    assert!(
        !out.status.success(),
        "pushing a human-authored commit must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // The bare remote must have received nothing.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "no ref should have reached the remote: {}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R4 real-wrapper regression for the allowlist alias guard. Two Thufir p3
/// bypass probes must be refused through the actual `buzz-acp`-as-`git`
/// multicall, and neither may create a commit:
///
/// (a) a *quoted* config alias — git's quote-aware parser dequotes `'-c'`
///     `'user.email=…'` into real `-c` config that the whitespace-naive
///     round-3 scan missed;
/// (b) a *shell* (`!`) commit alias — git runs it with real git ahead of the
///     wrapper on PATH, so its inner `-c` re-authors the commit.
///
/// A plain-subcommand alias must still resolve and commit as the agent
/// identity, proving the allowlist did not over-reject Gurney's working shapes.
#[test]
fn wrapper_rejects_quoted_and_shell_aliases_and_allows_plain_alias() {
    let (_shim, path, email, _keydir) = signed_shim_env();
    let repo = human_repo();

    // (a) quoted config alias — the parser-parity bypass.
    wrapper(
        &path,
        repo.path(),
        &[
            "config",
            "alias.quoted",
            "'-c' 'user.name=QuotedHuman' '-c' 'user.email=quoted@human.test' '-c' 'commit.gpgSign=false' commit",
        ],
    );
    // (b) shell commit alias — git prepends real git to PATH for `!` bodies.
    wrapper(
        &path,
        repo.path(),
        &[
            "config",
            "alias.sc",
            "!f(){ git -c user.name=ShellHuman -c user.email=shell@human.test -c commit.gpgSign=false commit \"$@\"; }; f",
        ],
    );
    // A plain-subcommand alias that must keep working.
    wrapper(&path, repo.path(), &["config", "alias.ci", "commit"]);

    let head_before = head_sha(repo.path());
    std::fs::write(repo.path().join("f"), "two").unwrap();
    wrapper(&path, repo.path(), &["add", "f"]);

    // (a) refused, no commit created.
    let out = wrapper(&path, repo.path(), &["quoted", "-m", "via quoted alias"]);
    assert!(
        !out.status.success(),
        "quoted config alias must be refused; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        head_sha(repo.path()),
        head_before,
        "the refused quoted alias must not create a commit"
    );

    // (b) refused, no commit created.
    let out = wrapper(&path, repo.path(), &["sc", "-m", "via shell alias"]);
    assert!(
        !out.status.success(),
        "shell commit alias must be refused; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("shell (`!`) git alias"),
        "expected the shell-alias rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        head_sha(repo.path()),
        head_before,
        "the refused shell alias must not create a commit"
    );

    // The plain alias must still resolve and commit as the agent identity.
    let out = wrapper(&path, repo.path(), &["ci", "-m", "via plain alias"]);
    assert!(
        out.status.success(),
        "plain-subcommand alias must still commit; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let author = hermetic_command("git")
        .args([
            "-C",
            repo.path().to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae",
            "HEAD",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&author.stdout).trim(),
        email,
        "plain-alias commit must be authored as the agent identity"
    );
}

/// R5 real-wrapper regression for the alias-unification fix (Thufir's rd-4
/// IMPORTANT). A bare-word alias whose body carries identity/signing *flags*
/// passes the allowlist (every token is a plain bare word), but after the alias
/// is expanded the wrapper holds the expansion to the SAME `enforce`/author
/// policy as a directly-typed command — so the alias can do no more than its
/// expansion could. Three shapes, each of which is refused when typed directly,
/// must therefore be refused through the alias too, with `HEAD` unchanged:
///
/// (a) Thufir's exact probe — `--author` (split form) plus `--no-gpg-sign`;
/// (b) `--no-gpg-sign` alone, pinning that the fix is not one hard-coded string;
/// (c) an alias *chain* that resolves to `commit --no-gpg-sign` through two
///     hops, pinning that unification applies to the final accumulated command.
#[test]
fn wrapper_rejects_bare_word_alias_carried_identity_and_signing_flags() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();

    // (a) Thufir's exact bypass probe — bare-word `--author`/`--no-gpg-sign`.
    wrapper(
        &path,
        repo.path(),
        &[
            "config",
            "alias.human",
            "commit --author Human<human@human.test> --no-gpg-sign",
        ],
    );
    // (b) `--no-gpg-sign` alone.
    wrapper(
        &path,
        repo.path(),
        &["config", "alias.unsign", "commit --no-gpg-sign"],
    );
    // (c) an alias chain: `chain` → `co --no-gpg-sign` → `commit --no-gpg-sign`.
    wrapper(&path, repo.path(), &["config", "alias.co", "commit"]);
    wrapper(
        &path,
        repo.path(),
        &["config", "alias.chain", "co --no-gpg-sign"],
    );

    let head_before = head_sha(repo.path());
    std::fs::write(repo.path().join("f"), "two").unwrap();
    wrapper(&path, repo.path(), &["add", "f"]);

    for (alias, label) in [
        ("human", "author+no-gpg-sign alias"),
        ("unsign", "no-gpg-sign-only alias"),
        ("chain", "chained no-gpg-sign alias"),
    ] {
        let out = wrapper(&path, repo.path(), &[alias, "-m", "leak"]);
        assert!(
            !out.status.success(),
            "{label} must be refused; stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("machine-managed"),
            "{label} must give the identity/signing rejection; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(
            head_sha(repo.path()),
            head_before,
            "{label} must not create a commit"
        );
    }
}

#[test]
fn wrapper_refuses_alias_chain_beyond_limit_and_allows_exact_limit() {
    let (_shim, path, email, _keydir) = signed_shim_env();
    let repo = unborn_repo();

    // Exactly ALIAS_HOP_LIMIT substitutions end at real `commit`, so the wrapper
    // must preserve the boundary's useful side: it resolves and commits under
    // the managed agent identity.
    for index in 0..ALIAS_HOP_LIMIT {
        let name = format!("at{index}");
        let next = if index + 1 == ALIAS_HOP_LIMIT {
            "commit".to_string()
        } else {
            format!("at{}", index + 1)
        };
        wrapper(
            &path,
            repo.path(),
            &["config", &format!("alias.{name}"), &next],
        );
    }
    let out = wrapper(&path, repo.path(), &["at0", "-m", "at the alias limit"]);
    assert!(
        out.status.success(),
        "chain at the limit must reach the real command; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let author = hermetic_command("git")
        .args([
            "-C",
            repo.path().to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae",
            "HEAD",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&author.stdout).trim(), email);

    // Thufir's limit+1 counterexample: the wrapper must not hand a partial
    // expansion to git. A human-author `commit` beyond the bound is refused
    // before git runs, leaving the fresh repo unborn with no commit objects.
    let beyond = unborn_repo();
    for index in 0..=ALIAS_HOP_LIMIT {
        let name = format!("a{index}");
        let next = if index == ALIAS_HOP_LIMIT {
            "commit --author Human<human@human.test>".to_string()
        } else {
            format!("a{}", index + 1)
        };
        wrapper(
            &path,
            beyond.path(),
            &["config", &format!("alias.{name}"), &next],
        );
    }
    let out = wrapper(&path, beyond.path(), &["a0", "-m", "leak"]);
    assert!(
        !out.status.success(),
        "chain past the limit must be refused; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains(&format!("after {ALIAS_HOP_LIMIT} expansions")),
        "expected the alias-limit refusal; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let beyond_head = head_sha(beyond.path());
    assert!(
        beyond_head.is_empty(),
        "HEAD must remain unborn; HEAD={beyond_head:?}; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(
        commit_object_count(beyond.path()),
        0,
        "the refused chain must not create an unreachable commit object"
    );
}

#[test]
fn wrapper_reapplies_agent_identity_over_repo_config() {
    // The env-var / repo-config override vector: repo-local config names a
    // human, yet the wrapper re-appends the agent identity at the highest
    // GIT_CONFIG_* index, so the resulting commit is agent-authored. Deleting
    // `apply_authority_env` makes this commit land as `human@example.com`.
    let (_shim, path, email, _keydir) = signed_shim_env();
    let repo = human_repo();
    std::fs::write(repo.path().join("f"), "two").unwrap();
    wrapper(&path, repo.path(), &["add", "f"]);

    let out = wrapper(&path, repo.path(), &["commit", "-m", "agent authored"]);
    assert!(
        out.status.success(),
        "ordinary commit should succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    let author = hermetic_command("git")
        .args([
            "-C",
            repo.path().to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae",
            "HEAD",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&author.stdout).trim(),
        email,
        "commit must be authored as the agent identity, not the repo-local human"
    );
}

/// A fixture agent key persisted the way the harness persists it.
struct AgentKey {
    keyfile_path: String,
    pubkey_hex: String,
    npub: String,
}

/// Write `nsec` to an owner-only keyfile in `dir` and derive its identity.
fn write_agent_key(dir: &Path, nsec: &str) -> Option<AgentKey> {
    use std::os::unix::fs::OpenOptionsExt;
    let keys = nostr::Keys::parse(nsec).ok()?;
    let keyfile = dir.join(".nostr-key");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&keyfile)
        .and_then(|mut f| std::io::Write::write_all(&mut f, nsec.as_bytes()))
        .ok()?;
    Some(AgentKey {
        keyfile_path: keyfile.to_str()?.to_owned(),
        pubkey_hex: keys.public_key().to_hex(),
        npub: keys.public_key().to_bech32().ok()?,
    })
}

/// The complete manifest a managed install writes for `id` on `relay.test`.
fn manifest_entries(id: &AgentKey) -> Vec<(String, String)> {
    let mut entries = vec![
        ("user.name".to_owned(), id.npub.clone()),
        (
            "user.email".to_owned(),
            format!("{}@relay.test", id.pubkey_hex),
        ),
    ];
    entries.extend(
        buzz_git_identity::FIXED_SIGNING_ENTRIES
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned())),
    );
    entries.push(("user.signingkey".to_owned(), id.pubkey_hex.clone()));
    entries.push(("nostr.keyfile".to_owned(), id.keyfile_path.clone()));
    entries
}

/// Build a shim dir wired for REAL signing: `git` and `git-sign-nostr` both
/// symlink to the buzz-acp multicall, and the `.git-identity` manifest carries
/// the full identity + signing config (`commit.gpgSign=true`, the signer
/// program, `user.signingkey`, and the keyfile) for a freshly generated key.
/// Returns (shim TempDir, PATH string, expected author email, keyfile-holding
/// TempDir). Signing itself needs no network — `BUZZ_AUTH_TAG` is left unset so
/// the signer works offline.
fn signed_shim_env() -> (tempfile::TempDir, String, String, tempfile::TempDir) {
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();

    // Keyfile + derived identity live in their own 0700 dir (the manifest's
    // `nostr.keyfile` points here). Kept separate from the shim so the shim
    // holds only the git symlinks + manifest, as the harness installs them.
    let keydir = tempfile::tempdir().unwrap();
    let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
    let expected_email = format!("{}@relay.test", id.pubkey_hex);

    let shim = tempfile::tempdir().unwrap();
    for name in ["git", "git-sign-nostr"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name)).unwrap();
    }
    buzz_git_identity::write_identity_manifest(shim.path(), &manifest_entries(&id)).unwrap();

    let real = real_git_dir();
    let path = std::env::join_paths([shim.path().to_path_buf(), real])
        .unwrap()
        .into_string()
        .unwrap();
    (shim, path, expected_email, keydir)
}

/// A repo whose local config names the AGENT identity (author is correct) and a
/// reachable bare remote, ready for one commit. Returns (work TempDir, repo
/// path, remote path). `commit.gpgSign` is left to the wrapper's injected config
/// so the commit shape is set per-test.
fn agent_repo_with_remote(agent_email: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let remote = work.path().join("remote.git");
    let g = |cwd: &Path, args: &[&str]| {
        assert!(hermetic_command("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success());
    };
    g(
        work.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    g(&repo, &["init", "-q", "-b", "main"]);
    g(&repo, &["config", "user.name", "Agent"]);
    g(&repo, &["config", "user.email", agent_email]);
    g(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    (work, repo, remote)
}

/// L3b (real signer): a genuinely signed agent commit pushes cleanly. This
/// proves the push-gate signature check accepts a valid NIP-GS signature by the
/// agent key — the happy path that the reject tests below are measured against.
#[test]
fn wrapper_allows_push_of_signed_agent_commit() {
    let (_shim, path, email, _keydir) = signed_shim_env();
    let (_work, repo, _remote) = agent_repo_with_remote(&email);
    std::fs::write(repo.join("f"), "x").unwrap();
    wrapper(&path, &repo, &["add", "f"]);
    // The wrapper injects commit.gpgSign=true + the signer, so this commit is
    // signed by the agent key.
    let out = wrapper(&path, &repo, &["commit", "-m", "agent signed"]);
    assert!(
        out.status.success(),
        "signed agent commit should succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let out = wrapper(&path, &repo, &["push", "origin", "main"]);
    assert!(
        out.status.success(),
        "pushing a signed agent commit must be allowed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// L3b (real signer): an agent-authored but UNSIGNED commit created via
/// `git merge --no-gpg-sign` — Carl's live repro of the P1 gap — must be refused
/// at push. The merge commit is correctly agent-authored, so only the signature
/// check catches it; `enforce` cannot, because `merge` is not in its signing
/// blocklist.
#[test]
fn wrapper_refuses_push_of_unsigned_merge_commit() {
    let (_shim, path, email, _keydir) = signed_shim_env();
    let (_work, repo, _remote) = agent_repo_with_remote(&email);
    // Base signed commit on main.
    std::fs::write(repo.join("base"), "b").unwrap();
    wrapper(&path, &repo, &["add", "base"]);
    assert!(wrapper(&path, &repo, &["commit", "-m", "base"])
        .status
        .success());
    // A signed commit on a side branch.
    wrapper(&path, &repo, &["checkout", "-q", "-b", "side"]);
    std::fs::write(repo.join("side"), "s").unwrap();
    wrapper(&path, &repo, &["add", "side"]);
    assert!(wrapper(&path, &repo, &["commit", "-m", "side work"])
        .status
        .success());
    // Back on main, merge the side branch WITHOUT signing — an agent-authored
    // but unsigned merge commit. `--no-ff` forces a merge commit object.
    wrapper(&path, &repo, &["checkout", "-q", "main"]);
    let m = wrapper(
        &path,
        &repo,
        &[
            "merge",
            "--no-ff",
            "--no-gpg-sign",
            "-m",
            "merge side",
            "side",
        ],
    );
    assert!(
        m.status.success(),
        "the unsigned merge itself should succeed; stderr={}",
        String::from_utf8_lossy(&m.stderr),
    );
    let out = wrapper(&path, &repo, &["push", "origin", "main"]);
    assert!(
        !out.status.success(),
        "pushing an unsigned merge commit must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no valid signature by your agent key"),
        "expected the unsigned-commit rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// L3b (real signer): an agent-authored but UNSIGNED commit created via the
/// `commit-tree` plumbing — which bypasses `commit` entirely and so is never
/// touched by `enforce` — must be refused at push. Pins that the push-gate
/// check covers the plumbing path, not just porcelain.
#[test]
fn wrapper_refuses_push_of_unsigned_commit_tree() {
    let (_shim, path, email, _keydir) = signed_shim_env();
    let (_work, repo, _remote) = agent_repo_with_remote(&email);
    // Seed a signed base so HEAD and the tree exist.
    std::fs::write(repo.join("f"), "x").unwrap();
    wrapper(&path, &repo, &["add", "f"]);
    assert!(wrapper(&path, &repo, &["commit", "-m", "base"])
        .status
        .success());
    // Build an unsigned commit object directly with `commit-tree` (no signing,
    // agent identity via env), then move the branch to it.
    let tree = wrapper(&path, &repo, &["write-tree"]);
    let tree_sha = String::from_utf8_lossy(&tree.stdout).trim().to_string();
    let parent = head_sha(&repo);
    let out = hermetic_command("git")
        .args(["commit-tree", &tree_sha, "-p", &parent, "-m", "plumbed"])
        .current_dir(&repo)
        .env("PATH", &path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Agent")
        .env("GIT_AUTHOR_EMAIL", &email)
        .env("GIT_COMMITTER_NAME", "Agent")
        .env("GIT_COMMITTER_EMAIL", &email)
        .output()
        .expect("run commit-tree");
    assert!(
        out.status.success(),
        "commit-tree should succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let new_sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    wrapper(&path, &repo, &["update-ref", "refs/heads/main", &new_sha]);

    let out = wrapper(&path, &repo, &["push", "origin", "main"]);
    assert!(
        !out.status.success(),
        "pushing an unsigned commit-tree commit must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no valid signature by your agent key"),
        "expected the unsigned-commit rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Contract regression (Thufir rd-2 IMPORTANT): a `.git-identity` manifest that
/// names the agent but drops, falsifies, or misdirects the signing contract is
/// tampered — not a legitimate unsigned mode — and must fail closed for EVERY
/// command, not silently disable or redirect the push-gate signature check.
/// Six mutations, each a distinct silent-disable/misdirect the contract check
/// rejects at classification time (`run()` refuses before any dispatch), so
/// even a read-only `status` is refused. The duplicate-key and `include.path`
/// cases cover the last-value-wins redirect that a first-value-only validator
/// would miss.
#[test]
fn wrapper_refuses_every_command_when_manifest_signing_contract_is_tampered() {
    let variants: [ManifestMutation; 6] = [
        // `commit.gpgSign` removed → the signature gate would never fire.
        ("commit.gpgSign removed", |m| {
            m.lines()
                .filter(|l| !l.starts_with("commit.gpgSign="))
                .collect::<Vec<_>>()
                .join("\n")
        }),
        // `commit.gpgSign=false` → the same silent-disable, spelled out.
        ("commit.gpgSign=false", |m| {
            m.replace("commit.gpgSign=true", "commit.gpgSign=false")
        }),
        // `user.signingkey` swapped to a key the author email does not encode →
        // the probe would trust the wrong key.
        ("user.signingkey swapped", |m| {
            m.lines()
                .map(|l| {
                    if l.starts_with("user.signingkey=") {
                        format!("user.signingkey={}", "b".repeat(64))
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }),
        // A DUPLICATE later `user.signingkey=B` appended after the canonical
        // first value → git config is last-value-wins, so the probe would
        // resolve B. classify() rejects any duplicate canonical key.
        ("duplicate user.signingkey", |m| {
            format!("{m}\nuser.signingkey={}", "b".repeat(64))
        }),
        // An appended `include.path` → an unknown key that could pull in another
        // key file whose later `user.signingkey` wins. classify() rejects any
        // unknown key.
        ("appended include.path", |m| {
            format!("{m}\ninclude.path=/tmp/evil.inc")
        }),
        // `gpg.x509.program` differing ONLY in case. On a case-sensitive host
        // `GIT-SIGN-NOSTR` resolves past the managed install to an agent-
        // controlled fake verifier later on PATH; the probe would then accept
        // an unsigned commit. classify() compares fixed values byte for byte.
        ("gpg.x509.program cased", |m| {
            m.replace(
                "gpg.x509.program=git-sign-nostr",
                "gpg.x509.program=GIT-SIGN-NOSTR",
            )
        }),
    ];

    for (label, mutate) in variants {
        let (shim, path, _email, _keydir) = signed_shim_env();
        let manifest_path = shim.path().join(".git-identity");
        let original = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(&manifest_path, mutate(&original)).unwrap();

        let repo = human_repo();
        let out = wrapper(&path, repo.path(), &["status"]);
        assert!(
            !out.status.success(),
            "[{label}] a tampered manifest must refuse every command; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("complete signing contract"),
            "[{label}] expected the tampered-manifest refusal; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// L3b (real signer): the wrong-key case Thufir called out — a commit correctly
/// authored as the agent (key A) but VALIDLY signed by a DIFFERENT key B must be
/// refused at push. `git-sign-nostr` verifies B's signature as cryptographically
/// good, but the push probe injects the authority's `user.signingkey=A`, so the
/// verified key ≠ the expected key → not `TRUST_FULLY` → `%G?` ≠ `G`. A valid
/// signature by the wrong key is not a valid agent signature.
#[test]
fn wrapper_refuses_push_of_commit_validly_signed_by_wrong_key() {
    let (_shim, path, email_a, _keydir_a) = signed_shim_env();
    let (_work, repo, _remote) = agent_repo_with_remote(&email_a);

    // A second, unrelated signing identity (key B) with its own keyfile.
    let keys_b = nostr::Keys::generate();
    let nsec_b = keys_b.secret_key().to_bech32().unwrap();
    let keydir_b = tempfile::tempdir().unwrap();
    let id_b = write_agent_key(keydir_b.path(), &nsec_b).expect("write B keyfile");

    // Create a commit authored as agent A but signed with key B, bypassing the
    // wrapper's `enforce` by invoking the real git binary directly with B's
    // signing config. `git-sign-nostr` resolves from the shim on PATH.
    let real_git = real_git_dir().join("git");
    let out = hermetic_command(&real_git)
        .args([
            "-C",
            repo.to_str().unwrap(),
            "-c",
            "gpg.format=x509",
            "-c",
            "gpg.x509.program=git-sign-nostr",
            "-c",
            "commit.gpgSign=true",
            "-c",
            &format!("user.signingkey={}", id_b.pubkey_hex),
            "-c",
            &format!("nostr.keyfile={}", id_b.keyfile_path),
            "commit",
            "--allow-empty",
            "-m",
            "authored by A, signed by B",
        ])
        .current_dir(&repo)
        .env("PATH", &path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Agent")
        .env("GIT_AUTHOR_EMAIL", &email_a)
        .env("GIT_COMMITTER_NAME", "Agent")
        .env("GIT_COMMITTER_EMAIL", &email_a)
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("create B-signed commit");
    assert!(
        out.status.success(),
        "the B-signed commit itself should be created; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Sanity: it is agent-authored, so the push gate demands a valid agent
    // signature on it (rather than skipping it as someone else's commit).
    let author = hermetic_command(&real_git)
        .args([
            "-C",
            repo.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae",
            "HEAD",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&author.stdout).trim(), email_a);

    let out = wrapper(&path, &repo, &["push", "origin", "main"]);
    assert!(
        !out.status.success(),
        "a commit signed by the wrong key must be refused at push; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no valid signature by your agent key"),
        "expected the wrong-key rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Run the real `buzz-acp` harness with a script adapter and `BUZZ_GIT_IDENTITY`
/// set to `mode` (unset for `None`). The adapter runs `probe` in `work`, writes
/// `done`, and idles until the harness is terminated. Returns the harness exit
/// status and its log. Isolated from operator global/system Git config, so any
/// identity the probe sees came from the harness or the fixture's repo config.
fn run_harness(
    work: &Path,
    mode: Option<&str>,
    probe: &str,
    tmpdir: Option<&Path>,
    inherited: &[(&str, &str)],
    path_prefix: Option<&Path>,
) -> (std::process::ExitStatus, String) {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let adapter = work.join("adapter.sh");
    std::fs::write(
        &adapter,
        format!(
            "#!/bin/sh\nset -eu\ncd \"$PROBE_DIR\"\n{probe}\nprintf done > done\nexec sleep 60\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
    let log = std::fs::File::create(work.join("harness.log")).unwrap();
    let mut cmd = hermetic_command(env!("CARGO_BIN_EXE_buzz-acp"));
    let path = std::env::join_paths(path_prefix.map(Path::to_path_buf).into_iter().chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    cmd.env_clear()
        .env("PATH", path)
        .env("PROBE_DIR", work)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "--private-key",
            &nostr::Keys::generate().secret_key().to_secret_hex(),
            "--relay-url",
            "wss://relay.test",
            "--agent-command",
            adapter.to_str().unwrap(),
            "--agent-args",
            "",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    if let Some(mode) = mode {
        cmd.env("BUZZ_GIT_IDENTITY", mode);
    }
    if let Some(tmpdir) = tmpdir {
        cmd.env("TMPDIR", tmpdir);
    }
    if !inherited.is_empty() {
        cmd.env("GIT_CONFIG_COUNT", inherited.len().to_string());
        for (i, (key, value)) in inherited.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), key)
                .env(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
    }
    let mut child = cmd.spawn().unwrap();
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !work.join("done").exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Always terminate the exact child so a failure leaves no harness behind.
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).ok();
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && Instant::now() < exit_deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let status = child.wait().unwrap();
    (
        status,
        std::fs::read_to_string(work.join("harness.log")).unwrap(),
    )
}

/// I4: the harness's `GitEnvironment` must put the enforcement wrapper ahead of
/// real git in the adapter's native shell. The script configures a human
/// identity in the repo; a bare commit must still land as the agent, and a
/// `-c user.email=` override must be refused. Removing the wrapper symlink from
/// the install turns this RED on the refusal only: the `GIT_CONFIG_*` block
/// still attributes the bare commit to the agent, but the override succeeds.
#[test]
fn harness_native_shell_commits_as_agent_and_refuses_identity_override() {
    let work = tempfile::tempdir().unwrap();
    let (status, logs) = run_harness(
        work.path(),
        None,
        r#"git init -q -b main repo
cd repo
git config user.name 'Human Dev'
git config user.email human@example.com
git commit -q --allow-empty -m 'agent authored'
git show -s --format=%ae HEAD > ../author
git verify-commit HEAD
if git -c user.email=human@example.com commit -q --allow-empty -m override 2> ../override-stderr; then
  echo accepted > ../override
else
  echo refused > ../override
fi
cd .."#,
        None,
        &[],
        None,
    );
    assert!(work.path().join("done").exists(), "probe failed: {logs}");
    assert!(status.success(), "harness shutdown failed: {logs}");
    let author = std::fs::read_to_string(work.path().join("author")).unwrap();
    assert!(
        author.trim().ends_with("@relay.test") && author.trim().len() == 64 + "@relay.test".len(),
        "bare commit must be authored by the agent key, got {author:?}"
    );
    assert_eq!(
        std::fs::read_to_string(work.path().join("override"))
            .unwrap()
            .trim(),
        "refused",
        "stderr: {}",
        std::fs::read_to_string(work.path().join("override-stderr")).unwrap_or_default()
    );
}

/// `user` mode installs only relay credentials: the nostr helper answers for
/// the relay, but there is no wrapper, no signer, no manifest and no injected
/// identity, so git resolves the operator's own configuration.
#[test]
fn harness_user_mode_installs_only_relay_credentials() {
    let work = tempfile::tempdir().unwrap();
    let (status, logs) = run_harness(
        work.path(),
        Some("user"),
        r#"absent() { if git config "$1"; then echo "$1 is set" >&2; exit 1; fi; }
helper=$(command -v git-credential-nostr)
dir=$(dirname "$helper")
test "$(git config --get-urlmatch credential.helper https://relay.test/git/o/r)" = nostr
test -n "$(git config nostr.keyfile)"
test ! -e "$dir/git"
test ! -e "$dir/git-sign-nostr"
test ! -e "$dir/.git-identity"
test "$(dirname "$(command -v git)")" != "$dir"
absent user.name
absent user.email
absent user.signingkey
absent commit.gpgSign"#,
        None,
        &[],
        None,
    );
    assert!(work.path().join("done").exists(), "probe failed: {logs}");
    assert!(status.success(), "harness shutdown failed: {logs}");
}

/// `user` mode drops inherited `GIT_CONFIG_*` identity, `author.*` /
/// `committer.*` and signing keys and config includes (which could carry
/// identity), in any case spelling, so the operator's own configuration
/// applies; unrelated inherited entries still reach the adapter.
#[test]
fn harness_user_mode_drops_inherited_identity_and_signing() {
    let work = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "Operator"],
        &["config", "user.email", "operator@example.invalid"],
    ] {
        let status = hermetic_command("git")
            .arg("-C")
            .arg(work.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    let include_file = work.path().join("identity.inc");
    std::fs::write(
        &include_file,
        "[user]\n\temail = included@example.invalid\n",
    )
    .unwrap();
    let include = include_file.to_str().unwrap().to_owned();
    // A parent harness's install dir ahead of real git on the inherited PATH.
    let parent = work.path().join("parent-install");
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join(".git-identity"), "").unwrap();
    std::fs::write(parent.join("git"), "#!/bin/sh\necho parent-wrapper\n").unwrap();
    std::fs::set_permissions(
        parent.join("git"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let (status, logs) = run_harness(
        work.path(),
        Some("user"),
        r#"case "$(command -v git)" in */parent-install/*) echo "git resolves to the parent wrapper" >&2; exit 1;; esac
absent() { if git config "$1"; then echo "$1 is set" >&2; exit 1; fi; }
operator() { case "$(git var "$1")" in "Operator <operator@example.invalid> "*) ;; *) echo "$1 is not the operator" >&2; exit 1;; esac; }
operator GIT_AUTHOR_IDENT
operator GIT_COMMITTER_IDENT
absent user.signingkey
absent gpg.format
absent gpg.x509.program
absent commit.gpgSign
absent tag.gpgSign
test "$(git config core.abbrev)" = 12
test "$(git config gpg.X509.program)" = distinct-subsection"#,
        None,
        &[
            ("user.name", "Inherited Agent"),
            ("USER.EMAIL", "inherited@example.invalid"),
            ("user.signingKey", "inherited-key"),
            ("GPG.Format", "openpgp"),
            ("GPG.x509.PROGRAM", "inherited-signer"),
            ("gpg.X509.program", "distinct-subsection"),
            ("commit.gpgsign", "true"),
            ("TAG.GPGSIGN", "true"),
            ("Author.Name", "Inherited Author"),
            ("Author.Email", "author@example.invalid"),
            ("COMMITTER.name", "Inherited Committer"),
            ("committer.EMAIL", "committer@example.invalid"),
            ("Include.Path", &include),
            ("INCLUDEIF.gitdir:/.PATH", &include),
            ("core.abbrev", "12"),
        ],
        Some(&parent),
    );
    assert!(work.path().join("done").exists(), "probe failed: {logs}");
    assert!(status.success(), "harness shutdown failed: {logs}");
}

/// An unrecognized mode stops the harness before any adapter runs.
#[test]
fn harness_invalid_mode_fails_startup() {
    let work = tempfile::tempdir().unwrap();
    let (status, logs) = run_harness(work.path(), Some("usr"), "true", None, &[], None);
    assert!(!status.success(), "invalid mode must fail startup: {logs}");
    assert!(!work.path().join("done").exists(), "adapter ran: {logs}");
    assert!(
        logs.contains("BUZZ_GIT_IDENTITY"),
        "error must name the var: {logs}"
    );
}

/// A configured key whose Git install fails stops the harness; the adapter
/// never runs with the ambient identity.
#[test]
fn harness_git_install_failure_fails_startup() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("missing-tmp");
    let (status, logs) = run_harness(work.path(), None, "true", Some(&missing), &[], None);
    assert!(
        !status.success(),
        "install failure must fail startup: {logs}"
    );
    assert!(!work.path().join("done").exists(), "adapter ran: {logs}");
}

/// Wes (5055999359) P1 — real-wrapper regression: `--receive-pack` (custom
/// receivepack program) is refused through the ACTUAL `buzz-acp`-as-`git`
/// multicall. Also covers `remote.origin.receivepack` config spelling.
///
/// Bypass shape (what happens WITHOUT the guard):
///
///   `origin` URL → `decoy` (bare repo seeded with the offending human HEAD).
///   receivepack script → ignores its `<url>` argument; exec's
///     `git-receive-pack <actual>` instead.
///
///   1. `resolve_push_sources` runs `git push --dry-run --porcelain`.
///      Git invokes the script as the receive-pack process; negotiation
///      happens against `actual` (empty).  The porcelain `To` header shows
///      the `origin` URL (`decoy`).
///   2. `remote_object_ids` calls `git ls-remote decoy` → returns HEAD's IDs
///      → those IDs are used as the exclusion set → HEAD is treated as
///      already-remote → `partition_outgoing` finds zero offenders.
///   3. The real push runs; git calls the script again → data flows to
///      `actual` → HEAD populates `actual`.
///
/// With guard: `reject_receive_pack_override` fires before any dry-run →
///   `actual` stays empty.
/// Without guard: decoy supplies exemption; push lands in `actual`.
///
/// Mutation-sensitive: removing `reject_receive_pack_override` from
/// `verify_push` makes the bypass succeed; `actual/refs/heads/main` appears
/// and the `show-ref` assertion fires.
#[test]
fn wrapper_refuses_receive_pack_flag_and_leaves_target_empty() {
    use std::os::unix::fs::PermissionsExt;

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    // `decoy` already holds the offending HEAD — ls-remote here supplies the
    // exclusion set that exempts HEAD from outgoing verification.
    let decoy = tempfile::tempdir().unwrap();
    // `actual` starts empty — this is where data lands under the bypass.
    let actual = tempfile::tempdir().unwrap();

    // Init both bare repos.
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", decoy.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", actual.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    // Seed decoy with the human HEAD commit.
    // ls-remote on decoy returns HEAD's IDs; without the guard those IDs
    // exempt HEAD and the push succeeds to actual.
    assert!(hermetic_command("git")
        .args([
            "-C",
            repo.path().to_str().unwrap(),
            "push",
            "-q",
            decoy.path().to_str().unwrap(),
            "HEAD:refs/heads/main",
        ])
        .status()
        .unwrap()
        .success());

    // Wire origin → decoy.  The porcelain To header will show the decoy URL;
    // ls-remote on that URL returns HEAD's IDs (the bypass exclusion source).
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", decoy.path().to_str().unwrap()],
    );

    // The receivepack script ignores its <url> argument and routes all
    // receive-pack traffic to `actual` instead.  Without the guard, git's
    // dry-run + real push both talk to actual via this script while ls-remote
    // exempts HEAD by reading decoy.
    let actual_path = actual.path().to_str().unwrap().to_owned();
    let script = repo.path().join("rp.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nexec git-receive-pack {actual_path}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    // ── Test 1a: --receive-pack=<script> flag form ────────────────────────────
    let rp_arg = format!("--receive-pack={}", script.display());
    let out = wrapper(&path, repo.path(), &["push", &rp_arg, "origin", "main"]);

    assert!(
        !out.status.success(),
        "push with --receive-pack must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("receive-pack"),
        "expected the receive-pack managed-mode refusal; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    // `actual` must be empty — the push was refused before any transport.
    let refs = hermetic_command("git")
        .args([
            "-C",
            actual.path().to_str().unwrap(),
            "show-ref",
            "--verify",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert!(
        !refs.status.success(),
        "actual must be empty after refused push; show-ref found: {}",
        String::from_utf8_lossy(&refs.stdout),
    );

    // ── Test 1b: remote.origin.receivepack config spelling ───────────────────
    // Set the config key (Wes's explicit reproduction path); origin URL stays
    // pointing at decoy so ls-remote still supplies the exemption set.
    wrapper(
        &path,
        repo.path(),
        &[
            "config",
            "remote.origin.receivepack",
            script.to_str().unwrap(),
        ],
    );

    let out2 = wrapper(&path, repo.path(), &["push", "origin", "main"]);
    assert!(
        !out2.status.success(),
        "push with remote.origin.receivepack config must be refused; stderr={}",
        String::from_utf8_lossy(&out2.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out2.stderr).contains("receive-pack")
            || String::from_utf8_lossy(&out2.stderr).contains("receivepack"),
        "expected the receivepack-config managed-mode refusal; stderr={}",
        String::from_utf8_lossy(&out2.stderr),
    );
    let refs2 = hermetic_command("git")
        .args([
            "-C",
            actual.path().to_str().unwrap(),
            "show-ref",
            "--verify",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert!(
        !refs2.status.success(),
        "actual must still be empty after config-based receivepack refusal; found: {}",
        String::from_utf8_lossy(&refs2.stdout),
    );
}

/// Wes (5055999359) P1 — real-wrapper regression: an alias that carries
/// `--rece <script>` (abbreviated `--receive-pack`, separate-value form) is
/// refused through the actual `buzz-acp`-as-`git` multicall.
///
/// Bypass shape (same orientation as the flag test above):
///
///   `origin` URL → `decoy` (seeded with offending HEAD — supplies exclusion set).
///   receivepack script → ignores its `<url>` argument; exec's
///     `git-receive-pack <actual>` instead.
///   alias body: `push --rece <script_path> origin main`
///
///   Without the guard: alias expands to `push --rece <script> origin main`;
///   git interprets `--rece` as `--receive-pack=<script>`; script routes all
///   receive-pack traffic to `actual`; ls-remote on decoy exempts HEAD →
///   `actual` receives the push.
///
/// With guard: `is_receive_pack_or_exec_flag("--rece")` fires before any push
///   → `actual` stays empty.
///
/// Mutation-sensitive: removing the `--rece` prefix check from
/// `is_receive_pack_or_exec_flag` makes the bypass succeed; `actual` becomes
/// populated and the `show-ref` assertion fires.
///
/// Note: `is_safe_alias_token("--rece")` returns `true` (starts with `-` but
/// contains no `=` and no quote), so the token passes the alias-safety filter
/// and reaches the receive-pack guard.  An unrelated alias rejection would not
/// produce a "receive-pack" stderr message; the assertion below distinguishes
/// the two.
#[test]
fn wrapper_refuses_alias_with_abbreviated_receive_pack_flag() {
    use std::os::unix::fs::PermissionsExt;

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    // decoy: seeded with HEAD; ls-remote here is the exemption source.
    let decoy = tempfile::tempdir().unwrap();
    // actual: starts empty; populated by the bypass if the guard is absent.
    let actual = tempfile::tempdir().unwrap();

    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", decoy.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", actual.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    // Seed decoy with HEAD so its IDs would exempt the commit under bypass.
    assert!(hermetic_command("git")
        .args([
            "-C",
            repo.path().to_str().unwrap(),
            "push",
            "-q",
            decoy.path().to_str().unwrap(),
            "HEAD:refs/heads/main",
        ])
        .status()
        .unwrap()
        .success());

    // Wire origin → decoy (the exclusion-set source, not the write target).
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", decoy.path().to_str().unwrap()],
    );

    // The receivepack script ignores its <url> argument and routes all
    // receive-pack traffic to `actual`.  No spaces in the path (tempdir on
    // macOS/Linux is under /private/var/folders/... or /tmp — no spaces).
    let actual_path = actual.path().to_str().unwrap().to_owned();
    let script = repo.path().join("rp.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nexec git-receive-pack {actual_path}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Configure the alias using the separate-value abbreviated form `--rece`.
    // `is_safe_alias_token("--rece")` passes (starts with `-` but no `=`, no
    // quote), so the alias expander emits it into effective_argv, where
    // `is_receive_pack_or_exec_flag` must catch it.
    wrapper(
        &path,
        repo.path(),
        &[
            "config",
            "alias.p",
            &format!("push --rece {} origin main", script.display()),
        ],
    );

    let out = wrapper(&path, repo.path(), &["p"]);

    assert!(
        !out.status.success(),
        "alias-carried --rece must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Must be the receive-pack managed-mode refusal specifically, not an
    // unrelated alias rejection (which would not mention "receive-pack").
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("receive-pack"),
        "expected the receive-pack managed-mode refusal (not an unrelated alias \
         rejection); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    // actual must be empty — the push was refused before any data was sent.
    let refs = hermetic_command("git")
        .args([
            "-C",
            actual.path().to_str().unwrap(),
            "show-ref",
            "--verify",
            "refs/heads/main",
        ])
        .output()
        .unwrap();
    assert!(
        !refs.status.success(),
        "actual must be empty after refused push; show-ref found: {}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R9 real-wrapper regression: `alias.push = status` (a builtin-shadowing alias
/// git silently ignores) must NOT suppress push verification.
///
/// **Bypass shape (without the fix):**
/// `verify_alias_safety` previously performed an unconditional alias lookup on
/// every command word, including builtins.  With `alias.push=status` set it
/// expanded `push` → `status` and returned `effective_argv` with subcommand
/// `status` → `is_push_command` returned `NotPush` → `verify_push` was skipped
/// → `exec_real_git` ran the original `push` argv → git ignored the alias (it
/// is a builtin) → real push succeeded with the human-authored HEAD.
///
/// **Mutation evidence:**
/// Removing the `!builtins.is_empty() && builtins.contains(name.as_str())` break
/// from `verify_alias_safety` recreates the bypass: the alias expands, the
/// subcommand reads `status`, verification is skipped, and the remote receives
/// the commit.  The `for-each-ref` assertion below would then fire.
#[test]
fn wrapper_refuses_push_via_builtin_shadowing_alias() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Set alias.push=status — a builtin-shadowing alias git silently ignores.
    // The wrapper must honour git's builtin-first precedence and refuse the
    // human-authored commit exactly as it would without this alias.
    wrapper(&path, repo.path(), &["config", "alias.push", "status"]);

    let out = wrapper(&path, repo.path(), &["push", "origin", "main"]);
    assert!(
        !out.status.success(),
        "push with alias.push=status must be refused (builtin-shadowing alias must not \
         suppress verification); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection (not an alias safety error or status output); \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Destination must be empty — the push was refused before anything was sent.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R10/P1-a real-wrapper regression: `git pub` with `alias.pub.command = push`
/// (the Git 2.54 subsection form, no plain `alias.pub`) must be expanded and
/// refused — a human-authored commit must not reach the remote.
///
/// **Bypass shape (without the fix):**
/// The wrapper probed only `config --get alias.pub`, which exits non-zero for a
/// `.command`-only alias.  Both `verify_alias_safety` and `is_push_command`
/// returned early as if `pub` were a real subcommand; `enforce` saw no alias to
/// expand, skipped the push gate, and `exec_real_git` ran the original argv.
/// Git then expanded `alias.pub.command=push` internally and published the
/// human-authored commit.
///
/// **Fix:** `git_supports_subsection_alias` probes actual dispatch (not config
/// storage); `resolve_alias` enumerates all matching entries in traversal order
/// via `config --get-regexp`; the last definition wins.
///
/// **Self-gate:** skips on binaries where the dispatch probe returns `Unsupported` —
/// those binaries don't execute `.command` aliases.
#[test]
fn wrapper_refuses_push_via_subsection_command_alias() {
    // Dispatch probe: does this binary execute alias._probe_.command=version?
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Set ONLY the .command form (no plain alias.pub).
    wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub.command=push must be expanded and refused (human-authored HEAD); \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Destination must be empty.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via .command alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R10/P1-b real-wrapper regression: when both plain `alias.pub = status` and
/// subsection `alias.pub.command = push` are present (subsection last), the
/// last definition wins and the push must be refused.
///
/// **Bypass shape (without the fix):**
/// The wrapper only queried `config --get alias.pub` (the plain form), saw
/// `status`, followed it to `NotPush`, and skipped push verification.  Real git
/// 2.54 resolved in traversal order; with `.command` last, it dispatched `push`
/// and published the commit.
///
/// **Fix:** `resolve_alias` enumerates all matching forms in traversal order
/// via `config --get-regexp`; the last entry's value is the effective alias.
///
/// **Self-gate:** skips on binaries without subsection alias dispatch support.
#[test]
fn wrapper_refuses_push_subsection_command_overrides_plain_alias() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Plain form would redirect to status (NotPush); command form correctly
    // points to push.  git 2.54 resolves .command first — the wrapper must too.
    wrapper(&path, repo.path(), &["config", "alias.pub", "status"]);
    wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub.command=push must override alias.pub=status and be refused; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection (not a status execution); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via .command-override alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R10/P2 real-wrapper positive control: a genuine `git status` with
/// `alias.status = push` must NOT be blocked — the builtin-shadowing alias
/// must not cause `verify_push` to run a push probe on a status invocation.
///
/// **Defect shape (without the fix):**
/// `is_push_command` did an unconditional alias walk.  With `alias.status=push`
/// set it returned `Push`, triggering `verify_push` which probed the original
/// `status` argv with `--dry-run --porcelain --no-verify`.  Git status rejects
/// push-only flags → exit non-zero → legitimate `status` was blocked.
///
/// **Fix:** `is_push_command` now short-circuits at non-deprecated builtins
/// before consulting alias config.
///
/// **Mutation evidence:** removing the `is_nondeprecated_builtin` early-return
/// from `is_push_command` makes this test FAIL (status is incorrectly blocked).
#[test]
fn wrapper_allows_status_with_builtin_shadowing_push_alias() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    // Set alias.status=push — a builtin-shadowing alias git silently ignores.
    wrapper(&path, repo.path(), &["config", "alias.status", "push"]);

    let out = wrapper(&path, repo.path(), &["status", "--short"]);
    assert!(
        out.status.success(),
        "git status with a builtin-shadowing alias.status=push must succeed; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Confirm it actually produced status output (not an error or push output).
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "status must not have hit the push gate; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// R9/P5 real-wrapper regression: `alias.whatchanged = -p push origin main`
/// must be expanded and held to policy on binaries where `--list-cmds=deprecated`
/// succeeds and lists `whatchanged` (i.e. deprecated builtins are alias-first).
/// at `whatchanged` (it is in `--list-cmds=builtins`) and returns `Ok(None)`.
/// The fallback `is_push_command` then reads `alias.whatchanged = -p push` and
/// takes `-p` as the first command word (no `alias.-p` → `NotPush`).  Real git,
/// however, reparses `-p` as a global option and executes `push` — the command
/// executes unverified.  The destination acquires commits and the
/// `for-each-ref` assertion below fires.
///
/// **Why `-p push` and not plain `push`:**
/// With a plain `alias.whatchanged = push` body the fallback `is_push_command`
/// (git_wrapper.rs:318-332) also follows the alias and returns `Push` on its own,
/// so `verify_push` still runs and the test stays green even without the
/// deprecated-aware fix.  The `-p push` body breaks that secondary path while
/// remaining transparent to real git, making this test genuinely sensitive to
/// the deprecated-exclusion fix.
///
/// **Fix:** `git_deprecated_commands(real_git)` queries `--list-cmds=deprecated`
/// for the same binary.  The short-circuit now requires `builtin AND NOT
/// deprecated`: when `whatchanged` is in both sets, alias expansion continues,
/// resolves to `push`, and policy refuses the human-authored commit.
///
/// Self-gate: skips cleanly on binaries where `--list-cmds=deprecated` exits
/// non-zero or does not list `whatchanged` — those binaries do not have the
/// alias-first deprecated dispatch, so there is no bypass to test.
#[test]
fn wrapper_refuses_push_via_deprecated_builtin_alias() {
    // Probe whether the PATH git supports --list-cmds=deprecated and actually
    // lists whatchanged.  Skip on binaries where the deprecated-builtin
    // alias-first path does not exist.
    let probe = hermetic_command("git")
        .args(["--list-cmds=deprecated"])
        .output()
        .unwrap();
    if !probe.status.success() {
        eprintln!("skip: git --list-cmds=deprecated unsupported on this binary");
        return;
    }
    let deprecated_list = String::from_utf8_lossy(&probe.stdout);
    if !deprecated_list.lines().any(|l| l.trim() == "whatchanged") {
        eprintln!("skip: `whatchanged` not in --list-cmds=deprecated output on this binary");
        return;
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Set alias.whatchanged = -p push.
    // `-p` is a safe bare-word token (no `-c`, no `=`, no quote) that real git
    // reparses as the --paginate global, so `git whatchanged -p push origin
    // main` is equivalent to `git -p push origin main` at exec time.
    // With the deprecated-aware fix, `verify_alias_safety` follows the alias
    // (whatchanged is deprecated → alias-first), expands to effective argv with
    // subcommand `push`, and policy refuses.
    // Without the fix, the builtin short-circuit fires at `whatchanged`,
    // verify_alias_safety returns Ok(None), and the fallback is_push_command
    // reads alias.whatchanged = -p push but takes `-p` as the command word
    // (no alias.-p exists → NotPush) — verification is skipped, real git
    // reparses -p as a global and executes the push unverified.
    wrapper(
        &path,
        repo.path(),
        &["config", "alias.whatchanged", "-p push"],
    );

    // Invoke via the deprecated builtin name `whatchanged`.
    // Pass `origin main` as trailing argv so verify_push / resolve_push_sources
    // can identify the destination.
    let out = wrapper(&path, repo.path(), &["whatchanged", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.whatchanged=-p push must be expanded and refused (deprecated builtin is \
         alias-first on this binary); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection via whatchanged alias; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Destination must be empty — no commit should have reached the remote.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via whatchanged alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Thufir R11 reverse-order: when plain `alias.pub = push` is defined **after**
/// `alias.pub.command = status` (so plain is the last definition), the last
/// definition wins and the push must be refused.
///
/// This confirms the wrapper implements true last-wins ordering rather than
/// hard-coding `.command`-first precedence.  An old `.command`-first fix would
/// see `status` (the `.command` value) and classify the command as NotPush,
/// letting a real push through unverified.
///
/// **Self-gate:** skips on binaries without subsection alias dispatch support.
#[test]
fn wrapper_refuses_push_plain_last_overrides_subsection_command() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // command=status first, plain=push last — last definition wins → push.
    // A .command-first resolver would see status and wrongly classify as NotPush.
    wrapper(
        &path,
        repo.path(),
        &["config", "alias.pub.command", "status"],
    );
    wrapper(&path, repo.path(), &["config", "alias.pub", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub=push (last) must override alias.pub.command=status (first) and be refused; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection (not status output); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Mutation evidence: deleting the last-wins branch from `resolve_alias`
    // and reverting to `.command`-first makes this test PASS (push reaches the
    // remote) instead of asserting the push was refused.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via plain-last alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Thufir R11 empty-subsection: `alias..pub = push` (empty-subsection form)
/// must be refused on binaries that dispatch it (Git 2.54).  Git treats
/// `alias..<name>` as plain configuration and includes it in last-wins ordering.
///
/// **Self-gate:** skips on binaries that do not dispatch the empty-subsection
/// form — those binaries treat it as an unknown alias name, so no bypass exists.
#[test]
fn wrapper_refuses_push_via_empty_subsection_alias() {
    // Two-step gate: dispatch probe AND empty-subsection dispatch (both isolated).
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }
    match isolated_empty_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias..<name> form");
            return;
        }
        ProbeVerdict::Failure => panic!("empty-subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Empty-subsection form alias..pub = push.
    // `git config` cannot set an empty-subsection key via command-line arguments;
    // write the config fragment directly.
    let git_config = repo.path().join(".git").join("config");
    let existing = std::fs::read_to_string(&git_config).unwrap_or_default();
    std::fs::write(
        &git_config,
        format!("{existing}\n[alias \"\"]\n\tpub = push\n"),
    )
    .unwrap();

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias..pub=push (empty-subsection) must be expanded and refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected the push-gate rejection; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Mutation evidence: removing empty-subsection matching from `resolve_alias`
    // leaves `pub` unresolved; the wrapper treats it as a real (non-push)
    // command; git dispatches the push; the remote acquires the commit.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via empty-subsection alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Thufir R11 positive control: a legitimate agent-authored push via a
/// subsection alias (`alias.pub.command = push`) is allowed through — the
/// wrapper must not block valid subsection-alias pushes.
///
/// **Self-gate:** skips on binaries without subsection alias dispatch support.
#[test]
fn wrapper_allows_agent_push_via_subsection_command_alias() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, email, _keydir) = signed_shim_env();
    let (_work, repo, remote) = agent_repo_with_remote(&email);
    // Make an agent-signed commit so there is something to push.
    std::fs::write(repo.join("f"), "x").unwrap();
    wrapper(&path, &repo, &["add", "f"]);
    let commit_out = wrapper(&path, &repo, &["commit", "-m", "agent commit"]);
    assert!(
        commit_out.status.success(),
        "agent commit must succeed; stderr={}",
        String::from_utf8_lossy(&commit_out.stderr),
    );
    // Set ONLY the subsection form; no plain alias.pub.
    wrapper(&path, &repo, &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, &repo, &["pub", "origin", "main"]);
    assert!(
        out.status.success(),
        "agent-authored commit via subsection alias.pub.command=push must succeed; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Remote must have received the commit.
    let refs = hermetic_command("git")
        .args(["-C", remote.to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        !refs.stdout.is_empty(),
        "remote must have received the agent commit via subsection alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

// ── Thufir pass-2 new regression shapes ──────────────────────────────────────

/// R12-a: Uppercase invocation — `git PUB` where `alias.pub = push` must be
/// refused.  Git normalises section-variable names to lowercase, so the plain
/// alias key is `alias.pub` regardless of how the command was invoked.  The
/// wrapper must case-insensitively compare the invocation name to the key; a
/// case-sensitive compare would return `NotPush` and bypass verification.
///
/// The invocation passes `PUB` (uppercased) as the subcommand.  On a
/// supporting binary the NUL-framed walk compares case-insensitively; on an
/// unsupporting binary `git config --get alias.PUB` would fail (git normalises
/// on write, so the key is stored as `alias.pub`).  The resolver handles both:
/// the plain-form path on unsupporting binaries falls through to `config --get`
/// which already does the right thing (git's own key lookup is case-insensitive
/// for the variable part).
#[test]
fn wrapper_refuses_push_via_uppercase_alias_invocation() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    wrapper(&path, repo.path(), &["config", "alias.pub", "push"]);

    // Invoke as "PUB" (uppercase) — same underlying alias, different casing.
    let out = wrapper(&path, repo.path(), &["PUB", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub=push invoked as PUB must be refused (human-authored HEAD); \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected push-gate author refusal; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via uppercase alias invocation; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R12-b: Regex-metachar subsection name — `alias.pub[1].command = push`
/// (subsection `pub[1]`) invoked as `pub[1]`.  The name `pub[1]` would be
/// mismatched by a regex-interpolated pattern (the `[1]` is a character class
/// in ERE); the structural-parse resolver must match it literally.
///
/// Self-gate: skips on binaries that don't dispatch `.command` form aliases.
#[test]
fn wrapper_refuses_push_via_regex_metachar_subsection_alias() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Write alias.pub[1].command = push directly into repo config (git config
    // CLI won't accept metachar subsection names as arguments).
    let git_config = repo.path().join(".git").join("config");
    let existing = std::fs::read_to_string(&git_config).unwrap_or_default();
    std::fs::write(
        &git_config,
        format!("{existing}\n[alias \"pub[1]\"]\n\tcommand = push\n"),
    )
    .unwrap();

    let out = wrapper(&path, repo.path(), &["pub[1]", "origin", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub[1].command=push must be refused; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected push-gate author refusal; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via regex-metachar subsection alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R12-c/d: Dotted-command alias regression — two cases.
///
/// Case A: `alias.pub.command.command = push` + `alias.pub.command = status`.
/// Invoking `pub.command` should resolve the subsection alias for `pub.command`
/// (defined by `alias.pub.command.command=push`) and refuse.  The presence of
/// `alias.pub.command=status` must not obscure the push.
///
/// Case B (CRIT-1): `alias..pub.command = push` invoked as `.pub`.
/// Git 2.54 parses this as subsection `.pub`, variable `command` — dispatches
/// `.pub` via the subsection alias path.  The wrapper must classify the
/// subsection as ".pub" (leading dot preserved) and refuse.
///
/// Both cases must produce an author-refusal + empty remote.
///
/// Self-gate: skips on binaries that don't dispatch `.command` form aliases.
#[test]
fn wrapper_does_not_misresove_dotted_command_name_as_subsection_alias() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    // ── Case A: alias.pub.command.command=push, alias.pub.command=status ──
    {
        let (_shim, path, _email, _keydir) = signed_shim_env();
        let repo = human_repo();
        let remote = tempfile::tempdir().unwrap();
        assert!(hermetic_command("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        wrapper(
            &path,
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );

        // Both entries present: the push lives in the deeper subsection key.
        wrapper(
            &path,
            repo.path(),
            &["config", "alias.pub.command.command", "push"],
        );
        wrapper(
            &path,
            repo.path(),
            &["config", "alias.pub.command", "status"],
        );

        // Invoke pub.command — should resolve to push via the subsection alias;
        // must be refused.
        let out = wrapper(&path, repo.path(), &["pub.command", "origin", "main"]);
        assert!(
            !out.status.success(),
            "alias.pub.command.command=push must be refused for `pub.command`; \
             stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
            "expected push-gate author refusal for dotted-name alias; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        let refs = hermetic_command("git")
            .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
            .output()
            .unwrap();
        assert!(
            refs.stdout.is_empty(),
            "remote must be empty after refused dotted-command push; refs={}",
            String::from_utf8_lossy(&refs.stdout),
        );
    }

    // ── Case B: CRIT-1 — alias..pub.command=push invoked as `.pub` ──
    {
        let (_shim, path, _email, _keydir) = signed_shim_env();
        let repo = human_repo();
        let remote = tempfile::tempdir().unwrap();
        assert!(hermetic_command("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        wrapper(
            &path,
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );

        let out = wrapper(
            &path,
            repo.path(),
            &[
                "-c",
                "alias..pub.command=push",
                ".pub",
                remote.path().to_str().unwrap(),
                "main",
            ],
        );
        assert!(
            !out.status.success(),
            "alias..pub.command=push must be refused when invoked as `.pub`; \
             stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
            "expected push-gate author refusal for leading-dot subsection alias; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        let refs = hermetic_command("git")
            .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
            .output()
            .unwrap();
        assert!(
            refs.stdout.is_empty(),
            "remote must be empty after refused leading-dot subsection push; refs={}",
            String::from_utf8_lossy(&refs.stdout),
        );
    }
}

/// R12-d: Stored-newline value — `alias.pub = push\norigin` (a literal newline
/// in the value, stored in repo config, NOT in argv).  Git treats the newline
/// as whitespace when parsing the alias body, so `git pub main` dispatches
/// `git push origin main`.  The wrapper must resolve the full multi-line value
/// from the NUL-framed output and detect the push subcommand in the body.
///
/// This is distinct from the argv-newline guard (which rejects `\n` in push
/// URLs): here the newline lives in the stored alias body, not on the command
/// line.
///
/// Self-gate: skips on binaries that don't dispatch `.command` form aliases
/// (NUL-framed enumeration is only used on supporting binaries; the old-git
/// path falls back to `config --get` which also returns the full value).
#[test]
fn wrapper_refuses_push_via_multiline_alias_value() {
    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );

    // Store alias.pub = "push\norigin" — a value containing a literal newline.
    // `git config` can set multi-line values; the value is preserved as-is.
    let git_config = repo.path().join(".git").join("config");
    let existing = std::fs::read_to_string(&git_config).unwrap_or_default();
    std::fs::write(
        &git_config,
        format!("{existing}\n[alias]\n\tpub = push\\norigin\n"),
    )
    .unwrap();

    // git pub main — git expands "push\norigin" treating \n as whitespace →
    // effectively `git push origin main`.
    let out = wrapper(&path, repo.path(), &["pub", "main"]);
    assert!(
        !out.status.success(),
        "alias.pub with newline-in-body (push\\norigin) must be refused; \
         stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // The wrapper must have caught the push alias, not silently let git run it.
    // Either a push-gate refusal OR an alias-safety refusal (the body may
    // contain a backslash or other unsafe token) is acceptable here — the key
    // is that the push did NOT reach the remote.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via multiline alias value; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R12-e: Probe-poisoning — a `git-_probe_` helper placed BESIDE the selected
/// git binary must NOT flip the capability verdict.  The new probe uses a
/// probe-only private directory (containing only a controlled `git` symlink)
/// for both `PATH` and `GIT_EXEC_PATH`, so git's exec-path search cannot find
/// any adjacent helper.
///
/// This test exercises the INSTALLED WRAPPER (not the test-only probe fn):
///  - Positive control: sibling helper exits 42 + prints "git version sentinel
///    helper" → wrapper still refuses the push with author-refusal (probe was
///    not fooled; capability verdict is correct).
///  - Negative control: same alias without any sibling helper → also refused.
///
/// Both controls must produce author-refusal + empty remote.
/// Does NOT mutate the global PATH — uses explicit PATH env on wrapper() instead.
#[test]
fn probe_isolation_rejects_sibling_helper_poisoning() {
    match isolated_subsection_probe() {
        ProbeVerdict::Supported => {}
        ProbeVerdict::Unsupported => {
            eprintln!("skip: installed git does not dispatch alias.<name>.command form");
            return;
        }
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }

    // Find the real git binary's absolute path.
    let git_dir = real_git_dir();
    let real_git_abs = {
        let candidate = git_dir.join("git");
        candidate.canonicalize().unwrap_or(candidate)
    };

    // Create a "sibling dir": a directory that contains both `git` (symlink to
    // the real binary) and `git-_probe_` (a helper that exits 42 and prints
    // the sentinel string — the classic PATH-poisoning payload).
    // When we build a PATH with sibling_dir before shim_dir, find_real_git()
    // inside the wrapper finds sibling_dir/git as the "real" git, so the
    // probe's isolation logic is exercised against a helper adjacent to the
    // selected binary.
    let sibling_dir = tempfile::tempdir().unwrap();
    let git_link_in_sibling = sibling_dir.path().join("git");
    std::os::unix::fs::symlink(&real_git_abs, &git_link_in_sibling).unwrap();
    let helper_path = sibling_dir.path().join("git-_probe_");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&helper_path).unwrap();
        write!(
            f,
            "#!/bin/sh\nprintf 'git version sentinel helper\\n'\nexit 42\n"
        )
        .unwrap();
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&helper_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&helper_path, perms).unwrap();
    }

    // Build the shim environment manually, using sibling_dir as the real-git
    // dir in the PATH string.  This avoids mutating the process-global PATH,
    // which would cause races with parallel tests.
    //
    // PATH = shim_dir : sibling_dir : (original PATH minus real git dir)
    //
    // find_real_git() inside the wrapper will:
    //   1. See shim_dir/git — skip it (that's itself, buzz-acp).
    //   2. See sibling_dir/git — pick it up as the "real" git.
    //   (sibling_dir/git is a symlink to real_git_abs, so execution is correct.)
    let make_path_with_sibling = |shim: &tempfile::TempDir| -> String {
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let filtered: Vec<PathBuf> = std::env::split_paths(&original_path)
            .filter(|d| d.canonicalize().ok() != git_dir.canonicalize().ok())
            .collect();
        std::env::join_paths(
            std::iter::once(shim.path())
                .chain(std::iter::once(sibling_dir.path()))
                .chain(filtered.iter().map(|d| d.as_path())),
        )
        .unwrap()
        .into_string()
        .unwrap()
    };

    // ── Positive control: sibling helper present ──
    {
        use nostr::ToBech32;
        let keys = nostr::Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let keydir = tempfile::tempdir().unwrap();
        let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
        let shim = tempfile::tempdir().unwrap();
        for name in ["git", "git-sign-nostr"] {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name))
                .unwrap();
        }
        let entries = manifest_entries(&id);
        buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();
        let path = make_path_with_sibling(&shim);

        let repo = human_repo();
        let remote = tempfile::tempdir().unwrap();
        assert!(hermetic_command("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        // Use the shim path for setup commands too.
        wrapper(
            &path,
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

        let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
        assert!(
            !out.status.success(),
            "push via subsection alias must be refused despite sibling helper; \
             stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
            "expected author-refusal despite sibling helper; stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        let refs = hermetic_command("git")
            .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
            .output()
            .unwrap();
        assert!(
            refs.stdout.is_empty(),
            "remote must be empty (positive control); refs={}",
            String::from_utf8_lossy(&refs.stdout),
        );
    }

    // ── Negative control: no helper, same PATH setup ──
    std::fs::remove_file(&helper_path).unwrap();
    {
        use nostr::ToBech32;
        let keys = nostr::Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let keydir = tempfile::tempdir().unwrap();
        let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
        let shim = tempfile::tempdir().unwrap();
        for name in ["git", "git-sign-nostr"] {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name))
                .unwrap();
        }
        let entries = manifest_entries(&id);
        buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();
        let path = make_path_with_sibling(&shim);

        let repo = human_repo();
        let remote = tempfile::tempdir().unwrap();
        assert!(hermetic_command("git")
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        wrapper(
            &path,
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

        let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
        assert!(
            !out.status.success(),
            "push via subsection alias must be refused without sibling helper; \
             stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
            "expected author-refusal (negative control); stderr={}",
            String::from_utf8_lossy(&out.stderr),
        );
        let refs = hermetic_command("git")
            .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
            .output()
            .unwrap();
        assert!(
            refs.stdout.is_empty(),
            "remote must be empty (negative control); refs={}",
            String::from_utf8_lossy(&refs.stdout),
        );
    }
}

/// R12-f (primary binary): Mixed-definition on the primary git binary — when BOTH
/// `alias.pub` (plain form, push) AND `alias.pub.command` (subsection form, status)
/// are set, the effective alias depends on capability.
///
/// **Definition order:** plain=push first, `.command=status` second. This order
/// distinguishes the two semantics: on a supporting binary last-wins applies and
/// `.command=status` overrides the plain form; on a non-supporting binary the
/// `.command` form is invisible and `plain=push` is the only effective definition.
///
/// On supporting git (2.54): `.command=status` wins (last-wins). The wrapper
///   allows the alias (no push → no push-gate refusal).
/// On non-supporting git (2.50): `.command` invisible; plain=push is effective
///   → push is refused with the author-policy error, remote stays empty, and
///   `for-each-ref` exits 0 confirming the ref check itself succeeded.
///
/// Runs unconditionally: does not require a second git binary.  For two-binary
/// capability-matrix coverage see `wrapper_refuses_push_plain_last_wins_alt_binary`.
#[test]
fn wrapper_refuses_push_plain_last_wins_primary_binary() {
    // ── Primary binary ──
    let primary_supports_subsection = isolated_subsection_probe();
    match primary_supports_subsection {
        ProbeVerdict::Supported | ProbeVerdict::Unsupported => {}
        ProbeVerdict::Failure => panic!("subsection probe failed (setup/spawn/timeout)"),
    }
    eprintln!(
        "primary binary: {} subsection aliases",
        if primary_supports_subsection == ProbeVerdict::Supported {
            "SUPPORTS"
        } else {
            "does NOT support"
        }
    );

    let (_shim, path, _email, _keydir) = signed_shim_env();
    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    // Write BOTH definitions: plain=push first, .command=status second.
    // Definition order distinguishes the two semantics:
    //   - Supporting binary (last-wins): .command=status overrides plain=push
    //     → alias expands to status → no push-gate refusal.
    //   - Non-supporting binary: .command form is invisible; plain=push is the
    //     only effective definition → push → push-gate refusal.
    wrapper(&path, repo.path(), &["config", "alias.pub", "push"]);
    wrapper(
        &path,
        repo.path(),
        &["config", "alias.pub.command", "status"],
    );

    match primary_supports_subsection {
        ProbeVerdict::Supported => {
            // Supporting binary: .command=status wins; the push gate does not
            // fire. The invocation must succeed, and the remote stays empty
            // because status never pushed anything.
            let out = wrapper(&path, repo.path(), &["pub"]);
            assert!(
                out.status.success(),
                "supporting binary (.command wins): status alias must succeed; stderr={}",
                String::from_utf8_lossy(&out.stderr),
            );
            assert_remote_empty(remote.path());
        }
        ProbeVerdict::Unsupported => {
            // Non-supporting binary: .command is invisible; plain=push fires.
            let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
            assert!(
                !out.status.success(),
                "alias.pub=push (plain, non-supporting binary) must be refused; stderr={}",
                String::from_utf8_lossy(&out.stderr),
            );
            assert!(
                String::from_utf8_lossy(&out.stderr)
                    .contains("not authored by your agent identity"),
                "expected push-gate author refusal on non-supporting primary binary; stderr={}",
                String::from_utf8_lossy(&out.stderr),
            );
            let refs = hermetic_command("git")
                .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
                .output()
                .unwrap();
            assert!(
                refs.status.success(),
                "for-each-ref must exit 0; status={:?}",
                refs.status,
            );
            assert!(
                refs.stdout.is_empty(),
                "remote must be empty on non-supporting primary binary; refs={}",
                String::from_utf8_lossy(&refs.stdout),
            );
        }
        ProbeVerdict::Failure => unreachable!("Failure already caught above"),
    }
}

/// R12-f (alternate binary): same mixed-definition refusal exercised against a
/// second git installation (e.g. Homebrew git alongside Apple git).
///
/// **Requires two distinct git binaries at `/usr/bin/git` and
/// `/opt/homebrew/bin/git`.**  Run with `--run-ignored` on a machine that has
/// both installations; the required `just test-unit` CI lane (Ubuntu) does not
/// provision a second git and would fail deterministically without `#[ignore]`.
#[test]
#[ignore = "requires two git installations (e.g. /opt/homebrew/bin/git + /usr/bin/git)"]
fn wrapper_refuses_push_plain_last_wins_alt_binary() {
    let apple_git = std::path::Path::new("/usr/bin/git");
    let brew_git = std::path::Path::new("/opt/homebrew/bin/git");
    let primary = real_git_dir().join("git");
    let alt_git_dir: Option<std::path::PathBuf> = [apple_git, brew_git]
        .iter()
        .find(|p| p.is_file() && p.canonicalize().ok() != primary.canonicalize().ok())
        .map(|p| p.parent().unwrap().to_path_buf());

    let alt_dir = match alt_git_dir {
        Some(d) => d,
        None => {
            eprintln!(
                "wrapper_refuses_push_plain_last_wins_alt_binary: skipping — \
                 no second git binary found at /usr/bin/git or /opt/homebrew/bin/git \
                 distinct from the primary; provision two git installations to run this test"
            );
            return;
        }
    };
    let alt_git = alt_dir.join("git");
    let alt_ver = hermetic_command(&alt_git)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Probe the alt binary with the same isolation model as the primary.
    let alt_supports_subsection =
        run_isolated_probe_for(&alt_git, "alias._probe_.command=version", "_probe_");
    assert!(
        alt_supports_subsection != ProbeVerdict::Failure,
        "alt-git subsection probe failed (setup/spawn/timeout/unclassifiable)"
    );
    let primary_supports_subsection = isolated_subsection_probe();
    assert!(
        primary_supports_subsection != ProbeVerdict::Failure,
        "primary-git subsection probe failed (setup/spawn/timeout/unclassifiable)"
    );
    assert_ne!(
        alt_supports_subsection, primary_supports_subsection,
        "prerequisite: two git installations with DIFFERENT subsection-alias capability"
    );
    eprintln!(
        "alternate binary ({alt_ver} @ {}): {} subsection aliases",
        alt_git.display(),
        if alt_supports_subsection == ProbeVerdict::Supported {
            "SUPPORTS"
        } else {
            "does NOT support"
        }
    );

    // Build the shim PATH string with alt_dir as the real-git dir — no global
    // PATH mutation needed.  PATH = shim_dir : alt_dir : (original minus primary)
    // find_real_git() in the wrapper subprocess sees shim_dir/git (itself, skip)
    // then alt_dir/git (the alt binary — pick it up).
    let primary_dir = real_git_dir();
    let make_alt_path = |shim: &tempfile::TempDir| -> String {
        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let filtered: Vec<PathBuf> = std::env::split_paths(&original_path)
            .filter(|d| d.canonicalize().ok() != primary_dir.canonicalize().ok())
            .collect();
        std::env::join_paths(
            std::iter::once(shim.path())
                .chain(std::iter::once(alt_dir.as_path()))
                .chain(filtered.iter().map(|d| d.as_path())),
        )
        .unwrap()
        .into_string()
        .unwrap()
    };

    let result = std::panic::catch_unwind(|| {
        use nostr::ToBech32;
        let keys = nostr::Keys::generate();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let keydir = tempfile::tempdir().unwrap();
        let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
        let shim = tempfile::tempdir().unwrap();
        for name in ["git", "git-sign-nostr"] {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name))
                .unwrap();
        }
        let entries = manifest_entries(&id);
        buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();
        let path = make_alt_path(&shim);

        let repo = human_repo();
        let remote = tempfile::tempdir().unwrap();
        assert!(hermetic_command(&alt_git)
            .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        wrapper(
            &path,
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        // Write BOTH definitions on the alt binary too: plain=push first,
        // .command=status second. The alt binary has the OPPOSITE capability
        // from primary (enforced by the prerequisite assertion above), so this
        // ordering exercises the branch that the primary test cannot reach.
        wrapper(&path, repo.path(), &["config", "alias.pub", "push"]);
        wrapper(
            &path,
            repo.path(),
            &["config", "alias.pub.command", "status"],
        );

        match alt_supports_subsection {
            ProbeVerdict::Supported => {
                // Supporting alt binary: .command=status wins; the invocation
                // must succeed and push nothing.
                let out = wrapper(&path, repo.path(), &["pub"]);
                assert!(
                    out.status.success(),
                    "alt binary ({alt_ver}, .command wins): status alias must succeed; stderr={}",
                    String::from_utf8_lossy(&out.stderr),
                );
                assert_remote_empty(remote.path());
            }
            ProbeVerdict::Unsupported => {
                // Non-supporting alt binary: .command invisible; plain=push.
                let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
                assert!(
                    !out.status.success(),
                    "alias.pub=push must be refused on alt git ({alt_ver}); stderr={}",
                    String::from_utf8_lossy(&out.stderr),
                );
                assert!(
                    String::from_utf8_lossy(&out.stderr)
                        .contains("not authored by your agent identity"),
                    "expected push-gate author refusal on alt git ({alt_ver}); stderr={}",
                    String::from_utf8_lossy(&out.stderr),
                );
                let refs = hermetic_command(&alt_git)
                    .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
                    .output()
                    .unwrap();
                assert!(
                    refs.status.success(),
                    "for-each-ref must exit 0 on alt git; status={:?}",
                    refs.status,
                );
                assert!(
                    refs.stdout.is_empty(),
                    "remote must be empty on alt git ({alt_ver}); refs={}",
                    String::from_utf8_lossy(&refs.stdout),
                );
            }
            ProbeVerdict::Failure => {
                panic!("alt-git subsection probe failed unexpectedly inside catch_unwind")
            }
        }
    });

    result.unwrap();
}

/// Regression: empty-value-then-valid ordering — last-wins allows the valid expansion.
///
/// `git -c alias.pub= -c alias.pub=version pub` executes `git version` on both
/// real git binaries (exit 0).  The wrapper must also allow this invocation
/// (the push gate refuses it for identity reasons, not alias-resolution reasons).
///
/// Self-gate: skips on binaries that don't support plain aliases (all do).
#[test]
fn wrapper_allows_empty_then_valid_alias_override() {
    // Both git binaries accept this ordering.  Verify the wrapper's resolver
    // does not prematurely bail on the first empty record.
    let (_shim, path, _email, _keydir) = signed_shim_env();

    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );

    // Write empty first, valid second — last-wins means "push" is effective.
    wrapper(&path, repo.path(), &["config", "alias.pub", ""]);
    wrapper(
        &path,
        repo.path(),
        &["config", "--add", "alias.pub", "push"],
    );

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    // The push gate fires (author-identity check) — the alias WAS resolved.
    assert!(
        !out.status.success(),
        "wrapper must refuse (author gate), not fail on alias resolution; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not authored by your agent identity"),
        "expected author-gate refusal after resolving empty-then-valid alias; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Destination must be empty — the push was refused before anything landed.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Regression: valid-then-empty ordering — last-wins means empty wins, which
/// must be refused by the wrapper (fail closed on empty final expansion).
///
/// `git -c alias.pub=version -c alias.pub= pub` fails on both real git binaries
/// ("'' is not a git command").  The wrapper must refuse this too.
///
/// Self-gate: skips on binaries that don't support plain aliases (all do).
#[test]
fn wrapper_refuses_valid_then_empty_alias_override() {
    let (_shim, path, _email, _keydir) = signed_shim_env();

    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );

    // Write valid first, empty second — last-wins means empty is effective.
    wrapper(&path, repo.path(), &["config", "alias.pub", "push"]);
    wrapper(&path, repo.path(), &["config", "--add", "alias.pub", ""]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "wrapper must refuse empty final alias expansion; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    // The remote must be untouched — refused before any push.
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused empty-final alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Regression: probe classifier — exit 1 with version-looking stdout must NOT
/// classify as Unsupported; it must result in ProbeFailure (fail closed).
///
/// This test verifies the installed wrapper treats such a probe result as
/// ProbeFailure by constructing a PATH where the selected `git` binary is a
/// helper that exits 1 but prints "git version sentinel" to stdout.
#[test]
fn wrapper_treats_exit1_with_stdout_as_probe_failure_not_unsupported() {
    // Build a fake "git" that exits 1 and prints "git version exit1" to stdout.
    let fake_dir = tempfile::tempdir().unwrap();
    let fake_git = fake_dir.path().join("git");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&fake_git).unwrap();
        write!(f, "#!/bin/sh\nprintf 'git version exit1\\n'\nexit 1\n").unwrap();
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake_git).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_git, perms).unwrap();
    }

    // Build a shim PATH: shim_dir → fake_dir (fake git) → rest.
    // find_real_git() will skip shim_dir (itself) and pick up fake_dir/git.
    // The production probe for fake_dir/git will get exit 1 + "git version"
    // stdout — this must yield ProbeFailure, causing the wrapper to refuse
    // with "alias capability probe failed".
    use nostr::ToBech32;
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let keydir = tempfile::tempdir().unwrap();
    let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
    let shim = tempfile::tempdir().unwrap();
    for name in ["git", "git-sign-nostr"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name)).unwrap();
    }
    let entries = manifest_entries(&id);
    buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();

    // PATH: shim_dir : fake_dir : (original minus real-git dir)
    let real_dir = real_git_dir();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let filtered: Vec<PathBuf> = std::env::split_paths(&original_path)
        .filter(|d| d.canonicalize().ok() != real_dir.canonicalize().ok())
        .collect();
    let path = std::env::join_paths(
        std::iter::once(shim.path())
            .chain(std::iter::once(fake_dir.path()))
            .chain(filtered.iter().map(|d| d.as_path())),
    )
    .unwrap()
    .into_string()
    .unwrap();

    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "wrapper must refuse when probe yields exit-1+stdout; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("alias capability probe failed"),
        "expected ProbeFailure message for exit-1+stdout probe; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after probe-failure refusal; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Regression: probe classifier — exit 1 + empty stdout + UNRELATED stderr must NOT
/// classify as Unsupported; it must result in ProbeFailure (fail closed).
///
/// The Unsupported branch requires stderr to start with old git's sentinel
/// diagnostic (`git: '_probe_' is not a git command`); any other message is
/// not a recognized refusal.
#[test]
fn wrapper_treats_exit1_with_unrelated_stderr_as_probe_failure_not_unsupported() {
    // Build a fake "git" that exits 1 with unrelated stderr and empty stdout.
    let fake_dir = tempfile::tempdir().unwrap();
    let fake_git = fake_dir.path().join("git");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&fake_git).unwrap();
        // Exits 1, stdout empty, stderr = unrelated message (no "is not a git command")
        write!(f, "#!/bin/sh\nprintf 'unrelated error\\n' >&2\nexit 1\n").unwrap();
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake_git).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_git, perms).unwrap();
    }

    use nostr::ToBech32;
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let keydir = tempfile::tempdir().unwrap();
    let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
    let shim = tempfile::tempdir().unwrap();
    for name in ["git", "git-sign-nostr"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name)).unwrap();
    }
    let entries = manifest_entries(&id);
    buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();

    // PATH: shim_dir : fake_dir : (original minus real-git dir)
    let real_dir = real_git_dir();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let filtered: Vec<PathBuf> = std::env::split_paths(&original_path)
        .filter(|d| d.canonicalize().ok() != real_dir.canonicalize().ok())
        .collect();
    let path = std::env::join_paths(
        std::iter::once(shim.path())
            .chain(std::iter::once(fake_dir.path()))
            .chain(filtered.iter().map(|d| d.as_path())),
    )
    .unwrap()
    .into_string()
    .unwrap();

    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "wrapper must refuse when probe yields exit-1+unrelated-stderr; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("alias capability probe failed"),
        "expected ProbeFailure message for exit-1+unrelated-stderr probe; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after probe-failure refusal; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// Regression: probe classifier — exit 1 + empty stdout + EMPTY stderr must NOT
/// classify as Unsupported; it must result in ProbeFailure (fail closed).
///
/// A silent exit-1 (no stdout, no stderr) is not recognizable as old-git's
/// unknown-command refusal and must be treated as ProbeFailure.
#[test]
fn wrapper_treats_exit1_with_empty_stderr_as_probe_failure_not_unsupported() {
    // Build a fake "git" that exits 1 with no stdout and no stderr.
    let fake_dir = tempfile::tempdir().unwrap();
    let fake_git = fake_dir.path().join("git");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&fake_git).unwrap();
        // Exits 1, stdout empty, stderr empty
        write!(f, "#!/bin/sh\nexit 1\n").unwrap();
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake_git).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_git, perms).unwrap();
    }

    use nostr::ToBech32;
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let keydir = tempfile::tempdir().unwrap();
    let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
    let shim = tempfile::tempdir().unwrap();
    for name in ["git", "git-sign-nostr"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name)).unwrap();
    }
    let entries = manifest_entries(&id);
    buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();

    // PATH: shim_dir : fake_dir : (original minus real-git dir)
    let real_dir = real_git_dir();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let filtered: Vec<PathBuf> = std::env::split_paths(&original_path)
        .filter(|d| d.canonicalize().ok() != real_dir.canonicalize().ok())
        .collect();
    let path = std::env::join_paths(
        std::iter::once(shim.path())
            .chain(std::iter::once(fake_dir.path()))
            .chain(filtered.iter().map(|d| d.as_path())),
    )
    .unwrap()
    .into_string()
    .unwrap();

    let repo = human_repo();
    let remote = tempfile::tempdir().unwrap();
    assert!(hermetic_command("git")
        .args(["init", "-q", "--bare", remote.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    wrapper(
        &path,
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    wrapper(&path, repo.path(), &["config", "alias.pub.command", "push"]);

    let out = wrapper(&path, repo.path(), &["pub", "origin", "main"]);
    assert!(
        !out.status.success(),
        "wrapper must refuse when probe yields exit-1+empty-stderr; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("alias capability probe failed"),
        "expected ProbeFailure message for exit-1+empty-stderr probe; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let refs = hermetic_command("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after probe-failure refusal; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

// ── Nested-agent process-level tests ─────────────────────────────────────────
//
// These tests use REAL wrapper installations (the buzz-acp multicall binary)
// with valid parent and child manifests to verify nested-agent execution.
// They exercise actual `git init`, `git add`, and `git commit` operations and
// verify that the resulting commit carries the child's author identity.
//
// The child install directory uses a **copy** of the buzz-acp binary so its
// canonical path differs from the parent's, exercising distinct-executable-path
// nesting. The parent directory uses a symlink as in normal installs.

/// Build a nested wrapper install directory.
///
/// If `copy_binary` is true, the `git` and `git-sign-nostr` entries are copies
/// of the buzz-acp binary (distinct canonical executable); if false they are
/// symlinks. Both layouts carry a valid `.git-identity` manifest.
/// Returns (TempDir for the install, TempDir for the keyfile, expected email).
#[cfg(unix)]
fn nested_shim(copy_binary: bool) -> (tempfile::TempDir, tempfile::TempDir, String) {
    use nostr::ToBech32;
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let keydir = tempfile::tempdir().unwrap();
    let id = write_agent_key(keydir.path(), &nsec).expect("write keyfile");
    let expected_email = format!("{}@relay.test", id.pubkey_hex);

    let shim = tempfile::tempdir().unwrap();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_buzz-acp"));
    for name in ["git", "git-sign-nostr"] {
        let dest = shim.path().join(name);
        if copy_binary {
            std::fs::copy(bin, &dest).unwrap();
            // Ensure the copy is executable.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
        } else {
            std::os::unix::fs::symlink(bin, &dest).unwrap();
        }
    }
    buzz_git_identity::write_identity_manifest(shim.path(), &manifest_entries(&id)).unwrap();
    (shim, keydir, expected_email)
}

/// Nested-agent: distinct canonical executables, child first on PATH.
///
/// Child install dir uses a COPY of buzz-acp (distinct inode/canonical path).
/// Parent install dir uses a symlink (normal layout).
/// PATH = child_dir : parent_dir : real_git_dir.
///
/// `find_real_git` in the child wrapper: skips child dir (self-skip via
/// canonicalization), skips parent dir (marker), selects real git. An ordinary
/// `git commit` completes successfully and the commit carries the CHILD's author
/// identity, not the parent's. Bounded: the whole sequence must complete within
/// 30 seconds (no hang).
#[cfg(unix)]
#[test]
fn nested_agent_distinct_binaries_child_first_uses_child_identity() {
    let (child_shim, _child_keydir, child_email) = nested_shim(true); // copy = distinct canonical
    let (parent_shim, _parent_keydir, _parent_email) = nested_shim(false); // symlink

    let real = real_git_dir();
    let path = std::env::join_paths([
        child_shim.path().to_path_buf(),
        parent_shim.path().to_path_buf(),
        real,
    ])
    .unwrap()
    .into_string()
    .unwrap();

    // Set repo's local identity to an unrelated human so only the wrapper's
    // injection can produce the child agent email. If injection does not happen,
    // %ae/%ce will be "human@example.invalid", not child_email.
    let (_work, repo, _remote) = agent_repo_with_remote("human@example.invalid");

    let timeout = std::time::Duration::from_secs(30);

    // status: confirms ordinary git operations complete without hang.
    let status = wrapper_bounded(&path, &repo, &["status"], timeout);
    assert!(
        status.status.success(),
        "git status must succeed in nested context; stderr={}",
        String::from_utf8_lossy(&status.stderr),
    );

    std::fs::write(repo.join("nested.txt"), b"nested\n").unwrap();
    let add = wrapper_bounded(&path, &repo, &["add", "nested.txt"], timeout);
    assert!(
        add.status.success(),
        "git add must succeed in nested context; stderr={}",
        String::from_utf8_lossy(&add.stderr),
    );

    let commit = wrapper_bounded(
        &path,
        &repo,
        &["commit", "-m", "nested agent commit"],
        timeout,
    );
    assert!(
        commit.status.success(),
        "git commit must succeed in nested context; stderr={}; stdout={}",
        String::from_utf8_lossy(&commit.stderr),
        String::from_utf8_lossy(&commit.stdout),
    );

    // Both author and committer emails must be the child's agent email.
    // %ae alone could pass if local repo config happened to set the child email;
    // requiring %ce (set independently by the wrapper) eliminates that.
    let identity = hermetic_command("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae%n%ce",
            "HEAD",
        ])
        .output()
        .unwrap();
    let identity_str = String::from_utf8_lossy(&identity.stdout);
    let lines: Vec<&str> = identity_str.trim().splitn(2, '\n').collect();
    assert_eq!(lines.len(), 2, "expected two identity lines from git show");
    assert_eq!(
        lines[0].trim(),
        child_email,
        "author email must be child agent email"
    );
    assert_eq!(
        lines[1].trim(),
        child_email,
        "committer email must be child agent email"
    );
}

/// Nested-agent: same canonical executable, child first on PATH.
///
/// Both child and parent install dirs use symlinks to the same buzz-acp binary;
/// their canonical paths are identical. The child's own dir is skipped by the
/// self-skip (canonicalization) check, which runs before the marker check, so
/// it skips the parent's dir too. Real git is selected, and the child's identity (injected by
/// the child wrapper's manifest) is used for the commit.
#[cfg(unix)]
#[test]
fn nested_agent_same_binary_child_first_uses_child_identity() {
    let (child_shim, _child_keydir, child_email) = nested_shim(false); // symlink
    let (parent_shim, _parent_keydir, _parent_email) = nested_shim(false); // symlink

    let real = real_git_dir();
    let path = std::env::join_paths([
        child_shim.path().to_path_buf(),
        parent_shim.path().to_path_buf(),
        real,
    ])
    .unwrap()
    .into_string()
    .unwrap();

    let (_work, repo, _remote) = agent_repo_with_remote("human@example.invalid");

    let timeout = std::time::Duration::from_secs(30);

    let status = wrapper_bounded(&path, &repo, &["status"], timeout);
    assert!(
        status.status.success(),
        "git status must succeed (same-binary nested); stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );

    std::fs::write(repo.join("same.txt"), b"same\n").unwrap();
    let add = wrapper_bounded(&path, &repo, &["add", "same.txt"], timeout);
    assert!(
        add.status.success(),
        "git add must succeed; stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let commit = wrapper_bounded(
        &path,
        &repo,
        &["commit", "-m", "same-binary nested commit"],
        timeout,
    );
    assert!(
        commit.status.success(),
        "git commit must succeed (same-binary nested); stderr={}",
        String::from_utf8_lossy(&commit.stderr),
    );

    let identity = hermetic_command("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae%n%ce",
            "HEAD",
        ])
        .output()
        .unwrap();
    let identity_str = String::from_utf8_lossy(&identity.stdout);
    let lines: Vec<&str> = identity_str.trim().splitn(2, '\n').collect();
    assert_eq!(lines.len(), 2, "expected two identity lines (same-binary)");
    assert_eq!(
        lines[0].trim(),
        child_email,
        "author email must be child agent email (same-binary)"
    );
    assert_eq!(
        lines[1].trim(),
        child_email,
        "committer email must be child agent email (same-binary)"
    );
}

/// Nested-agent: reverse ordering — parent invoked with child dir also on PATH.
///
/// PATH = parent_dir : child_dir : real_git_dir. The parent wrapper is the
/// outermost invocation. `find_real_git` in the parent skips parent dir
/// (self-skip), skips child dir (marker), selects real git. The child dir must
/// never be selected as real git in this ordering.
#[cfg(unix)]
#[test]
fn nested_agent_reverse_ordering_parent_first_never_selects_child_as_real_git() {
    let (child_shim, _child_keydir, child_email) = nested_shim(true); // copy = distinct canonical
    let (parent_shim, _parent_keydir, parent_email) = nested_shim(false); // symlink

    let real = real_git_dir();
    // Reverse: parent first, child second.
    let path = std::env::join_paths([
        parent_shim.path().to_path_buf(),
        child_shim.path().to_path_buf(),
        real,
    ])
    .unwrap()
    .into_string()
    .unwrap();

    // Repo uses an unrelated human identity; only the parent wrapper's injection
    // can produce parent_email.
    let (_work, repo, _remote) = agent_repo_with_remote("human@example.invalid");

    let timeout = std::time::Duration::from_secs(30);

    let status = wrapper_bounded(&path, &repo, &["status"], timeout);
    assert!(
        status.status.success(),
        "git status must succeed (reverse ordering); stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );

    std::fs::write(repo.join("rev.txt"), b"reverse\n").unwrap();
    let add = wrapper_bounded(&path, &repo, &["add", "rev.txt"], timeout);
    assert!(
        add.status.success(),
        "git add must succeed (reverse); stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );

    let commit = wrapper_bounded(
        &path,
        &repo,
        &["commit", "-m", "reverse-order nested commit"],
        timeout,
    );
    assert!(
        commit.status.success(),
        "git commit must succeed (reverse ordering); stderr={}",
        String::from_utf8_lossy(&commit.stderr),
    );

    // The parent wrapper is first on PATH; commit must carry PARENT identity.
    // Critically: the child dir (which is a Buzz wrapper with its own manifest)
    // must NOT have been selected as real git.
    let identity = hermetic_command("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ae%n%ce",
            "HEAD",
        ])
        .output()
        .unwrap();
    let identity_str = String::from_utf8_lossy(&identity.stdout);
    let lines: Vec<&str> = identity_str.trim().splitn(2, '\n').collect();
    assert_eq!(lines.len(), 2, "expected two identity lines (reverse)");
    let actual_ae = lines[0].trim();
    let actual_ce = lines[1].trim();
    assert_eq!(
        actual_ae, parent_email,
        "reverse: author must be parent agent email (not child {})",
        child_email
    );
    assert_eq!(
        actual_ce, parent_email,
        "reverse: committer must be parent agent email"
    );
    assert_ne!(
        actual_ae, child_email,
        "reverse-order: child dir must not be selected as real git"
    );
}

// ── Relative-PATH absolutize regression ───────────────────────────────────────
//
// When the real-git directory is on PATH as a RELATIVE entry, find_real_git()
// must still return an absolute path. If it returns a relative path, the
// capability probe's absolute-path guard fires (ProbeFailure), and all aliases
// fail closed — including safe aliases like `alias.st=status`.
//
// This is the process-level regression for the absolutize fix in find_real_git.
// See also the unit-level `find_real_git_returns_absolute_path_for_relative_entry`
// in buzz-git-identity.

/// Absolutize regression: safe alias and push-gate refusal both work with a
/// relative real-git directory on PATH.
///
/// PATH = shim_dir (absolute) : `./real-git-dirname` (relative).
/// The wrapper process runs with cwd = parent of the real-git dir, so the
/// relative entry resolves to the real git binary. The `-C repo` flag points
/// git to the test repo regardless of the wrapper's cwd.
///
/// Asserts:
///  (a) `git -c alias.st=status st` exits 0 — the alias probe ran against
///      real git and resolved the alias (R10-1 would have caused ProbeFailure
///      here if the absolute guarantee were broken).
///  (b) `git push` with a human-authored commit produces the author-policy
///      refusal and `for-each-ref` exits 0 with empty refs.
#[cfg(unix)]
#[test]
fn wrapper_handles_relative_real_git_entry_via_absolutize() {
    let timeout = std::time::Duration::from_secs(60);
    // The real-git dir becomes a *relative* PATH entry (`./real-git`) that
    // resolves only because the wrapper process runs with cwd = rel_parent.
    let rel_parent = tempfile::tempdir().unwrap();
    let rel_dir_name = "real-git";
    let rel_dir = rel_parent.path().join(rel_dir_name);
    std::fs::create_dir_all(&rel_dir).unwrap();
    std::os::unix::fs::symlink(real_git_dir().join("git"), rel_dir.join("git")).unwrap();

    let (shim, _abs_path, _agent_email, _keydir) = signed_shim_env();
    let path = std::env::join_paths([shim.path().to_path_buf(), Path::new(".").join(rel_dir_name)])
        .unwrap()
        .into_string()
        .unwrap();

    // (a) Safe alias: without the absolutize fix the probe fails closed with
    // "alias capability probe failed".
    let repo = human_repo();
    let alias_out = wrapper_bounded(
        &path,
        rel_parent.path(),
        &[
            "-C",
            repo.path().to_str().unwrap(),
            "-c",
            "alias.st=status",
            "st",
        ],
        timeout,
    );
    assert!(
        alias_out.status.success(),
        "(a) safe alias must succeed with relative real-git dir; stderr={}",
        String::from_utf8_lossy(&alias_out.stderr),
    );

    // (b) A human-authored commit, made through real git, pushed through the
    // wrapper to an empty bare origin must hit the author-policy refusal.
    let work = tempfile::tempdir().unwrap();
    let push_repo = work.path().join("repo");
    let remote = work.path().join("remote.git");
    std::fs::create_dir_all(&push_repo).unwrap();
    let g = |cwd: &Path, args: &[&str]| {
        let ok = hermetic_command("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    };
    g(
        work.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    g(&push_repo, &["init", "-q", "-b", "main"]);
    g(&push_repo, &["config", "user.name", "Human Dev"]);
    g(
        &push_repo,
        &["config", "user.email", "human@example.invalid"],
    );
    g(&push_repo, &["config", "commit.gpgSign", "false"]);
    std::fs::write(push_repo.join("f.txt"), b"rel\n").unwrap();
    g(&push_repo, &["add", "f.txt"]);
    g(&push_repo, &["commit", "-qm", "human commit"]);

    let remote_path = remote.to_str().unwrap();
    let push = wrapper_bounded(
        &path,
        rel_parent.path(),
        &[
            "-C",
            push_repo.to_str().unwrap(),
            "push",
            remote_path,
            "main",
        ],
        timeout,
    );
    let stderr = String::from_utf8_lossy(&push.stderr);
    assert!(
        !push.status.success(),
        "(b) push must be refused; stderr={stderr}"
    );
    assert!(
        stderr.contains("not authored by your agent identity"),
        "(b) expected push-gate author refusal; stderr={stderr}",
    );
    let refs = hermetic_command("git")
        .args(["-C", remote_path, "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.status.success(),
        "(b) for-each-ref must exit 0; status={:?}",
        refs.status
    );
    assert!(
        refs.stdout.is_empty(),
        "(b) remote must be empty after refusal; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}
