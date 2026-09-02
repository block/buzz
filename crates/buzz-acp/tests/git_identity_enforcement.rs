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
///
/// `NOSTR_PRIVATE_KEY`/`BUZZ_PRIVATE_KEY` are scrubbed so `git-sign-nostr` signs
/// from the manifest's `nostr.keyfile` — the real agent-runtime child has the
/// private key env removed, and leaving the runner's ambient key set would make
/// the signer load the wrong identity (a non-hermetic test). `BUZZ_AUTH_TAG` is
/// scrubbed so the signer skips NIP-OA owner attestation (no relay to verify
/// against offline); signing itself needs no network.
fn wrapper(path: &str, cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
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
        email,
        "commit must be authored as the agent identity, not the repo-local human"
    );
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
    let id = buzz_git_identity::write_keyfile(keydir.path(), &nsec).expect("write keyfile");
    let expected_email = buzz_git_identity::derive_git_email(&id.pubkey_hex);

    let shim = tempfile::tempdir().unwrap();
    for name in ["git", "git-sign-nostr"] {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_buzz-acp"), shim.path().join(name)).unwrap();
    }
    let entries = buzz_git_identity::identity_signing_entries(&id);
    buzz_git_identity::write_identity_manifest(shim.path(), &entries).unwrap();

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
        assert!(Command::new("git")
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
    let out = Command::new("git")
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
    let id_b = buzz_git_identity::write_keyfile(keydir_b.path(), &nsec_b).expect("write B keyfile");

    // Create a commit authored as agent A but signed with key B, bypassing the
    // wrapper's `enforce` by invoking the real git binary directly with B's
    // signing config. `git-sign-nostr` resolves from the shim on PATH.
    let real_git = real_git_dir().join("git");
    let out = Command::new(&real_git)
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
    let author = Command::new(&real_git)
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

/// I4: the spawn-path wiring — `AcpClient::spawn` → `install_git_identity` —
/// must actually install the wrapper + manifest onto the agent-runtime child.
///
/// The tests above wire their own shim + `.git-identity` manifest, so they stay
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
    assert!(Command::new("git")
        .args(["init", "-q", "--bare", decoy.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["init", "-q", "--bare", actual.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    // Seed decoy with the human HEAD commit.
    // ls-remote on decoy returns HEAD's IDs; without the guard those IDs
    // exempt HEAD and the push succeeds to actual.
    assert!(Command::new("git")
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
    let refs = Command::new("git")
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
    let refs2 = Command::new("git")
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

    assert!(Command::new("git")
        .args(["init", "-q", "--bare", decoy.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["init", "-q", "--bare", actual.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());

    // Seed decoy with HEAD so its IDs would exempt the commit under bypass.
    assert!(Command::new("git")
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
    let refs = Command::new("git")
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
    let refs = Command::new("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}

/// R9/P5 real-wrapper regression: `alias.whatchanged = -p push origin main`
/// must be expanded and held to policy on binaries where `--list-cmds=deprecated`
/// succeeds and lists `whatchanged` (i.e. deprecated builtins are alias-first).
///
/// **Bypass shape (without the fix):**
/// When `!deprecated.contains(name)` is removed, `verify_alias_safety` breaks
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
    let probe = Command::new("git")
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
    let refs = Command::new("git")
        .args(["-C", remote.path().to_str().unwrap(), "for-each-ref"])
        .output()
        .unwrap();
    assert!(
        refs.stdout.is_empty(),
        "remote must be empty after refused push via whatchanged alias; refs={}",
        String::from_utf8_lossy(&refs.stdout),
    );
}
