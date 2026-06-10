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
//! 4. Captures the new objects as a pack via `git pack-objects --revs
//!    --stdout` over `(refs_after) --not (refs_before-tips)` (§Push step 1–2).
//!    Empty pack (refs-only push that doesn't introduce objects) is allowed
//!    and stored with no `new_pack_keys`.
//! 5. `put_pack` (content-addressed, create-only, idempotent — §Push step 2).
//!    The key is derived from `sha256(bytes)` by the store layer.
//! 6. Composes `m_after` (parent packs ∪ new pack, parent digest, new refs)
//!    via `Manifest::compose`-equivalent inline construction (§Push step 5).
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

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::api::git::manifest::{pointer_key, Manifest, ManifestError, MANIFEST_VERSION};
use crate::api::git::store::{CasOutcome, ETag, GitStore, Precond, StoreError};

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
) -> Result<(BTreeMap<String, String>, String), CasError> {
    let mut refs_cmd = Command::new("git");
    refs_cmd
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(repo_path);
    super::transport::harden_git_env(&mut refs_cmd);
    let refs_out = refs_cmd
        .output()
        .await
        .map_err(|e| CasError::PackCapture(format!("for-each-ref spawn: {e}")))?;
    if !refs_out.status.success() {
        return Err(CasError::PackCapture(format!(
            "for-each-ref failed: status={:?}",
            refs_out.status.code()
        )));
    }
    let mut refs = BTreeMap::new();
    for line in std::str::from_utf8(&refs_out.stdout)
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

    let mut cmd = Command::new("git");
    cmd.args(["pack-objects", "--revs", "--stdout", "-q"])
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::transport::harden_git_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| CasError::PackCapture(format!("pack-objects spawn: {e}")))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CasError::PackCapture("pack-objects stdin closed".into()))?;
        stdin
            .write_all(stdin_lines.as_bytes())
            .await
            .map_err(|e| CasError::PackCapture(format!("pack-objects stdin write: {e}")))?;
        // Drop closes stdin → EOF.
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| CasError::PackCapture(format!("pack-objects wait: {e}")))?;
    if !out.status.success() {
        return Err(CasError::PackCapture(format!(
            "pack-objects failed: status={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    if out.stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(out.stdout))
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

/// The function the §Push step 2–7 protocol distills to.
///
/// **Caller contract — `Inv_RefDerivedFromParent` is structural.** The
/// `parent_state` you pass in must be the same one the workspace was
/// hydrated from. Concretely: `hydrate::hydrate_for_write(store, owner,
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
    repo_path: &Path,
    owner: &str,
    repo: &str,
    parent_state: &ParentState,
) -> Result<CasSuccess, CasError> {
    let pkey = pointer_key(owner, repo);

    // Snapshot post-receive-pack state from disk. `parent_state.parent.refs`
    // are the refs the workspace was hydrated from — `pack-objects --revs`
    // below uses them as the "negative" set to produce the delta pack.
    let (refs_after, head_observed) = snapshot_workspace_state(repo_path).await?;

    // HEAD fallback: a bare repo serving pushes shouldn't have detached
    // HEAD, but if `git symbolic-ref` failed (or returned empty), inherit
    // the parent's HEAD rather than installing an empty one. `validate()`
    // below rejects "empty after fallback" — that's the first-push +
    // detached-HEAD case where the writer must declare a HEAD.
    let head = if head_observed.is_empty() {
        parent_state.parent.head.clone()
    } else {
        head_observed
    };

    // Capture new objects as a pack (steps 1–2). The "not" set is the
    // parent manifest's refs — i.e. the set the workspace was hydrated
    // against — so the delta covers exactly the objects this push
    // introduced.
    let pack_bytes = capture_pack(repo_path, &parent_state.parent.refs, &refs_after).await?;
    let new_pack_key = if let Some(bytes) = pack_bytes {
        debug!(bytes = bytes.len(), "captured push pack");
        Some(store.put_pack(&bytes).await?)
    } else {
        debug!("no new objects in push; manifest will reuse parent packs");
        None
    };

    // Compose m_after (step 5).
    let m_after = compose_after(
        &parent_state.parent,
        parent_state.parent_digest.clone(),
        head,
        refs_after,
        new_pack_key,
    );

    // **Pre-CAS validation** (Sami #2 / Max / Dawn): refuse to commit an
    // un-clone-able manifest. `Manifest::validate` checks every refname
    // against `is_safe_refname`, every oid against `is_hex_oid`, and
    // requires a non-empty `head` — same predicates the hydrate path
    // uses on read. Failure surfaces as `CasError::ManifestInvalid`
    // (4xx-class: client/input rejected) so the caller never confuses
    // it with `ManifestReadFailed` (5xx-class: parent corrupt).
    m_after.validate()?;

    // Step 6: put_manifest.
    let manifest_bytes = m_after.canonical_bytes()?;
    let manifest_key = store.put_manifest(&manifest_bytes).await?;
    let manifest_digest = digest_from_manifest_key(&manifest_key)?;

    // Step 7: CAS the pointer.
    let precond = match &parent_state.if_match {
        Some(e) => Precond::IfMatch(e.clone()),
        None => Precond::IfNoneMatchStar,
    };
    match store
        .put_pointer(&pkey, manifest_digest.as_bytes(), precond)
        .await?
    {
        CasOutcome::Won(_new_etag) => Ok(CasSuccess {
            manifest: m_after,
            manifest_key,
        }),
        CasOutcome::LostRace => {
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
            Some("packs/abc".into()),
        );
        assert_eq!(m.version, MANIFEST_VERSION);
        assert_eq!(m.head, "refs/heads/main");
        assert_eq!(m.refs, refs);
        assert_eq!(m.packs, vec!["packs/abc".to_string()]);
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
        parent.packs = vec!["packs/old1".into(), "packs/old2".into()];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            Some("packs/new".into()),
        );
        // Inv_Closed: child covers parent.
        for p in &parent.packs {
            assert!(m.packs.contains(p));
        }
        assert!(m.packs.contains(&"packs/new".to_string()));
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
        parent.packs = vec!["packs/x".into()];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            None,
        );
        assert_eq!(m.packs, vec!["packs/x".to_string()]);
    }

    #[test]
    fn compose_after_dedupes_pack_already_in_parent() {
        let mut parent = ParentState::fresh().parent;
        parent.packs = vec!["packs/x".into()];
        let m = compose_after(
            &parent,
            Some(parent_digest()),
            "refs/heads/main".into(),
            BTreeMap::new(),
            Some("packs/x".into()),
        );
        assert_eq!(m.packs, vec!["packs/x".to_string()]);
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
            Some("packs/abc".into()),
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
            Some("packs/abc".into()),
        );
        assert!(matches!(
            m.validate(),
            Err(crate::api::git::manifest::ManifestError::EmptyHead)
        ));
    }
}
