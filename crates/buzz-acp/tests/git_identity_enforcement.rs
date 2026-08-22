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

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use nostr::ToBech32;

const AGENT_EMAIL: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@relay.test";
const ALIAS_HOP_LIMIT: usize = 10;

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

/// Build a shim dir containing `git` -> the buzz-acp multicall binary and a
/// `.git-identity` manifest, and a PATH with the shim ahead of real git.
/// Returns (shim TempDir, PATH string).
fn shim_env(manifest: &str) -> (tempfile::TempDir, String) {
    let shim = tempfile::tempdir().unwrap();
    let git_link = shim.path().join("git");
    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), &git_link).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(env!("CARGO_BIN_EXE_buzz-acp"), &git_link).unwrap();

    std::fs::write(shim.path().join(".git-identity"), manifest).unwrap();

    let real = real_git_dir();
    let path = std::env::join_paths([shim.path().to_path_buf(), real])
        .unwrap()
        .into_string()
        .unwrap();
    (shim, path)
}

/// Standard managed manifest with signing OFF (the test box has no nostr
/// signer; signing enforcement is covered by unit tests).
fn manifest() -> String {
    format!("user.name=Agent\nuser.email={AGENT_EMAIL}\ncommit.gpgSign=false\n")
}

/// A git repo with one human-authored commit and human-named local config.
fn human_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let g = |args: &[&str]| {
        let ok = Command::new("git")
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
        let ok = Command::new("git")
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
    let out = Command::new("git")
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
fn wrapper(path: &str, cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run wrapper git")
}

/// The current `HEAD` commit SHA of `repo`, via real git (empty if unborn).
fn head_sha(repo: &Path) -> String {
    let out = Command::new("git")
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
    let (_shim, path) = shim_env(&manifest());
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
    let (_shim, path) = shim_env(&manifest());
    let repo = human_repo();
    // A reachable bare remote so the dry-run plan resolves and HEAD (human
    // authored) is examined as an offender.
    let remote = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
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
    let refs = Command::new("git")
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
    let (_shim, path) = shim_env(&manifest());
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
    let author = Command::new("git")
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
        AGENT_EMAIL,
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
    let (_shim, path) = shim_env(&manifest());
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
    let (_shim, path) = shim_env(&manifest());
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
    let author = Command::new("git")
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
    assert_eq!(String::from_utf8_lossy(&author.stdout).trim(), AGENT_EMAIL);

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
    let (_shim, path) = shim_env(&manifest());
    let repo = human_repo();
    std::fs::write(repo.path().join("f"), "two").unwrap();
    wrapper(&path, repo.path(), &["add", "f"]);

    let out = wrapper(&path, repo.path(), &["commit", "-m", "agent authored"]);
    assert!(
        out.status.success(),
        "ordinary commit should succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    let author = Command::new("git")
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
        AGENT_EMAIL,
        "commit must be authored as the agent identity, not the repo-local human"
    );
}

/// I4: the spawn-path wiring — `AcpClient::spawn` → `install_git_identity` —
/// must actually install the wrapper + manifest onto the agent-runtime child.
///
/// The tests above manufacture their own `.git-identity` manifest, so they stay
/// green even if the `install_git_identity(&mut cmd)?` call in `spawn` is
/// deleted. This one drives the REAL `buzz-acp` binary through `buzz-acp models`
/// (whose spawn path is the code under test) with a script agent that runs a
/// bare `git commit` in a human-configured repo and records the resulting
/// author. It passes only when the spawn path installed the wrapper `git` ahead
/// of real git AND wrote a manifest naming the configured key's identity — so
/// removing the `install_git_identity` call makes it go RED (the commit lands as
/// the repo-local human, or fails).
///
/// `BUZZ_AUTH_TAG` is cleared so `git-sign-nostr` signs offline (no NIP-OA owner
/// attestation to verify against a relay); signing itself needs no network.
#[cfg(unix)]
#[test]
fn spawn_path_installs_identity_so_agent_commits_land_agent_authored() {
    use std::os::unix::fs::PermissionsExt;

    // A configured agent key and its derived author email (the wrapper builds
    // `<pubkey_hex>@<relay_host>` from BUZZ_RELAY_URL).
    let keys = nostr::Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let pubkey_hex = keys.public_key().to_hex();
    let expected_email = format!("{pubkey_hex}@relay.test");

    // A human-configured repo with a staged file, ready for one commit.
    let work = tempfile::tempdir().unwrap();
    let repo = work.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let g = |args: &[&str]| {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap()
            .success());
    };
    g(&["init", "-q", "-b", "main"]);
    g(&["config", "user.name", "Human Dev"]);
    g(&["config", "user.email", "human@example.com"]);
    std::fs::write(repo.join("f"), "hi").unwrap();
    g(&["add", "f"]);

    // Script "agent": commit in the repo using whatever `git` its PATH resolves
    // (the wrapper, if the spawn path installed it), record the author, exit.
    let out_file = work.path().join("author.txt");
    let agent = work.path().join("agent.sh");
    std::fs::write(
        &agent,
        format!(
            "#!/usr/bin/env bash\n\
             cd {repo:?}\n\
             git commit -m 'agent authored' >/dev/null 2>&1\n\
             git show -s --format=%ae HEAD > {out:?} 2>/dev/null\n\
             exit 0\n",
            repo = repo,
            out = out_file,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Drive the real binary. `models` spawns the agent (running install_git_identity),
    // then fails init (the script exits) — expected; we assert on the side effect.
    let output = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .args([
            "models",
            "--agent-command",
            agent.to_str().unwrap(),
            "--agent-args",
            "",
        ])
        .env("BUZZ_PRIVATE_KEY", &nsec)
        .env("BUZZ_RELAY_URL", "wss://relay.test")
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .output()
        .expect("run buzz-acp models");

    let author = std::fs::read_to_string(&out_file).unwrap_or_default();
    assert_eq!(
        author.trim(),
        expected_email,
        "spawn path must install the wrapper + manifest so the agent's commit is \
         authored as the configured key's identity; got {author:?}. models stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}
