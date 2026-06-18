//! End-to-end git-over-object-storage tests.
//!
//! Drives the real `git` binary (clone / push / fetch / force-push / tag,
//! plus a best-effort concurrent push race) against a running relay backed by
//! S3/MinIO, exercising the full manifest-pointer CAS commit path described in
//! `docs/git-on-object-storage.md`.
//!
//! Requires: relay at localhost:3000 with git + S3/MinIO configured, `git` on
//! PATH, and the `git-credential-nostr` helper built. All tests are `#[ignore]`
//! so they don't run in CI by default.
//!
//! # Running
//!
//! ```text
//! cargo build --release -p git-credential-nostr
//! GIT_CREDENTIAL_NOSTR_BIN=$PWD/target/release/git-credential-nostr \
//!   cargo test -p buzz-test-client --test e2e_git -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nostr::{EventBuilder, Keys, Kind, Tag};
use s3::creds::Credentials;
use s3::{Bucket, Region};

fn relay_http_url() -> String {
    std::env::var("RELAY_HTTP_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

/// Path to the compiled credential helper. Defaults to the workspace release
/// build; override with `GIT_CREDENTIAL_NOSTR_BIN`.
fn credential_helper() -> PathBuf {
    if let Ok(p) = std::env::var("GIT_CREDENTIAL_NOSTR_BIN") {
        return PathBuf::from(p);
    }
    // tests run from the crate dir; the workspace target is two levels up.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target/release/git-credential-nostr");
    p
}

/// Submit a signed event to the relay's REST bridge (`POST /api/events`).
async fn post_event(event: &nostr::Event) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", event.pubkey.to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("post event");
    assert!(
        resp.status().is_success(),
        "event rejected: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// Run `git` with the Buzz credential helper and isolated config.
fn git_status(args: &[&str], cwd: &Path, owner_nsec: &str) -> std::process::Output {
    let helper = credential_helper();
    Command::new("git")
        .args([
            "-c",
            "credential.useHttpPath=true",
            "-c",
            &format!("credential.helper={}", helper.display()),
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "user.name=E2E",
            "-c",
            "user.email=e2e@example.com",
        ])
        .args(args)
        .current_dir(cwd)
        // Isolate from any machine/agent git config (signing, etc.).
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env("NOSTR_PRIVATE_KEY", owner_nsec)
        .output()
        .expect("spawn git")
}

/// Run `git` with the Buzz credential helper and isolated config. Asserts the
/// command succeeds; returns stdout.
fn git(args: &[&str], cwd: &Path, owner_nsec: &str) -> String {
    let out = git_status(args, cwd, owner_nsec);
    assert!(
        out.status.success(),
        "git {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct GitS3Probe {
    bucket: Box<Bucket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PointerSnapshot {
    etag: String,
    digest: String,
}

impl GitS3Probe {
    fn from_env() -> Self {
        let endpoint = std::env::var("BUZZ_GIT_S3_ENDPOINT")
            .or_else(|_| std::env::var("BUZZ_S3_ENDPOINT"))
            .unwrap_or_else(|_| "http://localhost:9000".to_string());
        let access_key = std::env::var("BUZZ_GIT_S3_ACCESS_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_ACCESS_KEY"))
            .unwrap_or_else(|_| "buzz_dev".to_string());
        let secret_key = std::env::var("BUZZ_GIT_S3_SECRET_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_SECRET_KEY"))
            .unwrap_or_else(|_| "buzz_dev_secret".to_string());
        let bucket = std::env::var("BUZZ_GIT_S3_BUCKET")
            .or_else(|_| std::env::var("BUZZ_S3_BUCKET"))
            .unwrap_or_else(|_| "buzz-media".to_string());

        let region = Region::Custom {
            region: "us-east-1".into(),
            endpoint,
        };
        let creds = Credentials::new(Some(&access_key), Some(&secret_key), None, None, None)
            .expect("S3 credentials");
        let bucket = Bucket::new(&bucket, region, creds)
            .expect("S3 bucket")
            .with_path_style();
        Self { bucket }
    }

    fn pointer_key(owner: &str, repo: &str) -> String {
        let repo = repo.strip_suffix(".git").unwrap_or(repo);
        format!("repos/{owner}/{repo}/pointer")
    }

    async fn pointer(&self, owner: &str, repo: &str) -> Option<PointerSnapshot> {
        let key = Self::pointer_key(owner, repo);
        match self.bucket.get_object(&key).await {
            Ok(resp) => {
                let etag = resp
                    .headers()
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
                    .map(|(_, v)| v.to_string())
                    .expect("pointer GET must include ETag");
                let digest = String::from_utf8(resp.to_vec()).expect("pointer body utf-8");
                assert_eq!(digest.len(), 64, "pointer body is manifest digest");
                assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
                Some(PointerSnapshot { etag, digest })
            }
            Err(s3::error::S3Error::HttpFailWithBody(404, _)) => None,
            Err(e) => panic!("GET S3 pointer {key} failed: {e}"),
        }
    }

    async fn require_pointer(&self, owner: &str, repo: &str) -> PointerSnapshot {
        for _ in 0..40 {
            if let Some(p) = self.pointer(owner, repo).await {
                self.assert_manifest_exists(&p.digest).await;
                return p;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!(
            "S3 manifest pointer {} never appeared; git may have fallen back to disk",
            Self::pointer_key(owner, repo)
        );
    }

    async fn assert_manifest_exists(&self, digest: &str) {
        let key = format!("manifests/{digest}");
        match self.bucket.get_object(&key).await {
            Ok(_) => {}
            Err(e) => panic!("pointer named manifest {key}, but GET failed: {e}"),
        }
    }
}

#[tokio::test]
#[ignore = "requires live relay + MinIO + git"]
async fn git_clone_push_fetch_force_roundtrip() {
    use nostr::ToBech32;

    let owner = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let owner_nsec = owner.secret_key().to_bech32().unwrap();
    let repo = format!("e2e-git-{}", std::process::id());
    let s3 = GitS3Probe::from_env();

    // Announce the repo (kind:30617) so the relay creates the bare repo + hook.
    let announce = EventBuilder::new(Kind::from(30617), "")
        .tags(vec![
            Tag::parse(["d", &repo]).unwrap(),
            Tag::parse(["name", "e2e git repo"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    post_event(&announce).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let tmp = tempdir();
    let url = format!("{}/git/{}/{}", relay_http_url(), owner_hex, repo);

    // 1. Clone the empty repo.
    git(
        &["clone", "--quiet", &url, "clone1"],
        tmp.path(),
        &owner_nsec,
    );
    let clone1 = tmp.path().join("clone1");
    assert!(clone1.exists(), "clone1 created");
    let empty_pointer = s3.require_pointer(&owner_hex, &repo).await;

    // 2. Push an initial commit.
    std::fs::write(clone1.join("README.md"), "hello\n").unwrap();
    git(&["add", "."], &clone1, &owner_nsec);
    git(
        &["commit", "--quiet", "-m", "initial"],
        &clone1,
        &owner_nsec,
    );
    git(&["branch", "-M", "main"], &clone1, &owner_nsec);
    git(&["push", "--quiet", "origin", "main"], &clone1, &owner_nsec);
    let p1 = s3.require_pointer(&owner_hex, &repo).await;
    assert_ne!(
        p1, empty_pointer,
        "initial push must advance S3 manifest pointer"
    );
    let sha1 = git(&["rev-parse", "main"], &clone1, &owner_nsec)
        .trim()
        .to_string();

    // 3. A fresh clone observes the pushed content and exact SHA.
    git(
        &["clone", "--quiet", &url, "clone2"],
        tmp.path(),
        &owner_nsec,
    );
    let clone2 = tmp.path().join("clone2");
    assert_eq!(
        std::fs::read_to_string(clone2.join("README.md")).unwrap(),
        "hello\n",
        "fresh clone sees pushed content"
    );
    assert_eq!(
        git(&["rev-parse", "main"], &clone2, &owner_nsec).trim(),
        sha1,
        "fresh clone main == pushed SHA"
    );

    // 4. Second commit, push, pull into the other clone.
    std::fs::write(clone1.join("README.md"), "hello\nmore\n").unwrap();
    git(
        &["commit", "--quiet", "-am", "second"],
        &clone1,
        &owner_nsec,
    );
    git(&["push", "--quiet", "origin", "main"], &clone1, &owner_nsec);
    let p2 = s3.require_pointer(&owner_hex, &repo).await;
    assert_ne!(p2, p1, "second push must advance S3 manifest pointer");
    let sha2 = git(&["rev-parse", "main"], &clone1, &owner_nsec)
        .trim()
        .to_string();
    git(&["pull", "--quiet", "origin", "main"], &clone2, &owner_nsec);
    assert_eq!(
        git(&["rev-parse", "main"], &clone2, &owner_nsec).trim(),
        sha2,
        "clone2 fetched second commit"
    );

    // 5. Force-push a rewritten history.
    git(&["reset", "--quiet", "--hard", &sha1], &clone1, &owner_nsec);
    std::fs::write(clone1.join("README.md"), "forced\n").unwrap();
    git(
        &["commit", "--quiet", "-am", "forced"],
        &clone1,
        &owner_nsec,
    );
    let sha_f = git(&["rev-parse", "main"], &clone1, &owner_nsec)
        .trim()
        .to_string();
    git(
        &["push", "--quiet", "--force", "origin", "main"],
        &clone1,
        &owner_nsec,
    );
    let p3 = s3.require_pointer(&owner_hex, &repo).await;
    assert_ne!(p3, p2, "force push must advance S3 manifest pointer");
    assert_ne!(sha_f, sha2);

    // 6. A new clone after the force-push gets the rewritten history.
    git(
        &["clone", "--quiet", &url, "clone3"],
        tmp.path(),
        &owner_nsec,
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("clone3/README.md")).unwrap(),
        "forced\n",
        "clone3 has force-pushed content"
    );

    // 7. Tag push survives the round-trip.
    git(&["tag", "v1.0"], &clone1, &owner_nsec);
    git(&["push", "--quiet", "origin", "v1.0"], &clone1, &owner_nsec);
    let p4 = s3.require_pointer(&owner_hex, &repo).await;
    assert_ne!(p4, p3, "tag push must advance S3 manifest pointer");
    git(
        &["clone", "--quiet", &url, "clone4"],
        tmp.path(),
        &owner_nsec,
    );
    let tags = git(&["tag"], &tmp.path().join("clone4"), &owner_nsec);
    assert!(tags.contains("v1.0"), "tag v1.0 cloned back: {tags}");
}

#[tokio::test]
#[ignore = "requires live relay + MinIO + git"]
async fn git_concurrent_push_one_wins_and_repo_recovers() {
    use nostr::ToBech32;

    let owner = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let owner_nsec = owner.secret_key().to_bech32().unwrap();
    let repo = format!("e2e-git-concurrent-{}", std::process::id());
    let s3 = GitS3Probe::from_env();

    let announce = EventBuilder::new(Kind::from(30617), "")
        .tags(vec![
            Tag::parse(["d", &repo]).unwrap(),
            Tag::parse(["name", "e2e concurrent git repo"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    post_event(&announce).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let tmp = tempdir_named("buzz-e2e-git-concurrent");
    let url = format!("{}/git/{}/{}", relay_http_url(), owner_hex, repo);

    git(&["clone", "--quiet", &url, "seed"], tmp.path(), &owner_nsec);
    let seed = tmp.path().join("seed");
    std::fs::write(seed.join("README.md"), "base\n").unwrap();
    git(&["add", "."], &seed, &owner_nsec);
    git(&["commit", "--quiet", "-m", "base"], &seed, &owner_nsec);
    git(&["branch", "-M", "main"], &seed, &owner_nsec);
    git(&["push", "--quiet", "origin", "main"], &seed, &owner_nsec);
    let base_pointer = s3.require_pointer(&owner_hex, &repo).await;
    let base_sha = git(&["rev-parse", "main"], &seed, &owner_nsec)
        .trim()
        .to_string();

    let contenders = 8usize;
    let mut contenders_info = Vec::new();
    for i in 0..contenders {
        let dir = format!("c{i}");
        git(&["clone", "--quiet", &url, &dir], tmp.path(), &owner_nsec);
        let worktree = tmp.path().join(&dir);
        std::fs::write(
            worktree.join(format!("file-{i}.txt")),
            format!("winner? {i}\n"),
        )
        .unwrap();
        git(&["add", "."], &worktree, &owner_nsec);
        git(
            &["commit", "--quiet", "-m", &format!("contender {i}")],
            &worktree,
            &owner_nsec,
        );
        let sha = git(&["rev-parse", "main"], &worktree, &owner_nsec)
            .trim()
            .to_string();
        contenders_info.push((i, sha));
    }

    let mut children = Vec::new();
    for i in 0..contenders {
        let worktree = tmp.path().join(format!("c{i}"));
        let owner_nsec = owner_nsec.clone();
        children.push(std::thread::spawn(move || {
            git_status(
                &["push", "--quiet", "origin", "main"],
                &worktree,
                &owner_nsec,
            )
        }));
    }

    let mut successes = Vec::new();
    let mut failures = 0usize;
    for (i, child) in children.into_iter().enumerate() {
        let out = child.join().expect("push thread panicked");
        if out.status.success() {
            successes.push(i);
        } else {
            failures += 1;
        }
    }
    assert_eq!(successes.len(), 1, "exactly one concurrent push should win");
    assert_eq!(failures, contenders - 1, "the rest should lose cleanly");
    let winner_index = successes[0];
    let winner_sha = contenders_info
        .iter()
        .find_map(|(i, sha)| (*i == winner_index).then_some(sha.clone()))
        .expect("winner sha recorded");

    let after_pointer = s3.require_pointer(&owner_hex, &repo).await;
    assert_ne!(
        after_pointer, base_pointer,
        "winning push must advance S3 manifest pointer"
    );

    git(
        &["clone", "--quiet", &url, "after"],
        tmp.path(),
        &owner_nsec,
    );
    let after = tmp.path().join("after");
    let after_sha = git(&["rev-parse", "main"], &after, &owner_nsec)
        .trim()
        .to_string();
    assert_ne!(after_sha, base_sha, "winner advanced main");
    assert_eq!(
        after_sha, winner_sha,
        "published head must equal the successful contender's tip"
    );
    let log = git(
        &["log", "--oneline", "--decorate", "-1"],
        &after,
        &owner_nsec,
    );
    assert!(
        log.contains("contender"),
        "published head is one contender: {log}"
    );
}

// ── tiny tempdir (avoid an extra dep) ─────────────────────────────────────────

struct TempDir(PathBuf);
impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tempdir() -> TempDir {
    tempdir_named("buzz-e2e-git")
}

fn tempdir_named(prefix: &str) -> TempDir {
    let mut p = std::env::temp_dir();
    p.push(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    TempDir(p)
}
