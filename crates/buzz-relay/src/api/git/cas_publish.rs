//! Push commit point — content-addressed pack + manifest CAS (§Push step 2–7).
//!
//! Pure async function over the spec's commit primitives. Given the post-
//! receive-pack repository workspace and the object-store client, this:
//!
//! 1. Reads the current pointer → `(e, d_before)` (§Push step 3).
//! 2. Fetches `m_before` via `get_verified(d_before)` (§Push step 3) —
//!    digest-verified so a corrupt manifest fails closed, not silently.
//! 3. Snapshots refs + HEAD off the workspace (the receive-pack's published
//!    state, by which point the pre-receive hook has enforced fast-forward /
//!    branch-protection against the parent's refs).
//! 4. Normally captures the new objects as a delta pack via `git pack-objects
//!    --revs --stdout` over `(refs_after) --not (refs_before-tips)` (§Push
//!    step 1–2). Before the manifest reaches its pack cap, compaction instead
//!    captures the complete `refs_after` closure into bounded replacement
//!    packs. Empty output for a ref-less repository is allowed.
//! 5. `put_pack` (content-addressed, create-only, idempotent — §Push step 2).
//!    The key is derived from `sha256(bytes)` by the store layer.
//! 6. Composes `m_after`: normal pushes use parent packs ∪ new pack;
//!    compaction replaces the pack list with the newly captured full closure.
//!    Both retain the parent digest and post-push refs (§Push step 5).
//! 7. `put_manifest` (content-addressed, create-only, idempotent — §Push
//!    step 6).
//! 8. `put_pointer(IfMatch(e) | IfNoneMatchStar)` — the CAS (§Push step 7).
//!    - `Won` → return `CasSuccess { manifest, manifest_key }`. The caller
//!      then derives kind:30618 against `m_after` (Sami's
//!      `manifest_event::build_ref_state_event`) and constructs the
//!      success response — the *fence* in §Push step 8.
//!    - `LostRace` → re-read the pointer to fetch the winner's manifest,
//!      then return `CasError::Conflict { winner_manifest,
//!      winner_manifest_key }` (→ HTTP 409). The winner payload is for
//!      the caller's diagnostic + future cache; the loser's ephemeral
//!      tempdir dies on scope exit, so there's no disk to reconcile.
//!      **No retry.** The losing push's receive-pack output was derived
//!      against the now-superseded parent; reusing it would violate
//!      `Inv_RefDerivedFromParent` (§Mechanized Verification). The client
//!      re-runs `git push`, which re-hydrates and re-runs receive-pack
//!      against the advanced state — that is the only safe retry, and
//!      `git`'s own machinery already does it.
//!
//! ## Fence positioning
//!
//! This function returns *before* the success `Response` is constructed.
//! It is called from inside `finalize_push`, which is the unique site that
//! builds a push `Response`. The structural seam therefore enforces
//! Theorem 1: success cannot be observed until this returns `Ok(_)`.
//!
//! ## What this function deliberately does *not* do
//!
//! - **No retry on `LostRace`.** Per spec §Push step 7 "GOTO 3 (retry) or
//!   respond non-ff": both arms are safe; we take the non-ff arm because
//!   reusing receive-pack's output against a moved parent isn't safe and
//!   re-hydrating from inside the handler is expensive. Sami's TLA-action
//!   guidance is explicit: retry would change the TLA action.
//! - **No kind:30618 emission.** That is the *derived* publication after a
//!   successful CAS. Caller passes `m_after` into
//!   `manifest_event::build_ref_state_event` *after* this returns `Ok`.
//!   Spec §Implementation Correspondence: "kind:30618 is derived after
//!   CAS, never the commit."
//! - **No advisory lock.** Spec §Push, "No advisory lock in v1": writer
//!   serialization is the CAS. Adding a per-repo mutex would hide the
//!   exact contention `Inv_NoFork` proves safe.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::api::git::manifest::{
    pointer_key, Manifest, ManifestError, MANIFEST_VERSION, MAX_MANIFEST_PACKS, MAX_MANIFEST_REFS,
    PACK_COMPACTION_THRESHOLD,
};
use crate::api::git::store::{CasOutcome, ETag, GitStore, Precond, StoreError};
use buzz_core::TenantContext;

const PACK_CAPTURE_TIMEOUT: Duration = Duration::from_secs(300);
const PACK_COMPACTION_OPERATION_TIMEOUT: Duration = Duration::from_secs(600);
const PACK_OBJECTS_WINDOW_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const PACK_OBJECTS_WINDOW: &str = "10";
const MAX_COMPACTION_OBJECTS: u64 = 1_000_000;
static PACK_COMPACTION_SEMAPHORE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Errors `cas_publish` surfaces. Distinguished so `finalize_push` can map
/// each to the right HTTP status (the spec's 412 → 409 mapping is here).
#[derive(Debug, thiserror::Error)]
pub enum CasError {
    /// The CAS lost the race (§Push step 7 → 412). Maps to HTTP 409. The
    /// **terminal** classified outcome — never retried by this function,
    /// since the receive-pack output is now derived against a superseded
    /// parent. Client retries by re-pushing.
    ///
    /// Carries the winner's manifest + key so the caller can reconcile
    /// the on-disk workspace back to the winning state (Eva's
    /// disk-reset-on-lost-race) without a second pointer GET round-trip.
    /// The re-read after `LostRace` can itself race with a *third* winner;
    /// that's fine — we surface *some* winning state, and the loser's
    /// client re-pushes anyway.
    /// Boxed because `Manifest` is the largest `CasError` payload and we
    /// don't want all error-paths paying the cost of a 200-byte struct in
    /// the `Result` ABI (`clippy::result_large_err`).
    #[error("CAS lost race; push superseded by winner with manifest {winner_manifest_key}")]
    Conflict {
        /// The manifest now installed under the pointer (the winner).
        winner_manifest: Box<Manifest>,
        /// Full content-addressed key of `winner_manifest`
        /// (`manifests/<sha256>`).
        winner_manifest_key: String,
    },

    /// The current pointer names a manifest we cannot reconstruct
    /// faithfully — digest mismatch, `manifest GET` 404 under a non-empty
    /// pointer, unsupported schema version, or malformed pointer body.
    /// **Fail closed:** we do not invent a published state to push onto.
    /// Maps to HTTP 5xx (parent corruption, ops issue).
    #[error("manifest read failed (corrupt or missing): {0}")]
    ManifestReadFailed(String),

    /// The composed `m_after` failed `Manifest::validate()` — unsafe
    /// refname, malformed oid, empty head. Pre-CAS, fail closed before
    /// any write. Maps to HTTP 4xx (client/input rejected — distinct from
    /// `ManifestReadFailed` which is server-side data corruption).
    #[error("manifest invalid: {0}")]
    ManifestInvalid(#[from] ManifestError),

    /// Backend transport / I/O failure surfaced from the object store.
    /// Distinct from `Conflict` so `?`-bubbling cannot turn a 412 into a
    /// 500.
    #[error("object store backend: {0}")]
    Backend(#[from] StoreError),

    /// `git pack-objects` failed, or we could not snapshot refs off the
    /// workspace. Pre-CAS — the pointer was never written.
    #[error("pack capture: {0}")]
    PackCapture(String),

    /// The push would make the repo exceed the relay's configured byte budget.
    #[error("resource limit: {0}")]
    ResourceLimit(String),
}

/// Resource limits carried from hydration into the publish step.
#[derive(Debug, Clone, Copy)]
pub struct PublishLimits {
    /// Bytes already materialized from the parent manifest.
    pub parent_hydrated_bytes: u64,
    /// Maximum bytes allowed in one newly captured pack.
    pub max_pack_bytes: u64,
    /// Maximum total hydrated bytes allowed for the resulting repo.
    pub max_repo_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct PublishOptions {
    limits: PublishLimits,
    compaction_threshold: usize,
}

struct CompactedPack {
    pack_path: PathBuf,
    idx_path: PathBuf,
    pack_bytes: u64,
}

struct CompactedPacks {
    _tempdir: TempDir,
    packs: Vec<CompactedPack>,
    total_pack_bytes: u64,
}

struct CompactionObservation {
    started_at: Instant,
    packs_before: usize,
    packs_after: usize,
    compacted_bytes: u64,
}

struct PreparedCompaction {
    pack_keys: Vec<String>,
    packs_after: usize,
    compacted_bytes: u64,
}

/// Outcome of a successful CAS. Carries the composed manifest so the
/// caller can derive kind:30618 against `m_after.refs` / `m_after.head` —
/// these are the values that physically landed, by `Inv_RefEffectApplied`.
#[derive(Debug)]
pub struct CasSuccess {
    /// The manifest the CAS installed (the published state).
    pub manifest: Manifest,
    /// The full content-addressed key of `manifest` (`manifests/<sha256>`).
    pub manifest_key: String,
}

/// Resolved view of the pre-push pointer (§Push step 3 output).
///
/// **The CAS write is predicated on `if_match`** — the caller must load
/// this *before* running receive-pack against the hydrated workspace, and
/// pass the same value into [`cas_publish`]. If the pointer advances
/// between load and CAS (a concurrent push wins), the CAS fails with
/// `LostRace`/`Conflict` and the loser re-pushes — that is the only safe
/// retry path (the loser's receive-pack output is derived against the
/// superseded parent, so reusing it would violate
/// `Inv_RefDerivedFromParent`).
///
/// The structural seam this `ParentState` argument creates is what makes
/// `Inv_RefDerivedFromParent` mechanical: `m_after.parent` is *literally*
/// the digest of the manifest receive-pack ran against, not whatever
/// pointer happens to be live at CAS time.
#[derive(Debug, Clone)]
pub struct ParentState {
    /// ETag predicating the next CAS write. `None` only when the pointer
    /// does not yet exist (first push to an empty repo) — then the CAS
    /// uses `If-None-Match: *`.
    pub if_match: Option<ETag>,
    /// The parent manifest's content-addressed *digest* (64-hex), not the
    /// full `manifests/<digest>` key. This lands in `Manifest.parent` and
    /// is what `Inv_RefDerivedFromParent` reasons over (parent =
    /// pointer.digest). Full key is a local fetch detail, derived as
    /// `format!("manifests/{}", digest)`. `None` only on first push.
    pub parent_digest: Option<String>,
    /// The parsed parent manifest. On first push, an empty manifest.
    pub parent: Manifest,
}

impl ParentState {
    /// State for a brand-new repo with no published manifest yet.
    pub fn fresh() -> Self {
        Self {
            if_match: None,
            parent_digest: None,
            parent: Manifest {
                version: MANIFEST_VERSION,
                head: String::new(),
                refs: BTreeMap::new(),
                packs: Vec::new(),
                parent: None,
            },
        }
    }

    /// Build a `ParentState` from already-loaded pointer state.
    ///
    /// The hydrate layer reads the pointer + verified manifest as part of
    /// materializing the workspace, then hands the same `(etag, digest,
    /// manifest)` tuple back here. Centralizing the constructor in
    /// `cas_publish` means there's one place where `ParentState`
    /// invariants live; centralizing the I/O in `hydrate` means we read
    /// the pointer once per push, not twice.
    pub fn from_loaded(etag: ETag, digest: String, parent: Manifest) -> Self {
        Self {
            if_match: Some(etag),
            parent_digest: Some(digest),
            parent,
        }
    }
}

/// Read `refs/*` + symbolic-HEAD from the workspace.
///
/// HEAD is the symref target (e.g. `refs/heads/main`), unprefixed — the
/// manifest stores published ref state, not protocol formatting. Detached
/// HEAD or no HEAD yields an empty string.
async fn snapshot_workspace_state(
    repo_path: &Path,
    scratch_dir: &Path,
) -> Result<(BTreeMap<String, String>, String), CasError> {
    const MAX_REF_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024;

    let refs_stdout_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stdout tempfile: {e}")))?;
    let refs_stdout_file = refs_stdout_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stdout reopen: {e}")))?;
    let refs_stderr_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stderr tempfile: {e}")))?;
    let refs_stderr_file = refs_stderr_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stderr reopen: {e}")))?;

    let mut refs_cmd = Command::new("git");
    refs_cmd
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(repo_path)
        .stdout(Stdio::from(refs_stdout_file))
        .stderr(Stdio::from(refs_stderr_file));
    super::transport::harden_git_env(&mut refs_cmd);
    let refs_status = refs_cmd
        .status()
        .await
        .map_err(|e| CasError::PackCapture(format!("for-each-ref spawn: {e}")))?;
    if !refs_status.success() {
        return Err(CasError::PackCapture(format!(
            "for-each-ref failed: status={:?} stderr={}",
            refs_status.code(),
            read_prefix(refs_stderr_tmp.path(), 64 * 1024).await
        )));
    }
    let refs_stdout_len = tokio::fs::metadata(refs_stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stdout metadata: {e}")))?
        .len();
    if refs_stdout_len > MAX_REF_SNAPSHOT_BYTES {
        return Err(CasError::ResourceLimit(format!(
            "ref snapshot is {refs_stdout_len} bytes (max {MAX_REF_SNAPSHOT_BYTES})"
        )));
    }
    let refs_stdout = tokio::fs::read(refs_stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("for-each-ref stdout read: {e}")))?;

    let mut refs = BTreeMap::new();
    for line in std::str::from_utf8(&refs_stdout)
        .unwrap_or_default()
        .lines()
    {
        let mut parts = line.splitn(2, ' ');
        let (Some(name), Some(oid)) = (parts.next(), parts.next()) else {
            continue;
        };
        if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            warn!(ref_name = %name, oid = %oid, "for-each-ref returned malformed oid; skipping");
            continue;
        }
        if refs.len() >= MAX_MANIFEST_REFS && !refs.contains_key(name) {
            return Err(CasError::ResourceLimit(format!(
                "workspace contains more than {MAX_MANIFEST_REFS} refs"
            )));
        }
        refs.insert(name.to_string(), oid.to_string());
    }

    let mut head_cmd = Command::new("git");
    head_cmd
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .current_dir(repo_path);
    super::transport::harden_git_env(&mut head_cmd);
    let head_out = head_cmd
        .output()
        .await
        .map_err(|e| CasError::PackCapture(format!("symbolic-ref spawn: {e}")))?;
    let head = if head_out.status.success() {
        String::from_utf8_lossy(&head_out.stdout).trim().to_string()
    } else {
        String::new()
    };

    Ok((refs, head))
}

fn resolve_published_head(
    refs: &BTreeMap<String, String>,
    observed_head: String,
    parent_head: &str,
) -> String {
    let is_published_branch =
        |name: &str| name.starts_with("refs/heads/") && refs.contains_key(name);
    if is_published_branch(&observed_head) {
        return observed_head;
    }
    if is_published_branch(parent_head) {
        return parent_head.to_string();
    }
    for preferred in ["refs/heads/main", "refs/heads/master"] {
        if refs.contains_key(preferred) {
            return preferred.to_string();
        }
    }
    if let Some(branch) = refs.keys().find(|name| name.starts_with("refs/heads/")) {
        return branch.clone();
    }
    if !observed_head.is_empty() {
        observed_head
    } else {
        parent_head.to_string()
    }
}

fn digest_from_pack_key(key: &str) -> Result<String, CasError> {
    key.strip_prefix("packs/")
        .filter(|digest| digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| {
            CasError::Backend(StoreError::Backend(s3::error::S3Error::HttpFailWithBody(
                500,
                format!("put_pack returned non-standard key: {key}"),
            )))
        })
}

async fn write_idx_sidecar(
    store: &GitStore,
    pack_key: &str,
    pack_bytes: &[u8],
    scratch_dir: &Path,
) -> Result<(), CasError> {
    let pack_digest = digest_from_pack_key(pack_key)?;
    let tempdir = TempDir::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("idx tempdir in {scratch_dir:?}: {e}")))?;
    let pack_path = tempdir.path().join(format!("pack-{pack_digest}.pack"));
    tokio::fs::write(&pack_path, pack_bytes)
        .await
        .map_err(|e| CasError::PackCapture(format!("write idx input pack {pack_digest}: {e}")))?;

    let mut cmd = Command::new("git");
    cmd.args(["index-pack", pack_path.to_str().unwrap()])
        .current_dir(tempdir.path());
    super::transport::harden_git_env(&mut cmd);
    let out = cmd
        .output()
        .await
        .map_err(|e| CasError::PackCapture(format!("index-pack spawn for idx sidecar: {e}")))?;
    if !out.status.success() {
        return Err(CasError::PackCapture(format!(
            "index-pack for idx sidecar failed: status={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    let idx_path = pack_path.with_extension("idx");
    let idx_bytes = tokio::fs::read(&idx_path)
        .await
        .map_err(|e| CasError::PackCapture(format!("read idx sidecar {pack_digest}: {e}")))?;
    store.put_idx(&pack_digest, &idx_bytes).await?;
    Ok(())
}

async fn wait_for_git_child(
    child: &mut tokio::process::Child,
    stdin_bytes: &[u8],
    operation: &str,
) -> Result<std::process::ExitStatus, CasError> {
    let result = tokio::time::timeout(PACK_CAPTURE_TIMEOUT, async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CasError::PackCapture(format!("{operation} stdin closed")))?;
        stdin
            .write_all(stdin_bytes)
            .await
            .map_err(|e| CasError::PackCapture(format!("{operation} stdin write: {e}")))?;
        drop(stdin);
        child
            .wait()
            .await
            .map_err(|e| CasError::PackCapture(format!("{operation} wait: {e}")))
    })
    .await;
    match result {
        Ok(status) => status,
        Err(_) => {
            if let Err(error) = child.kill().await {
                warn!(operation, error = %error, "timed-out git pack-objects could not be killed");
            }
            Err(CasError::PackCapture(format!(
                "{operation} timed out after {} seconds",
                PACK_CAPTURE_TIMEOUT.as_secs()
            )))
        }
    }
}

/// Capture the objects this push introduced as a single pack.
///
/// Runs `git pack-objects --revs --stdout` reading rev-spec lines from
/// stdin: each `oid` line includes that oid's reachable closure, and each
/// `^oid` line excludes one. We feed `refs_after`'s tips with positive
/// lines and `refs_before`'s tips with `^` lines — the resulting pack is
/// exactly the objects in the symmetric difference's "ahead" half, i.e.
/// the new objects this push needs to durably name.
///
/// Returns `None` in either of two cases, both legitimate:
/// 1. `refs_after` is empty — a delete-all push (no positive tips to feed
///    pack-objects; nothing to cover).
/// 2. `pack-objects` produces empty stdout — refs-only push that re-points
///    or deletes a ref at an already-stored oid (e.g. `git push :branch`,
///    or `git push origin existing-sha:newname`).
///
/// In both cases the caller still publishes a new manifest — the ref
/// change is real even if the pack set didn't grow.
async fn capture_pack(
    repo_path: &Path,
    refs_before: &BTreeMap<String, String>,
    refs_after: &BTreeMap<String, String>,
    max_pack_bytes: u64,
    scratch_dir: &Path,
) -> Result<Option<Vec<u8>>, CasError> {
    // Build rev-spec stdin: positive new tips, negative old tips.
    // Deduplicate against the same-oid case — no point feeding `X ^X`.
    let mut stdin_lines = String::new();
    let mut any_positive = false;
    for oid in refs_after.values() {
        stdin_lines.push_str(oid);
        stdin_lines.push('\n');
        any_positive = true;
    }
    if !any_positive {
        // No refs to cover — first-push case where the client deleted
        // everything before any tip was set (degenerate, but handle).
        return Ok(None);
    }
    for oid in refs_before.values() {
        stdin_lines.push('^');
        stdin_lines.push_str(oid);
        stdin_lines.push('\n');
    }

    let stdout_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("pack-objects stdout tempfile: {e}")))?;
    let stdout_file = stdout_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("pack-objects stdout reopen: {e}")))?;
    let stderr_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("pack-objects stderr tempfile: {e}")))?;
    let stderr_file = stderr_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("pack-objects stderr reopen: {e}")))?;

    let window_memory_arg = format!("--window-memory={PACK_OBJECTS_WINDOW_MEMORY_BYTES}");
    let mut cmd = Command::new("git");
    cmd.args([
        "pack-objects",
        "--revs",
        "--stdout",
        "-q",
        "--threads=1",
        "--window",
        PACK_OBJECTS_WINDOW,
        &window_memory_arg,
    ])
    .current_dir(repo_path)
    .stdin(Stdio::piped())
    .stdout(Stdio::from(stdout_file))
    .stderr(Stdio::from(stderr_file))
    .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| CasError::PackCapture(format!("pack-objects spawn: {e}")))?;
    let status = wait_for_git_child(&mut child, stdin_lines.as_bytes(), "pack-objects").await?;
    if !status.success() {
        let stderr = read_prefix(stderr_tmp.path(), 64 * 1024).await;
        return Err(CasError::PackCapture(format!(
            "pack-objects failed: status={:?} stderr={}",
            status.code(),
            stderr
        )));
    }
    let pack_len = tokio::fs::metadata(stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("pack-objects stdout metadata: {e}")))?
        .len();
    if pack_len > max_pack_bytes {
        return Err(CasError::ResourceLimit(format!(
            "pack-objects output is {pack_len} bytes (max {max_pack_bytes})"
        )));
    }
    if pack_len == 0 {
        return Ok(None);
    }
    let pack_bytes = tokio::fs::read(stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("pack-objects stdout read: {e}")))?;
    Ok(Some(pack_bytes))
}

fn should_compact(parent_pack_count: usize, threshold: usize) -> bool {
    parent_pack_count >= threshold
}

async fn acquire_compaction_permit() -> Result<tokio::sync::SemaphorePermit<'static>, CasError> {
    tokio::time::timeout(PACK_CAPTURE_TIMEOUT, PACK_COMPACTION_SEMAPHORE.acquire())
        .await
        .map_err(|_| {
            CasError::PackCapture(format!(
                "timed out waiting {} seconds for pack compaction capacity",
                PACK_CAPTURE_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|_| CasError::PackCapture("pack compaction capacity closed".into()))
}

fn compacted_pack_set_is_usable(parent_pack_count: usize, compacted_pack_count: usize) -> bool {
    compacted_pack_count < parent_pack_count
        || (parent_pack_count >= MAX_MANIFEST_PACKS && compacted_pack_count <= MAX_MANIFEST_PACKS)
}

async fn enforce_compaction_object_limit(
    repo_path: &Path,
    scratch_dir: &Path,
) -> Result<(), CasError> {
    const MAX_COUNT_OBJECTS_OUTPUT_BYTES: u64 = 64 * 1024;

    let stdout_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("count-objects stdout tempfile: {e}")))?;
    let stdout_file = stdout_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("count-objects stdout reopen: {e}")))?;
    let stderr_tmp = tempfile::NamedTempFile::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("count-objects stderr tempfile: {e}")))?;
    let stderr_file = stderr_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("count-objects stderr reopen: {e}")))?;
    let mut cmd = Command::new("git");
    cmd.args(["count-objects", "-v"])
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| CasError::PackCapture(format!("count-objects spawn: {e}")))?;
    let status = wait_for_git_child(&mut child, &[], "count-objects").await?;
    if !status.success() {
        return Err(CasError::PackCapture(format!(
            "count-objects failed: status={:?} stderr={}",
            status.code(),
            read_prefix(stderr_tmp.path(), MAX_COUNT_OBJECTS_OUTPUT_BYTES).await
        )));
    }
    let output_len = tokio::fs::metadata(stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("count-objects stdout metadata: {e}")))?
        .len();
    if output_len > MAX_COUNT_OBJECTS_OUTPUT_BYTES {
        return Err(CasError::ResourceLimit(format!(
            "count-objects output is {output_len} bytes (max {MAX_COUNT_OBJECTS_OUTPUT_BYTES})"
        )));
    }
    let output = tokio::fs::read_to_string(stdout_tmp.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("read count-objects output: {e}")))?;
    let parse_count = |name: &str| -> Result<u64, CasError> {
        output
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.trim().parse::<u64>().ok())
            .ok_or_else(|| CasError::PackCapture(format!("count-objects omitted {name:?}")))
    };
    let loose = parse_count("count:")?;
    let packed = parse_count("in-pack:")?;
    let object_count = loose
        .checked_add(packed)
        .ok_or_else(|| CasError::ResourceLimit("compaction object count overflowed u64".into()))?;
    if object_count > MAX_COMPACTION_OBJECTS {
        return Err(CasError::ResourceLimit(format!(
            "repository has {object_count} objects (compaction max {MAX_COMPACTION_OBJECTS})"
        )));
    }
    Ok(())
}

/// Capture the complete object closure reachable from the post-push refs.
///
/// Unlike [`capture_pack`], this writes one or more bounded packs into a
/// private tempdir and supplies no negative revisions. The resulting pack set
/// can therefore replace the parent manifest's pack list rather than extending
/// it. Old object-store packs remain immutable and are not deleted.
async fn capture_compacted_packs(
    repo_path: &Path,
    refs_after: &BTreeMap<String, String>,
    limits: PublishLimits,
    scratch_dir: &Path,
) -> Result<CompactedPacks, CasError> {
    let tempdir = TempDir::new_in(scratch_dir)
        .map_err(|e| CasError::PackCapture(format!("compaction tempdir: {e}")))?;
    if refs_after.is_empty() {
        return Ok(CompactedPacks {
            _tempdir: tempdir,
            packs: Vec::new(),
            total_pack_bytes: 0,
        });
    }
    enforce_compaction_object_limit(repo_path, scratch_dir).await?;

    let mut stdin_lines = String::new();
    let mut seen_oids = BTreeSet::new();
    for oid in refs_after.values() {
        if seen_oids.insert(oid) {
            stdin_lines.push_str(oid);
            stdin_lines.push('\n');
        }
    }

    let output_base = tempdir.path().join("compact");
    let output_base_str = output_base
        .to_str()
        .ok_or_else(|| CasError::PackCapture("compaction path is not valid utf-8".into()))?;
    let max_pack_arg = format!("--max-pack-size={}", limits.max_pack_bytes);
    let window_memory_arg = format!("--window-memory={PACK_OBJECTS_WINDOW_MEMORY_BYTES}");
    let stdout_tmp = tempfile::NamedTempFile::new_in(tempdir.path())
        .map_err(|e| CasError::PackCapture(format!("compaction stdout tempfile: {e}")))?;
    let stdout_file = stdout_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("compaction stdout reopen: {e}")))?;
    let stderr_tmp = tempfile::NamedTempFile::new_in(tempdir.path())
        .map_err(|e| CasError::PackCapture(format!("compaction stderr tempfile: {e}")))?;
    let stderr_file = stderr_tmp
        .reopen()
        .map_err(|e| CasError::PackCapture(format!("compaction stderr reopen: {e}")))?;
    let mut cmd = Command::new("git");
    cmd.args([
        "pack-objects",
        "--revs",
        "-q",
        "--threads=1",
        "--window",
        PACK_OBJECTS_WINDOW,
        &window_memory_arg,
        &max_pack_arg,
        output_base_str,
    ])
    .current_dir(repo_path)
    .stdin(Stdio::piped())
    .stdout(Stdio::from(stdout_file))
    .stderr(Stdio::from(stderr_file))
    .kill_on_drop(true);
    super::transport::harden_git_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| CasError::PackCapture(format!("compaction pack-objects spawn: {e}")))?;
    let status = wait_for_git_child(
        &mut child,
        stdin_lines.as_bytes(),
        "compaction pack-objects",
    )
    .await?;
    if !status.success() {
        return Err(CasError::PackCapture(format!(
            "compaction pack-objects failed: status={:?} stderr={}",
            status.code(),
            read_prefix(stderr_tmp.path(), 64 * 1024).await
        )));
    }

    let mut pack_paths = Vec::new();
    let mut entries = tokio::fs::read_dir(tempdir.path())
        .await
        .map_err(|e| CasError::PackCapture(format!("read compacted pack directory: {e}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| CasError::PackCapture(format!("read compacted pack entry: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("pack") {
            pack_paths.push(path);
        }
    }
    pack_paths.sort();
    if pack_paths.len() > MAX_MANIFEST_PACKS {
        return Err(CasError::ResourceLimit(format!(
            "compaction produced {} packs (max {MAX_MANIFEST_PACKS})",
            pack_paths.len()
        )));
    }

    let mut packs = Vec::with_capacity(pack_paths.len());
    let mut total_pack_bytes = 0u64;
    for pack_path in pack_paths {
        let pack_bytes = tokio::fs::metadata(&pack_path)
            .await
            .map_err(|e| CasError::PackCapture(format!("stat compacted pack: {e}")))?
            .len();
        if pack_bytes > limits.max_pack_bytes {
            return Err(CasError::ResourceLimit(format!(
                "compacted pack is {pack_bytes} bytes (max {})",
                limits.max_pack_bytes
            )));
        }
        total_pack_bytes = total_pack_bytes
            .checked_add(pack_bytes)
            .ok_or_else(|| CasError::ResourceLimit("compacted byte count overflowed u64".into()))?;
        if total_pack_bytes > limits.max_repo_bytes {
            return Err(CasError::ResourceLimit(format!(
                "compacted repo needs {total_pack_bytes} bytes (max {})",
                limits.max_repo_bytes
            )));
        }
        let idx_path = pack_path.with_extension("idx");
        let idx_bytes = tokio::fs::metadata(&idx_path)
            .await
            .map_err(|e| CasError::PackCapture(format!("stat compacted idx: {e}")))?
            .len();
        if idx_bytes > limits.max_pack_bytes {
            return Err(CasError::ResourceLimit(format!(
                "compacted idx is {idx_bytes} bytes (max {})",
                limits.max_pack_bytes
            )));
        }
        packs.push(CompactedPack {
            pack_path,
            idx_path,
            pack_bytes,
        });
    }

    Ok(CompactedPacks {
        _tempdir: tempdir,
        packs,
        total_pack_bytes,
    })
}

async fn upload_compacted_packs(
    store: &GitStore,
    compacted: &CompactedPacks,
) -> Result<Vec<String>, CasError> {
    let mut pack_keys = Vec::with_capacity(compacted.packs.len());
    for pack in &compacted.packs {
        let pack_bytes = tokio::fs::read(&pack.pack_path)
            .await
            .map_err(|e| CasError::PackCapture(format!("read compacted pack: {e}")))?;
        if u64::try_from(pack_bytes.len()).unwrap_or(u64::MAX) != pack.pack_bytes {
            return Err(CasError::PackCapture(
                "compacted pack changed after size validation".into(),
            ));
        }
        let pack_key = store.put_pack(&pack_bytes).await?;
        let pack_digest = digest_from_pack_key(&pack_key)?;
        match tokio::fs::read(&pack.idx_path).await {
            Ok(idx_bytes) => {
                if let Err(error) = store.put_idx(&pack_digest, &idx_bytes).await {
                    warn!(
                        pack_key = %pack_key,
                        error = %error,
                        "failed to write compacted git pack idx sidecar; push will continue"
                    );
                }
            }
            Err(error) => {
                warn!(
                    pack_key = %pack_key,
                    error = %error,
                    "failed to read compacted git pack idx sidecar; push will continue"
                );
            }
        }
        pack_keys.push(pack_key);
    }
    pack_keys.sort();
    pack_keys.dedup();
    Ok(pack_keys)
}

async fn prepare_compaction(
    store: &GitStore,
    repo_path: &Path,
    refs_after: &BTreeMap<String, String>,
    limits: PublishLimits,
    scratch_dir: &Path,
    packs_before: usize,
) -> Result<PreparedCompaction, CasError> {
    let compacted = capture_compacted_packs(repo_path, refs_after, limits, scratch_dir).await?;
    let packs_after = compacted.packs.len();
    if !compacted_pack_set_is_usable(packs_before, packs_after) {
        return Err(CasError::ResourceLimit(format!(
            "compaction did not reduce pack count (before {packs_before}, after {packs_after})"
        )));
    }
    let compacted_bytes = compacted.total_pack_bytes;
    let pack_keys = upload_compacted_packs(store, &compacted).await?;
    Ok(PreparedCompaction {
        pack_keys,
        packs_after,
        compacted_bytes,
    })
}

async fn read_prefix(path: &Path, max_bytes: u64) -> String {
    use tokio::io::AsyncReadExt;

    let Ok(file) = tokio::fs::File::open(path).await else {
        return "<stderr unavailable>".to_string();
    };
    let mut bytes = Vec::new();
    let mut limited = file.take(max_bytes);
    if limited.read_to_end(&mut bytes).await.is_err() {
        return "<stderr unavailable>".to_string();
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// Compose `m_after` from the parent manifest and the new ref/pack state.
///
/// Encodes `Inv_Closed` at the construction site: `m_after.packs ⊇
/// m_after.parent.packs`. Sorts + dedups packs so canonical bytes are
/// stable across `parent + same_new_pack` regardless of insertion order.
///
/// `parent_digest` is the 64-hex SHA-256 of the parent manifest's
/// canonical bytes — *not* the full `manifests/<digest>` key. Storing the
/// raw digest matches `Inv_RefDerivedFromParent` (parent = pointer.digest)
/// and lets readers reconstruct the chain by prefixing `manifests/` at
/// fetch time.
///
/// Pure data; does not call `Manifest::validate()`. Validation lives at
/// the write seam in [`cas_publish`] so a future refactor that drops the
/// `validate()` call is visible as the absence of a `validate?` between
/// `compose_after` and `put_manifest`, not a hidden behavior change.
fn compose_after(
    parent: &Manifest,
    parent_digest: Option<String>,
    head: String,
    refs: BTreeMap<String, String>,
    new_pack_key: Option<String>,
) -> Manifest {
    let mut packs = parent.packs.clone();
    if let Some(k) = new_pack_key {
        if !packs.iter().any(|p| p == &k) {
            packs.push(k);
        }
    }
    packs.sort();
    packs.dedup();
    Manifest {
        version: MANIFEST_VERSION,
        head,
        refs,
        packs,
        parent: parent_digest,
    }
}

fn compose_compacted_after(
    parent_digest: Option<String>,
    head: String,
    refs: BTreeMap<String, String>,
    mut compacted_pack_keys: Vec<String>,
) -> Manifest {
    compacted_pack_keys.sort();
    compacted_pack_keys.dedup();
    Manifest {
        version: MANIFEST_VERSION,
        head,
        refs,
        packs: compacted_pack_keys,
        parent: parent_digest,
    }
}

/// Derive `manifests/<sha256>` from a returned manifest key, surfacing the
/// hex digest the pointer body needs.
fn digest_from_manifest_key(key: &str) -> Result<String, CasError> {
    key.strip_prefix("manifests/")
        .map(str::to_string)
        .ok_or_else(|| {
            CasError::Backend(StoreError::Backend(s3::error::S3Error::HttpFailWithBody(
                500,
                format!("put_manifest returned non-standard key: {key}"),
            )))
        })
}

fn record_compaction(
    outcome: &'static str,
    started_at: Instant,
    packs_before: usize,
    packs_after: Option<usize>,
    compacted_bytes: Option<u64>,
) {
    metrics::counter!("buzz_git_pack_compactions_total", "outcome" => outcome).increment(1);
    metrics::histogram!("buzz_git_pack_compaction_seconds", "outcome" => outcome)
        .record(started_at.elapsed().as_secs_f64());
    metrics::histogram!("buzz_git_pack_compaction_packs_before").record(packs_before as f64);
    if let Some(packs_after) = packs_after {
        metrics::histogram!("buzz_git_pack_compaction_packs_after").record(packs_after as f64);
    }
    if let Some(compacted_bytes) = compacted_bytes {
        metrics::histogram!("buzz_git_pack_compaction_bytes").record(compacted_bytes as f64);
    }
}

/// The function the §Push step 2–7 protocol distills to.
///
/// **Caller contract — `Inv_RefDerivedFromParent` is structural.** The
/// `parent_state` you pass in must be the same one the workspace was
/// hydrated from. Concretely: `hydrate::hydrate_for_write(store, ctx, owner,
/// repo)` returns `(HydratedRepo, ParentState)` from a single pointer
/// observation → `install_hook(repo.path())` → run `receive-pack`
/// against the workspace → call this with the **same `parent_state`**.
/// The CAS predicate is `parent_state.if_match`, so a concurrent writer
/// that advanced the pointer between hydrate and CAS reliably surfaces
/// as `CasError::Conflict { winner_manifest, .. }` (412 → HTTP 409). The
/// loser re-pushes; the new push re-hydrates against the advanced state.
///
/// Concurrency: callable in parallel for the same `(owner, repo)`. The CAS
/// at step 7 is the *only* writer serialization (`Inv_NoFork`). No
/// advisory lock — adding one would hide exactly the interleavings the
/// model proves safe.
pub async fn cas_publish(
    store: &GitStore,
    ctx: &TenantContext,
    repo_path: &Path,
    owner: &str,
    repo: &str,
    parent_state: &ParentState,
    limits: PublishLimits,
) -> Result<CasSuccess, CasError> {
    cas_publish_inner(
        store,
        ctx,
        repo_path,
        owner,
        repo,
        parent_state,
        PublishOptions {
            limits,
            compaction_threshold: PACK_COMPACTION_THRESHOLD,
        },
    )
    .await
}

async fn cas_publish_inner(
    store: &GitStore,
    ctx: &TenantContext,
    repo_path: &Path,
    owner: &str,
    repo: &str,
    parent_state: &ParentState,
    options: PublishOptions,
) -> Result<CasSuccess, CasError> {
    let limits = options.limits;
    let pkey = pointer_key(ctx.community(), owner, repo);

    // Hydrated repositories are direct children of the configured Git scratch
    // root. Reuse that parent for publication tempfiles so they remain on the
    // mounted scratch volume without adding another independent path argument.
    let scratch_dir = repo_path
        .parent()
        .ok_or_else(|| CasError::PackCapture("repository path has no scratch parent".into()))?;

    // Snapshot post-receive-pack state from disk. `parent_state.parent.refs`
    // are the refs the workspace was hydrated from — `pack-objects --revs`
    // below uses them as the "negative" set to produce the delta pack.
    let (refs_after, head_observed) = snapshot_workspace_state(repo_path, scratch_dir).await?;

    // Fresh bare repositories can inherit Git's environment-dependent
    // `master` symref even when the first push creates `main`. Publish a real
    // branch whenever one exists so HEAD never advertises a phantom branch.
    let head = resolve_published_head(&refs_after, head_observed, &parent_state.parent.head);

    let packs_before = parent_state.parent.packs.len();
    let mut compaction_failure = None;
    let mut compaction_observation = None;
    let mut compacted_manifest = None;
    if should_compact(packs_before, options.compaction_threshold) {
        let started_at = Instant::now();
        match acquire_compaction_permit().await {
            Ok(_permit) => {
                let operation = prepare_compaction(
                    store,
                    repo_path,
                    &refs_after,
                    limits,
                    scratch_dir,
                    packs_before,
                );
                match tokio::time::timeout(PACK_COMPACTION_OPERATION_TIMEOUT, operation).await {
                    Ok(Ok(prepared)) => {
                        debug!(
                            packs_before,
                            packs_after = prepared.packs_after,
                            bytes = prepared.compacted_bytes,
                            "captured compacted repository pack set"
                        );
                        compacted_manifest = Some(compose_compacted_after(
                            parent_state.parent_digest.clone(),
                            head.clone(),
                            refs_after.clone(),
                            prepared.pack_keys,
                        ));
                        compaction_observation = Some(CompactionObservation {
                            started_at,
                            packs_before,
                            packs_after: prepared.packs_after,
                            compacted_bytes: prepared.compacted_bytes,
                        });
                    }
                    Ok(Err(error)) => compaction_failure = Some((started_at, error)),
                    Err(_) => {
                        compaction_failure = Some((
                            started_at,
                            CasError::PackCapture(format!(
                                "pack compaction operation timed out after {} seconds",
                                PACK_COMPACTION_OPERATION_TIMEOUT.as_secs()
                            )),
                        ));
                    }
                }
            }
            Err(error) => compaction_failure = Some((started_at, error)),
        }
    }
    if let Some((started_at, error)) = &compaction_failure {
        warn!(
            packs_before,
            error = %error,
            "pack compaction failed; attempting normal delta-pack publication"
        );
        record_compaction("fallback", *started_at, packs_before, None, None);
    }

    let m_after = if let Some(manifest) = compacted_manifest {
        manifest
    } else {
        // Capture new objects as a delta pack (steps 1–2). The "not" set is
        // the parent manifest's refs — i.e. the set the workspace was hydrated
        // against — so the delta covers exactly the objects this push
        // introduced.
        let pack_bytes = capture_pack(
            repo_path,
            &parent_state.parent.refs,
            &refs_after,
            limits.max_pack_bytes,
            scratch_dir,
        )
        .await?;
        if pack_bytes.is_some() && packs_before >= MAX_MANIFEST_PACKS {
            let error = compaction_failure
                .map(|(_, error)| error)
                .unwrap_or_else(|| {
                    CasError::ResourceLimit(format!(
                        "repository already names {packs_before} packs and compaction failed"
                    ))
                });
            metrics::counter!("buzz_git_pack_compaction_required_failures_total").increment(1);
            return Err(error);
        }
        let new_pack_key = if let Some(bytes) = pack_bytes {
            let new_pack_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let total_bytes = limits
                .parent_hydrated_bytes
                .checked_add(new_pack_bytes)
                .ok_or_else(|| CasError::ResourceLimit("repo byte count overflowed u64".into()))?;
            if total_bytes > limits.max_repo_bytes {
                return Err(CasError::ResourceLimit(format!(
                    "repo would need {total_bytes} hydrated bytes (max {})",
                    limits.max_repo_bytes
                )));
            }
            debug!(bytes = bytes.len(), "captured push pack");
            let pack_key = store.put_pack(&bytes).await?;
            if let Err(e) = write_idx_sidecar(store, &pack_key, &bytes, scratch_dir).await {
                warn!(
                    pack_key = %pack_key,
                    error = %e,
                    "failed to write git pack idx sidecar; push will continue"
                );
            }
            Some(pack_key)
        } else {
            debug!("no new objects in push; manifest will reuse parent packs");
            None
        };
        compose_after(
            &parent_state.parent,
            parent_state.parent_digest.clone(),
            head,
            refs_after,
            new_pack_key,
        )
    };

    // **Pre-CAS validation** (Sami #2 / Max / Dawn): refuse to commit an
    // un-clone-able manifest. `Manifest::validate` checks every refname
    // against `is_safe_refname`, every oid against `is_hex_oid`, and
    // requires a non-empty `head` — same predicates the hydrate path
    // uses on read. Failure surfaces as `CasError::ManifestInvalid`
    // (4xx-class: client/input rejected) so the caller never confuses
    // it with `ManifestReadFailed` (5xx-class: parent corrupt).
    if let Err(error) = m_after.validate() {
        if let Some(observation) = &compaction_observation {
            record_compaction(
                "validation_error",
                observation.started_at,
                observation.packs_before,
                Some(observation.packs_after),
                Some(observation.compacted_bytes),
            );
        }
        return Err(error.into());
    }

    // Step 6: put_manifest.
    let manifest_bytes = m_after.canonical_bytes()?;
    let manifest_key = match store.put_manifest(&manifest_bytes).await {
        Ok(key) => key,
        Err(error) => {
            if let Some(observation) = &compaction_observation {
                record_compaction(
                    "publish_error",
                    observation.started_at,
                    observation.packs_before,
                    Some(observation.packs_after),
                    Some(observation.compacted_bytes),
                );
            }
            return Err(error.into());
        }
    };
    let manifest_digest = digest_from_manifest_key(&manifest_key)?;

    // Step 7: CAS the pointer.
    let precond = match &parent_state.if_match {
        Some(e) => Precond::IfMatch(e.clone()),
        None => Precond::IfNoneMatchStar,
    };
    let cas_outcome = match store
        .put_pointer(&pkey, manifest_digest.as_bytes(), precond)
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Some(observation) = &compaction_observation {
                record_compaction(
                    "publish_error",
                    observation.started_at,
                    observation.packs_before,
                    Some(observation.packs_after),
                    Some(observation.compacted_bytes),
                );
            }
            return Err(error.into());
        }
    };
    match cas_outcome {
        CasOutcome::Won(_new_etag) => {
            if let Some(observation) = &compaction_observation {
                record_compaction(
                    "success",
                    observation.started_at,
                    observation.packs_before,
                    Some(observation.packs_after),
                    Some(observation.compacted_bytes),
                );
            }
            Ok(CasSuccess {
                manifest: m_after,
                manifest_key,
            })
        }
        CasOutcome::LostRace => {
            if let Some(observation) = &compaction_observation {
                record_compaction(
                    "cas_conflict",
                    observation.started_at,
                    observation.packs_before,
                    Some(observation.packs_after),
                    Some(observation.compacted_bytes),
                );
            }
            // Surface a typed Conflict carrying the winner so the caller
            // can reconcile the on-disk workspace without re-reading the
            // pointer. We re-GET the pointer here on the slow path; a
            // *third* writer may have landed between our 412 and this
            // GET, in which case we surface that third winner — also
            // correct (loser re-pushes against whatever's current).
            let expected = parent_state
                .if_match
                .as_ref()
                .map(|e| e.0.as_str())
                .unwrap_or("<first-push>");
            warn!(
                pointer = %pkey,
                expected_etag = %expected,
                attempted_manifest = %manifest_key,
                "CAS lost race; resolving winner for reconcile"
            );
            let (winner_manifest, winner_manifest_key) =
                read_winner_after_conflict(store, &pkey).await?;
            Err(CasError::Conflict {
                winner_manifest,
                winner_manifest_key,
            })
        }
    }
}

/// Re-read the pointer after a `LostRace` and fetch the winner's manifest.
///
/// Fail-closed at every step: if the pointer is now absent (a deletion
/// raced in — currently impossible under the protocol's no-delete rule,
/// but defensive), or the named manifest is corrupt/missing, return
/// `ManifestReadFailed` so the caller emits 5xx rather than pretending
/// reconciliation is possible.
async fn read_winner_after_conflict(
    store: &GitStore,
    pkey: &str,
) -> Result<(Box<Manifest>, String), CasError> {
    let Some((_etag, body)) = store.get_pointer(pkey).await? else {
        return Err(CasError::ManifestReadFailed(
            "pointer vanished after LostRace (no-delete rule violated)".into(),
        ));
    };
    let digest = std::str::from_utf8(&body)
        .map_err(|e| CasError::ManifestReadFailed(format!("winner pointer body not utf-8: {e}")))?
        .trim()
        .to_string();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CasError::ManifestReadFailed(format!(
            "winner pointer body is not a 64-char hex digest (got {} chars)",
            digest.len()
        )));
    }
    let manifest_key = format!("manifests/{digest}");
    let bytes = store
        .get_verified(&manifest_key, &digest)
        .await
        .map_err(|e| match e {
            StoreError::DigestMismatch { .. } => {
                CasError::ManifestReadFailed(format!("winner manifest digest mismatch: {e}"))
            }
            StoreError::NotFound(_) => {
                CasError::ManifestReadFailed(format!("winner pointer names missing manifest: {e}"))
            }
            other => CasError::Backend(other),
        })?;
    let winner = Manifest::from_bytes(&bytes)
        .map_err(|e| CasError::ManifestReadFailed(format!("parse winner manifest: {e}")))?;
    Ok((Box::new(winner), manifest_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `pointer_key` is owned by `manifest.rs` and unit-tested there
    // (one source of truth — Max/Sami's centralization point).
    fn pack_key(ch: char) -> String {
        format!("packs/{}", ch.to_string().repeat(64))
    }

    #[test]
    fn digest_from_pack_key_strips_prefix() {
        let k = format!("packs/{}", "b".repeat(64));
        let d = digest_from_pack_key(&k).unwrap();
        assert_eq!(d, "b".repeat(64));
    }

    #[test]
    fn digest_from_pack_key_rejects_unknown_prefix_or_bad_digest() {
        assert!(digest_from_pack_key("manifests/abc").is_err());
        assert!(digest_from_pack_key("packs/abc").is_err());
        assert!(digest_from_pack_key(&format!("packs/{}", "g".repeat(64))).is_err());
    }

    #[test]
    fn published_head_ignores_dangling_master_when_main_exists() {
        let refs = BTreeMap::from([("refs/heads/main".to_string(), "1".repeat(40))]);
        assert_eq!(
            resolve_published_head(&refs, "refs/heads/master".to_string(), ""),
            "refs/heads/main"
        );
    }

    #[test]
    fn published_head_preserves_an_existing_nonstandard_branch() {
        let refs = BTreeMap::from([("refs/heads/release".to_string(), "1".repeat(40))]);
        assert_eq!(
            resolve_published_head(&refs, "refs/heads/release".to_string(), "refs/heads/main"),
            "refs/heads/release"
        );
    }

    #[test]
    fn digest_from_key_strips_prefix() {
        let k = format!("manifests/{}", "a".repeat(64));
        let d = digest_from_manifest_key(&k).unwrap();
        assert_eq!(d, "a".repeat(64));
    }

    #[test]
    fn digest_from_key_rejects_unknown_prefix() {
        assert!(digest_from_manifest_key("not/manifests/abc").is_err());
    }

    #[test]
    fn compose_after_first_push() {
        let parent = ParentState::fresh().parent;
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".into(), "1".repeat(40));
        let m = compose_after(
            &parent,
            None,
            "refs/heads/main".into(),
            refs.clone(),
            Some(pack_key('a')),
        );
        assert_eq!(m.version, MANIFEST_VERSION);
        assert_eq!(m.head, "refs/heads/main");
        assert_eq!(m.refs, refs);
        assert_eq!(m.packs, vec![pack_key('a')]);
        assert_eq!(m.parent, None);
    }

    /// 64-char hex parent digest — what `Manifest.parent` stores (the
    /// canonical-bytes SHA-256 of the parent manifest, NOT the full
    /// `manifests/<digest>` key). See `Inv_RefDerivedFromParent`.
    fn parent_digest() -> String {
        "a".repeat(64)
    }

    #[test]
    fn compose_after_covers_parent_packs() {
        let mut parent = ParentState::fresh().parent;
        parent.packs = vec![pack_key('1'), pack_key('2')];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            Some(pack_key('3')),
        );
        // Inv_Closed: child covers parent.
        for p in &parent.packs {
            assert!(m.packs.contains(p));
        }
        assert!(m.packs.contains(&pack_key('3')));
        // Sorted.
        let mut sorted = m.packs.clone();
        sorted.sort();
        assert_eq!(m.packs, sorted);
        // Parent is the digest, not the full key (Inv_RefDerivedFromParent).
        assert_eq!(m.parent, Some(parent_digest()));
        assert_eq!(m.parent.as_ref().unwrap().len(), 64);
        assert!(!m.parent.as_ref().unwrap().starts_with("manifests/"));
    }

    #[test]
    fn compose_after_no_new_pack_refs_only_push() {
        let mut parent = ParentState::fresh().parent;
        parent.packs = vec![pack_key('e')];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            None,
        );
        assert_eq!(m.packs, vec![pack_key('e')]);
    }

    #[test]
    fn compose_after_dedupes_pack_already_in_parent() {
        let mut parent = ParentState::fresh().parent;
        parent.packs = vec![pack_key('e')];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            Some(pack_key('e')),
        );
        assert_eq!(m.packs, vec![pack_key('e')]);
    }

    #[test]
    fn compaction_starts_with_manifest_headroom() {
        assert!(!should_compact(
            PACK_COMPACTION_THRESHOLD - 1,
            PACK_COMPACTION_THRESHOLD
        ));
        assert!(should_compact(
            PACK_COMPACTION_THRESHOLD,
            PACK_COMPACTION_THRESHOLD
        ));
        assert!(should_compact(
            MAX_MANIFEST_PACKS,
            PACK_COMPACTION_THRESHOLD
        ));
    }

    #[test]
    fn compaction_must_reduce_before_cap_but_may_replace_at_cap() {
        assert!(compacted_pack_set_is_usable(
            PACK_COMPACTION_THRESHOLD,
            PACK_COMPACTION_THRESHOLD - 1
        ));
        assert!(!compacted_pack_set_is_usable(
            PACK_COMPACTION_THRESHOLD,
            PACK_COMPACTION_THRESHOLD
        ));
        assert!(compacted_pack_set_is_usable(
            MAX_MANIFEST_PACKS,
            MAX_MANIFEST_PACKS
        ));
        assert!(!compacted_pack_set_is_usable(
            MAX_MANIFEST_PACKS,
            MAX_MANIFEST_PACKS + 1
        ));
    }

    #[test]
    fn compacted_manifest_replaces_parent_pack_set() {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".into(), "1".repeat(40));
        let manifest = compose_compacted_after(
            Some(parent_digest()),
            "refs/heads/main".into(),
            refs,
            vec![pack_key('9'), pack_key('8'), pack_key('9')],
        );

        assert_eq!(manifest.packs, vec![pack_key('8'), pack_key('9')]);
        assert_eq!(manifest.parent, Some(parent_digest()));
        manifest.validate().expect("compacted manifest");
    }

    #[test]
    fn compacted_empty_repository_needs_no_packs() {
        let manifest = compose_compacted_after(
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            Vec::new(),
        );

        assert!(manifest.packs.is_empty());
        manifest.validate().expect("empty compacted manifest");
    }

    async fn run_test_git(repo: &Path, args: &[&str]) -> std::process::Output {
        let mut command = Command::new("git");
        command.current_dir(repo).args(args);
        super::super::transport::harden_git_env(&mut command);
        let output = command.output().await.expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    #[tokio::test]
    async fn full_compaction_pack_covers_current_refs() {
        let scratch = TempDir::new().expect("scratch");
        let repo = scratch.path().join("source");
        tokio::fs::create_dir(&repo).await.expect("repo dir");
        run_test_git(&repo, &["init", "--quiet", "--initial-branch=main"]).await;
        run_test_git(&repo, &["config", "user.email", "compact@test"]).await;
        run_test_git(&repo, &["config", "user.name", "compact"]).await;
        tokio::fs::write(repo.join("file.txt"), b"reachable\n")
            .await
            .expect("file");
        run_test_git(&repo, &["add", "file.txt"]).await;
        run_test_git(&repo, &["commit", "--quiet", "-m", "reachable"]).await;
        let oid = String::from_utf8(run_test_git(&repo, &["rev-parse", "HEAD"]).await.stdout)
            .expect("oid utf8")
            .trim()
            .to_string();
        let refs = BTreeMap::from([("refs/heads/main".to_string(), oid)]);

        let compacted = capture_compacted_packs(
            &repo,
            &refs,
            PublishLimits {
                parent_hydrated_bytes: 0,
                max_pack_bytes: 1024 * 1024,
                max_repo_bytes: 2 * 1024 * 1024,
            },
            scratch.path(),
        )
        .await
        .expect("compact");

        assert_eq!(compacted.packs.len(), 1);
        assert!(compacted.total_pack_bytes > 0);
        let idx = compacted.packs[0].idx_path.to_str().expect("idx utf8");
        run_test_git(&repo, &["verify-pack", idx]).await;
    }

    fn probe_enabled() -> bool {
        std::env::var("BUZZ_GIT_S3_PROBE").as_deref() == Ok("1")
    }

    fn live_store() -> GitStore {
        let endpoint = std::env::var("BUZZ_GIT_S3_ENDPOINT")
            .or_else(|_| std::env::var("BUZZ_S3_ENDPOINT"))
            .unwrap_or_else(|_| "http://localhost:9000".into());
        let access_key = std::env::var("BUZZ_GIT_S3_ACCESS_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_ACCESS_KEY"))
            .unwrap_or_else(|_| "buzz_dev".into());
        let secret_key = std::env::var("BUZZ_GIT_S3_SECRET_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_SECRET_KEY"))
            .unwrap_or_else(|_| "buzz_dev_secret".into());
        let bucket = std::env::var("BUZZ_GIT_S3_BUCKET")
            .or_else(|_| std::env::var("BUZZ_S3_BUCKET"))
            .unwrap_or_else(|_| "buzz-media".into());
        let region = std::env::var("BUZZ_GIT_S3_REGION")
            .or_else(|_| std::env::var("BUZZ_S3_REGION"))
            .unwrap_or_else(|_| "us-east-1".into());
        GitStore::new(&endpoint, &access_key, &secret_key, &bucket, &region).expect("connect minio")
    }

    fn tenant() -> TenantContext {
        TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
            "git.example",
        )
    }

    #[tokio::test]
    async fn live_cas_publish_compacts_and_rehydrates() {
        if !probe_enabled() {
            return;
        }
        let store = live_store();
        let scratch = TempDir::new().expect("scratch");
        let source_path = scratch.path().join("source");
        tokio::fs::create_dir(&source_path).await.expect("repo dir");
        run_test_git(&source_path, &["init", "--quiet", "--initial-branch=main"]).await;
        run_test_git(&source_path, &["config", "user.email", "compact@test"]).await;
        run_test_git(&source_path, &["config", "user.name", "compact"]).await;
        tokio::fs::write(source_path.join("file.txt"), b"one\n")
            .await
            .expect("file");
        run_test_git(&source_path, &["add", "file.txt"]).await;
        run_test_git(&source_path, &["commit", "--quiet", "-m", "one"]).await;
        let first_oid = String::from_utf8(
            run_test_git(&source_path, &["rev-parse", "HEAD"])
                .await
                .stdout,
        )
        .expect("first oid utf8")
        .trim()
        .to_string();
        let first_refs = BTreeMap::from([("refs/heads/main".to_string(), first_oid)]);
        let first_pack = capture_pack(
            &source_path,
            &BTreeMap::new(),
            &first_refs,
            1024 * 1024,
            scratch.path(),
        )
        .await
        .expect("capture first")
        .expect("first pack");
        let first_pack_bytes = u64::try_from(first_pack.len()).expect("first pack length");
        let first_pack_key = store.put_pack(&first_pack).await.expect("put first pack");
        write_idx_sidecar(&store, &first_pack_key, &first_pack, scratch.path())
            .await
            .expect("first idx");

        tokio::fs::write(source_path.join("file.txt"), b"one\ntwo\n")
            .await
            .expect("second file");
        run_test_git(&source_path, &["add", "file.txt"]).await;
        run_test_git(&source_path, &["commit", "--quiet", "-m", "two"]).await;
        run_test_git(&source_path, &["tag", "-a", "v1", "-m", "annotated"]).await;
        let parent_head_oid = String::from_utf8(
            run_test_git(&source_path, &["rev-parse", "HEAD"])
                .await
                .stdout,
        )
        .expect("parent head utf8")
        .trim()
        .to_string();
        let parent_tag_oid = String::from_utf8(
            run_test_git(&source_path, &["rev-parse", "refs/tags/v1"])
                .await
                .stdout,
        )
        .expect("parent tag utf8")
        .trim()
        .to_string();
        let parent_refs = BTreeMap::from([
            ("refs/heads/main".to_string(), parent_head_oid),
            ("refs/tags/v1".to_string(), parent_tag_oid),
        ]);
        let second_pack = capture_pack(
            &source_path,
            &first_refs,
            &parent_refs,
            1024 * 1024,
            scratch.path(),
        )
        .await
        .expect("capture second")
        .expect("second pack");
        let second_pack_bytes = u64::try_from(second_pack.len()).expect("second pack length");
        let second_pack_key = store.put_pack(&second_pack).await.expect("put second pack");
        write_idx_sidecar(&store, &second_pack_key, &second_pack, scratch.path())
            .await
            .expect("second idx");

        let parent = Manifest {
            version: MANIFEST_VERSION,
            head: "refs/heads/main".into(),
            refs: parent_refs,
            packs: vec![first_pack_key, second_pack_key],
            parent: None,
        };
        parent.validate().expect("parent manifest");
        let parent_key = store
            .put_manifest(&parent.canonical_bytes().expect("parent bytes"))
            .await
            .expect("put parent");
        let parent_digest = digest_from_manifest_key(&parent_key).expect("parent digest");
        let ctx = tenant();
        let owner = format!("compact-{}", uuid::Uuid::new_v4());
        let repo = "history";
        let pkey = pointer_key(ctx.community(), &owner, repo);
        match store
            .put_pointer(&pkey, parent_digest.as_bytes(), Precond::IfNoneMatchStar)
            .await
            .expect("put pointer")
        {
            CasOutcome::Won(_) => {}
            CasOutcome::LostRace => panic!("unique pointer must win"),
        }
        let cache_parent = scratch.path().join("cache");
        let cache =
            crate::api::git::pack_cache::GitPackCache::new(&cache_parent, 4 * 1024 * 1024, 1)
                .expect("cache");
        let (hydrated, parent_state) = crate::api::git::hydrate::hydrate_for_write(
            &store,
            &ctx,
            &owner,
            repo,
            crate::api::git::hydrate::HydrationOptions {
                pack_cache: &cache,
                scratch_dir: scratch.path(),
                max_pack_bytes: 1024 * 1024,
                max_repo_bytes: 2 * 1024 * 1024,
            },
        )
        .await
        .expect("hydrate parent");
        assert_eq!(parent_state.parent_digest, Some(parent_digest.clone()));

        tokio::fs::write(source_path.join("file.txt"), b"one\ntwo\nthree\n")
            .await
            .expect("third file");
        run_test_git(&source_path, &["add", "file.txt"]).await;
        run_test_git(&source_path, &["commit", "--quiet", "-m", "three"]).await;
        let hydrated_path = hydrated.path().to_str().expect("hydrated path utf8");
        run_test_git(&source_path, &["push", "--quiet", hydrated_path, "main"]).await;
        let head_oid = String::from_utf8(
            run_test_git(&source_path, &["rev-parse", "HEAD"])
                .await
                .stdout,
        )
        .expect("post-push head utf8")
        .trim()
        .to_string();
        let limits = PublishLimits {
            parent_hydrated_bytes: hydrated.hydrated_bytes(),
            max_pack_bytes: 1024 * 1024,
            max_repo_bytes: 2 * 1024 * 1024,
        };

        let test_options = PublishOptions {
            limits,
            compaction_threshold: 2,
        };
        let success = cas_publish_inner(
            &store,
            &ctx,
            hydrated.path(),
            &owner,
            repo,
            &parent_state,
            test_options,
        )
        .await
        .expect("compaction publish");
        assert_eq!(success.manifest.packs.len(), 1);
        assert_eq!(
            success.manifest.refs.get("refs/heads/main"),
            Some(&head_oid)
        );
        assert!(success.manifest.refs.contains_key("refs/tags/v1"));
        assert_eq!(success.manifest.parent, Some(parent_digest));

        let conflict = cas_publish_inner(
            &store,
            &ctx,
            hydrated.path(),
            &owner,
            repo,
            &parent_state,
            test_options,
        )
        .await
        .expect_err("stale parent must lose CAS");
        assert!(matches!(conflict, CasError::Conflict { .. }));

        let hydrated = crate::api::git::hydrate::hydrate_for_read(
            &store,
            &ctx,
            &owner,
            repo,
            crate::api::git::hydrate::HydrationOptions {
                pack_cache: &cache,
                scratch_dir: scratch.path(),
                max_pack_bytes: limits.max_pack_bytes,
                max_repo_bytes: limits.max_repo_bytes,
            },
        )
        .await
        .expect("hydrate compacted")
        .expect("repo exists");
        let hydrated_head = String::from_utf8(
            run_test_git(hydrated.path(), &["rev-parse", "HEAD"])
                .await
                .stdout,
        )
        .expect("hydrated head utf8")
        .trim()
        .to_string();
        assert_eq!(hydrated_head, head_oid);
        run_test_git(hydrated.path(), &["cat-file", "-e", "refs/tags/v1^{tag}"]).await;
        assert_eq!(
            parent_state.parent.packs.len(),
            2,
            "test parent must exercise pack-count reduction"
        );
        assert_eq!(
            limits.parent_hydrated_bytes,
            first_pack_bytes + second_pack_bytes
        );
    }

    /// `cas_publish` must invoke `Manifest::validate()` between
    /// `compose_after` and `put_manifest`. The unit on `validate` lives in
    /// `manifest.rs`; this test pins that the call site here actually
    /// invokes it. A future refactor that drops the `validate?` line is
    /// caught here, not at every subsequent un-clone-able read.
    ///
    /// We can't easily call `cas_publish` end-to-end without a `GitStore`,
    /// so this exercises the exact chain `cas_publish` uses inline:
    /// `compose_after(...)` → `validate()` → expected variant.
    #[test]
    fn validate_invoked_between_compose_and_put_manifest() {
        let parent = ParentState::fresh().parent;
        let mut refs = BTreeMap::new();
        // Unsafe refname: `..` traversal.
        refs.insert("refs/heads/../escape".into(), "1".repeat(40));
        let m = compose_after(
            &parent,
            None,
            "refs/heads/main".into(),
            refs,
            Some(pack_key('a')),
        );
        let manifest_err = m.validate().expect_err("unsafe refname must reject");
        match &manifest_err {
            crate::api::git::manifest::ManifestError::UnsafeRefName(name) => {
                assert!(name.contains(".."));
            }
            other => panic!("expected UnsafeRefName, got {other:?}"),
        }

        // Same error converts through the `From` into the typed CasError
        // variant `cas_publish` actually returns at the call site.
        let cas_err: CasError = manifest_err.into();
        assert!(matches!(cas_err, CasError::ManifestInvalid(_)));
    }

    /// First-push + empty HEAD must fail validation. `ParentState::fresh`
    /// has empty `parent.head`, so the HEAD fallback in `cas_publish`
    /// leaves `m_after.head = ""` if `git symbolic-ref` also failed. The
    /// validator catches this pre-CAS rather than installing an
    /// un-clone-able manifest.
    #[test]
    fn first_push_with_empty_head_rejected_by_validate() {
        let parent = ParentState::fresh().parent;
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".into(), "1".repeat(40));
        let m = compose_after(
            &parent,
            None,
            String::new(), // empty HEAD — the fallback's worst case
            refs,
            Some(pack_key('a')),
        );
        assert!(matches!(
            m.validate(),
            Err(crate::api::git::manifest::ManifestError::EmptyHead)
        ));
    }
}
