//! Read-path hydration: materialize an ephemeral bare repo from an object-store
//! manifest, so the existing `upload-pack`/`info-refs` subprocess runner can
//! serve it.
//!
//! Flow (spec §Read):
//!
//! 1. GET pointer → manifest digest.
//! 2. GET manifest (digest-verified) → parsed [`Manifest`].
//! 3. GET every pack the manifest names (digest-verified, in parallel).
//! 4. **Phase 1** — for each pack: write `pack-<hex>.pack`; install and
//!    validate a cached `idx/<hex>` sidecar when present, otherwise run
//!    `git index-pack` to materialize `.idx`. Failure here tears down the
//!    tempdir with no refs/HEAD ever written.
//! 5. **Phase 2** — only after all packs are indexed: write loose refs and
//!    HEAD from the manifest.
//!
//! The phase boundary is load-bearing: `upload-pack` walks refs → objects;
//! a ref pointing into a not-yet-indexed pack is an opaque protocol failure
//! mid-stream. Sami/Max named this explicitly.
//!
//! The returned [`HydratedRepo`] owns a [`tempfile::TempDir`]; dropping it
//! cleans up. Every read currently re-hydrates from scratch — naive but
//! correct; caching is named follow-up work.

// Public surface is consumed by `transport.rs` after Eva's `AppState::git_store`
// wires in; the items below are intentionally `pub` to keep the consumer-side
// diff minimal once integration lands. We narrow `#[allow(dead_code)]` to those
// specific items rather than blanketing the module — that lets the compiler
// still catch accidental dead code inside `hydrate.rs` itself.

use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tokio::process::Command;

use super::cas_publish::ParentState;
use super::manifest::{is_hex_oid, is_safe_refname, pointer_key, Manifest, ManifestError};
use super::store::{ETag, GitStore, StoreError};
use buzz_core::TenantContext;

/// Manifests should stay small after ref/pack cardinality validation. Bound
/// the object read itself so malformed historical state cannot allocate an
/// arbitrary JSON body before validation runs.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

/// A bare repo hydrated to a temporary directory.
///
/// The tempdir is removed when this value is dropped — callers must keep the
/// handle alive for the duration of the subprocess that reads from `path()`.
pub struct HydratedRepo {
    /// Owns the lifetime of the on-disk tree.
    _tempdir: TempDir,
    /// Absolute path to the bare repo root.
    path: PathBuf,
    /// Total bytes of parent packs materialized into this workspace.
    hydrated_bytes: u64,
}

impl HydratedRepo {
    /// Path to the bare repository — pass this to `upload-pack`/`info-refs`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Total bytes of parent packs materialized into this workspace.
    pub fn hydrated_bytes(&self) -> u64 {
        self.hydrated_bytes
    }
}

/// Hydration errors.
///
/// "Repo doesn't exist" is signalled by `Ok(None)` from `hydrate_for_read`,
/// not a variant here — the type system enforces the 404-vs-5xx split.
/// Resource-limit failures map to 413; other variants are backend/data errors.
#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    /// Pointer body was not a valid manifest digest (64 hex chars).
    #[error("pointer body is not a 64-char hex digest")]
    InvalidPointer,
    /// Manifest serde / version error.
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    /// Store-level error (GET failure, digest mismatch, etc.).
    #[error("store: {0}")]
    Store(#[from] StoreError),
    /// `git init --bare`, `git index-pack`, or filesystem operation failed.
    #[error("hydrate: {0}")]
    Hydrate(String),
    /// A stored repo exceeds the relay's per-request resource budget.
    #[error("resource limit: {0}")]
    ResourceLimit(String),
}

// `pointer_key` is imported from `super::manifest` — single source of truth
// shared with `cas_publish` (write side). See manifest.rs.

/// Hydrate a bare repo for read (`upload-pack` / `info-refs`).
///
/// Returns `Ok(None)` when the pointer is absent — the repo doesn't exist;
/// caller should respond 404. `Ok(Some(_))` is a usable bare repo. Any other
/// failure is a backend/data error.
pub async fn hydrate_for_read(
    store: &GitStore,
    ctx: &TenantContext,
    owner: &str,
    repo: &str,
    max_pack_bytes: u64,
    max_repo_bytes: u64,
) -> Result<Option<HydratedRepo>, HydrateError> {
    let Some((_etag, _digest, manifest)) = load_pointer(store, ctx, owner, repo).await? else {
        return Ok(None);
    };
    Ok(Some(
        materialize_manifest(store, &manifest, max_pack_bytes, max_repo_bytes).await?,
    ))
}

/// Load the current manifest for a read path without materializing pack objects.
///
/// Used by `info/refs` fast paths that can answer directly from manifest refs.
/// Returns `Ok(None)` when the repo pointer is absent; otherwise verifies the
/// pointer-named manifest digest before returning it.
pub async fn load_manifest_for_read(
    store: &GitStore,
    ctx: &TenantContext,
    owner: &str,
    repo: &str,
) -> Result<Option<Manifest>, HydrateError> {
    Ok(load_pointer(store, ctx, owner, repo)
        .await?
        .map(|(_etag, _digest, manifest)| manifest))
}

/// Hydrate a bare repo for write (`receive-pack`) and return the
/// `ParentState` the workspace was hydrated from.
///
/// The returned `ParentState` *must* be passed into
/// [`crate::api::git::cas_publish::cas_publish`] without re-reading the
/// pointer. The CAS predicate is `parent_state.if_match`, so a concurrent
/// writer that advances the pointer between this call and the CAS surfaces
/// reliably as `Conflict`/HTTP 409 — `Inv_RefDerivedFromParent` holds
/// because `m_after.parent` is *literally* the digest of the manifest the
/// workspace was hydrated from.
///
/// First-push case (pointer absent): returns `(empty bare repo,
/// ParentState::fresh())`. The empty bare repo is a fresh `git init --bare`
/// with no refs and no objects; `receive-pack` will accept the first push
/// and create whatever refs the client sends, and `cas_publish` will CAS
/// the pointer with `If-None-Match: *`.
///
/// Any below-pointer failure (manifest 404 under non-empty pointer, digest
/// mismatch, malformed pointer body) is a hard error — never silently
/// treated as "fresh repo," because that would let a corrupt pointer
/// install a brand-new history alongside the broken one.
pub async fn hydrate_for_write(
    store: &GitStore,
    ctx: &TenantContext,
    owner: &str,
    repo: &str,
    max_pack_bytes: u64,
    max_repo_bytes: u64,
) -> Result<(HydratedRepo, ParentState), HydrateError> {
    match load_pointer(store, ctx, owner, repo).await? {
        Some((etag, digest, manifest)) => {
            let repo =
                materialize_manifest(store, &manifest, max_pack_bytes, max_repo_bytes).await?;
            let parent = ParentState::from_loaded(etag, digest, manifest);
            Ok((repo, parent))
        }
        None => {
            // First push: empty bare repo. No packs to fetch, no refs to
            // install. `receive-pack` will accept whatever the client
            // sends; `cas_publish` will use `If-None-Match: *`.
            let tempdir =
                TempDir::new().map_err(|e| HydrateError::Hydrate(format!("tempdir: {e}")))?;
            let path = tempdir.path().to_path_buf();
            run_git(&path, &["init", "--bare", "--quiet"]).await?;
            Ok((
                HydratedRepo {
                    _tempdir: tempdir,
                    path,
                    hydrated_bytes: 0,
                },
                ParentState::fresh(),
            ))
        }
    }
}

/// Resolve the pointer to its `(ETag, digest, verified Manifest)` triple.
///
/// `Ok(None)` if the pointer is absent (caller decides 404 vs first-push
/// per call site). `Err(_)` on any below-pointer failure.
async fn load_pointer(
    store: &GitStore,
    ctx: &TenantContext,
    owner: &str,
    repo: &str,
) -> Result<Option<(ETag, String, Manifest)>, HydrateError> {
    let pkey = pointer_key(ctx.community(), owner, repo);
    let (etag, pointer_bytes) = match store.get_pointer(&pkey).await? {
        Some(p) => p,
        None => return Ok(None),
    };
    let digest = std::str::from_utf8(&pointer_bytes)
        .map_err(|_| HydrateError::InvalidPointer)?
        .trim()
        .to_string();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HydrateError::InvalidPointer);
    }
    let manifest_key = format!("manifests/{digest}");
    let manifest_bytes =
        get_verified_limited(store, &manifest_key, &digest, MAX_MANIFEST_BYTES).await?;
    let manifest = Manifest::from_bytes(&manifest_bytes)?;
    manifest.validate()?;
    Ok(Some((etag, digest, manifest)))
}

async fn get_verified_limited(
    store: &GitStore,
    key: &str,
    digest: &str,
    max_bytes: u64,
) -> Result<bytes::Bytes, HydrateError> {
    match store.get_verified_limited(key, digest, max_bytes).await {
        Ok(bytes) => Ok(bytes),
        Err(StoreError::ObjectTooLarge { size, max, .. }) => Err(HydrateError::ResourceLimit(
            format!("object {key} is {size} bytes (max {max})"),
        )),
        Err(err) => Err(err.into()),
    }
}

/// Materialize a manifest into a fresh tempdir bare repo.
///
/// Shared by `hydrate_for_read` and `hydrate_for_write`. Phase-ordered
/// (packs first + verified + indexed, refs/HEAD only after) so a failed
/// hydrate leaves no advertised refs — failure mode is "empty/no refs,"
/// never "refs point at missing objects."
async fn materialize_manifest(
    store: &GitStore,
    manifest: &Manifest,
    max_pack_bytes: u64,
    max_repo_bytes: u64,
) -> Result<HydratedRepo, HydrateError> {
    // Init bare repo.
    let tempdir = TempDir::new().map_err(|e| HydrateError::Hydrate(format!("tempdir: {e}")))?;
    let path = tempdir.path().to_path_buf();
    run_git(&path, &["init", "--bare", "--quiet"]).await?;

    // Phase 1: fetch, write, and index one pack at a time. Keeping only one
    // verified pack body resident avoids a manifest with many historical packs
    // multiplying memory by pack_count. Any failure here aborts before any ref
    // is written — failed hydrate ⇒ no advertised refs.
    let pack_dir = path.join("objects").join("pack");
    let mut hydrated_bytes = 0u64;
    for key in &manifest.packs {
        let digest = key
            .strip_prefix("packs/")
            .ok_or_else(|| HydrateError::Hydrate(format!("malformed pack key {key:?}")))?;
        let bytes = get_verified_limited(store, key, digest, max_pack_bytes).await?;
        let pack_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if pack_bytes > max_pack_bytes {
            return Err(HydrateError::ResourceLimit(format!(
                "pack {digest} is {pack_bytes} bytes (max {max_pack_bytes})"
            )));
        }
        hydrated_bytes = hydrated_bytes.checked_add(pack_bytes).ok_or_else(|| {
            HydrateError::ResourceLimit("repo byte count overflowed u64".to_string())
        })?;
        if hydrated_bytes > max_repo_bytes {
            return Err(HydrateError::ResourceLimit(format!(
                "repo needs {hydrated_bytes} hydrated bytes (max {max_repo_bytes})"
            )));
        }

        let pack_path = pack_dir.join(format!("pack-{digest}.pack"));
        tokio::fs::write(&pack_path, &bytes)
            .await
            .map_err(|e| HydrateError::Hydrate(format!("write pack {digest}: {e}")))?;
        install_or_generate_idx(store, &path, digest, &pack_path).await?;
    }

    // Phase 2: install refs and HEAD. After this point, the repo advertises.
    for (refname, oid) in &manifest.refs {
        // Defensive: refuse any ref name that escapes the repo or contains
        // null/newline. The writer should already have sanitized; double-check
        // because we're about to write file paths.
        if !is_safe_refname(refname) {
            return Err(HydrateError::Hydrate(format!(
                "manifest contains unsafe refname {refname:?}"
            )));
        }
        if !is_hex_oid(oid) {
            return Err(HydrateError::Hydrate(format!(
                "manifest ref {refname} has malformed oid"
            )));
        }
        let ref_path = path.join(refname);
        if let Some(parent) = ref_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| HydrateError::Hydrate(format!("mkdir {parent:?}: {e}")))?;
        }
        // Loose ref format: oid + newline.
        tokio::fs::write(&ref_path, format!("{oid}\n"))
            .await
            .map_err(|e| HydrateError::Hydrate(format!("write ref {refname}: {e}")))?;
    }

    // HEAD: protocol formatting (`ref: <name>\n`) happens here, not in storage.
    if !is_safe_refname(&manifest.head) {
        return Err(HydrateError::Hydrate(format!(
            "manifest head {:?} is not a safe ref name",
            manifest.head
        )));
    }
    tokio::fs::write(path.join("HEAD"), format!("ref: {}\n", manifest.head))
        .await
        .map_err(|e| HydrateError::Hydrate(format!("write HEAD: {e}")))?;

    Ok(HydratedRepo {
        _tempdir: tempdir,
        path,
        hydrated_bytes,
    })
}

async fn install_or_generate_idx(
    store: &GitStore,
    repo_path: &Path,
    pack_digest: &str,
    pack_path: &Path,
) -> Result<(), HydrateError> {
    let idx_path = pack_path.with_extension("idx");
    match store.get_idx(pack_digest).await {
        Ok(Some(idx_bytes)) => {
            tokio::fs::write(&idx_path, &idx_bytes)
                .await
                .map_err(|e| HydrateError::Hydrate(format!("write idx {pack_digest}: {e}")))?;
            let idx_path_str = idx_path
                .to_str()
                .ok_or_else(|| HydrateError::Hydrate("idx path is not valid utf-8".to_string()))?;
            match run_git(repo_path, &["verify-pack", idx_path_str]).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!(
                        pack_digest = %pack_digest,
                        error = %e,
                        "git pack idx sidecar failed validation; regenerating"
                    );
                    let _ = tokio::fs::remove_file(&idx_path).await;
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(
                pack_digest = %pack_digest,
                error = %e,
                "failed to read git pack idx sidecar; regenerating"
            );
        }
    }

    // No `--strict`: `index-pack` already validates structural integrity
    // (CRC, type tags, internal refs). `--strict` adds connectivity-graph
    // checks, which would re-prove what manifest.packs already covers by
    // construction (Inv_Closed, write-path invariant). Latency cost on every
    // clone is not worth re-proving a write-path bug.
    let pack_path_str = pack_path
        .to_str()
        .ok_or_else(|| HydrateError::Hydrate("pack path is not valid utf-8".to_string()))?;
    run_git(repo_path, &["index-pack", pack_path_str]).await
}

/// Run `git <args>` in `cwd`, fail on non-zero exit.
async fn run_git(cwd: &Path, args: &[&str]) -> Result<(), HydrateError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args).kill_on_drop(true);
    // Match transport.rs's harden_git_env semantics for subprocesses: clear
    // user/system git config so behavior is reproducible.
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("HOME", cwd); // forces $HOME/.gitconfig lookups to miss

    let output = cmd
        .output()
        .await
        .map_err(|e| HydrateError::Hydrate(format!("spawn git {args:?}: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(HydrateError::Hydrate(format!(
            "git {args:?} exited {}: {stderr}",
            output.status
        )));
    }
    Ok(())
}

// `is_safe_refname` and `is_hex_oid` live in `super::manifest` — symmetric
// write-side (Manifest::validate) and read-side (here) protection, single
// source of truth. See `manifest.rs` for the predicates + tests.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn safe_refnames() {
        assert!(is_safe_refname("refs/heads/main"));
        assert!(is_safe_refname("refs/tags/v1.0.0"));
        assert!(is_safe_refname("refs/heads/feat/cas-publish"));
        assert!(!is_safe_refname("refs/heads/../escape"));
        assert!(!is_safe_refname("HEAD"));
        assert!(!is_safe_refname("refs/heads/"));
        assert!(!is_safe_refname("/refs/heads/main"));
        assert!(!is_safe_refname("refs/heads/main\nrefs/heads/evil"));
        assert!(!is_safe_refname("refs/heads/main\0"));
    }

    #[test]
    fn hex_oids() {
        assert!(is_hex_oid(&"a".repeat(40))); // SHA-1
        assert!(is_hex_oid(&"a".repeat(64))); // SHA-256
        assert!(!is_hex_oid(&"a".repeat(39)));
        assert!(!is_hex_oid(&"g".repeat(40))); // non-hex
        assert!(!is_hex_oid(""));
    }

    // `pointer_key` is tested in `super::manifest::tests` — single source.

    fn tenant() -> TenantContext {
        TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
            "git.example",
        )
    }

    // -------- Live MinIO + real git roundtrip ----------------------------------
    //
    // Run manually:
    //   BUZZ_GIT_S3_PROBE=1 cargo test -p buzz-relay --lib \
    //     api::git::hydrate::tests::live -- --nocapture --test-threads=1

    fn probe_enabled() -> bool {
        std::env::var("BUZZ_GIT_S3_PROBE").as_deref() == Ok("1")
    }

    fn store() -> GitStore {
        GitStore::new(
            "http://localhost:9000",
            "buzz_dev",
            "buzz_dev_secret",
            "buzz-git",
            "us-east-1",
        )
        .expect("connect minio")
    }

    /// Build a tiny on-disk repo, return (pack bytes, head_oid).
    async fn build_source_repo() -> (Vec<u8>, String) {
        let src = TempDir::new().unwrap();
        run_git(src.path(), &["init", "--quiet", "--initial-branch=main"])
            .await
            .unwrap();
        run_git(src.path(), &["config", "user.email", "probe@test"])
            .await
            .unwrap();
        run_git(src.path(), &["config", "user.name", "probe"])
            .await
            .unwrap();
        tokio::fs::write(src.path().join("hello.txt"), b"hello\n")
            .await
            .unwrap();
        run_git(src.path(), &["add", "hello.txt"]).await.unwrap();
        run_git(src.path(), &["commit", "-m", "init", "--quiet"])
            .await
            .unwrap();
        run_git(src.path(), &["repack", "-a", "-d", "--quiet"])
            .await
            .unwrap();

        // Find the single pack file and read it.
        let pack_dir = src.path().join(".git").join("objects").join("pack");
        let mut packs = vec![];
        let mut rd = tokio::fs::read_dir(&pack_dir).await.unwrap();
        while let Some(entry) = rd.next_entry().await.unwrap() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("pack") {
                packs.push(p);
            }
        }
        assert_eq!(packs.len(), 1, "expected exactly one pack");
        let pack_bytes = tokio::fs::read(&packs[0]).await.unwrap();

        // Read the HEAD oid.
        let mut cmd = Command::new("git");
        cmd.current_dir(src.path())
            .args(["rev-parse", "HEAD"])
            .kill_on_drop(true);
        let out = cmd.output().await.unwrap();
        let head_oid = String::from_utf8(out.stdout).unwrap().trim().to_string();

        (pack_bytes, head_oid)
    }

    #[tokio::test]
    async fn live_hydrate_roundtrip() {
        if !probe_enabled() {
            return;
        }
        let st = store();

        // Build a real source repo, capture its pack and HEAD oid.
        let (pack_bytes, head_oid) = build_source_repo().await;

        // Upload pack and manifest to S3 under a unique pointer key.
        let pack_key = st.put_pack(&pack_bytes).await.expect("put_pack");
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), head_oid.clone());
        let manifest = Manifest {
            version: 1,
            head: "refs/heads/main".into(),
            refs,
            packs: vec![pack_key.clone()],
            parent: None,
        };
        let manifest_bytes = manifest.canonical_bytes().expect("serialize");
        let manifest_key = st
            .put_manifest(&manifest_bytes)
            .await
            .expect("put_manifest");
        let manifest_digest = manifest_key.strip_prefix("manifests/").unwrap();

        let owner = format!("probe-{}", uuid::Uuid::new_v4());
        let repo = "hello";
        let ctx = tenant();
        let pkey = pointer_key(ctx.community(), &owner, repo);
        match st
            .put_pointer(
                &pkey,
                manifest_digest.as_bytes(),
                super::super::store::Precond::IfNoneMatchStar,
            )
            .await
            .expect("put_pointer")
        {
            super::super::store::CasOutcome::Won(_) => {}
            super::super::store::CasOutcome::LostRace => panic!("first INM* must win"),
        }

        // Hydrate.
        let hydrated = hydrate_for_read(&st, &ctx, &owner, repo, u64::MAX, u64::MAX)
            .await
            .expect("hydrate")
            .expect("hydrate Some");
        eprintln!("hydrated to {}", hydrated.path().display());

        // The hydrated repo must list the same ref with the same oid.
        let mut cmd = Command::new("git");
        cmd.current_dir(hydrated.path())
            .args(["for-each-ref", "--format=%(refname) %(objectname)"])
            .kill_on_drop(true);
        let out = cmd.output().await.unwrap();
        let listing = String::from_utf8(out.stdout).unwrap();
        eprintln!("for-each-ref: {listing}");
        assert!(
            listing.contains(&format!("refs/heads/main {head_oid}")),
            "ref/oid mismatch in hydrated repo: {listing}"
        );

        // HEAD points at refs/heads/main.
        let head_file = tokio::fs::read_to_string(hydrated.path().join("HEAD"))
            .await
            .unwrap();
        assert_eq!(head_file.trim(), "ref: refs/heads/main");

        // git rev-parse HEAD resolves to the same oid.
        let mut rp = Command::new("git");
        rp.current_dir(hydrated.path())
            .args(["rev-parse", "HEAD"])
            .kill_on_drop(true);
        let resolved = String::from_utf8(rp.output().await.unwrap().stdout)
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(resolved, head_oid, "HEAD did not resolve to original oid");

        // Bonus: clone the hydrated repo and verify the file content survives.
        let clone_target = TempDir::new().unwrap();
        let mut clone = Command::new("git");
        clone
            .args([
                "clone",
                "--quiet",
                hydrated.path().to_str().unwrap(),
                clone_target.path().to_str().unwrap(),
            ])
            .kill_on_drop(true);
        let cl_out = clone.output().await.unwrap();
        assert!(
            cl_out.status.success(),
            "clone failed: {}",
            String::from_utf8_lossy(&cl_out.stderr)
        );
        let hello = tokio::fs::read_to_string(clone_target.path().join("hello.txt"))
            .await
            .unwrap();
        assert_eq!(hello, "hello\n");

        eprintln!("✓ hydrate + clone roundtrip works");
        // We leave the probe pointer/manifest/pack behind — the owner is
        // UUID-namespaced so subsequent runs don't collide, and immutable
        // objects accumulate by design (retention is a backend concern).
        let _ = &pkey; // suppress unused warning when cleanup is omitted
    }

    #[tokio::test]
    async fn live_hydrate_missing_pointer_returns_none() {
        if !probe_enabled() {
            return;
        }
        let st = store();
        let owner = format!("nope-{}", uuid::Uuid::new_v4());
        let ctx = tenant();
        let result = hydrate_for_read(&st, &ctx, &owner, "ghost", u64::MAX, u64::MAX)
            .await
            .expect("ok");
        assert!(result.is_none(), "missing pointer must surface as None");
    }

    /// Empty repo: pointer present, manifest carries an empty refs map. A
    /// `git clone` of the hydrated repo must succeed and produce the same
    /// behavior as a `git clone` of a freshly `git init --bare`'d repo —
    /// no objects, no refs, HEAD pointing at the configured default branch.
    #[tokio::test]
    async fn live_hydrate_empty_repo() {
        if !probe_enabled() {
            return;
        }
        let st = store();

        let manifest = Manifest {
            version: 1,
            head: "refs/heads/main".into(),
            refs: BTreeMap::new(),
            packs: vec![],
            parent: None,
        };
        let manifest_bytes = manifest.canonical_bytes().expect("serialize");
        let manifest_key = st
            .put_manifest(&manifest_bytes)
            .await
            .expect("put_manifest");
        let manifest_digest = manifest_key.strip_prefix("manifests/").unwrap();

        let owner = format!("empty-{}", uuid::Uuid::new_v4());
        let ctx = tenant();
        let pkey = pointer_key(ctx.community(), &owner, "void");
        match st
            .put_pointer(
                &pkey,
                manifest_digest.as_bytes(),
                super::super::store::Precond::IfNoneMatchStar,
            )
            .await
            .expect("put_pointer")
        {
            super::super::store::CasOutcome::Won(_) => {}
            super::super::store::CasOutcome::LostRace => panic!("first INM* must win"),
        }

        let hydrated = hydrate_for_read(&st, &ctx, &owner, "void", u64::MAX, u64::MAX)
            .await
            .expect("hydrate")
            .expect("hydrate Some");

        // HEAD points where the manifest said.
        let head_file = tokio::fs::read_to_string(hydrated.path().join("HEAD"))
            .await
            .unwrap();
        assert_eq!(head_file.trim(), "ref: refs/heads/main");

        // No refs.
        let mut cmd = Command::new("git");
        cmd.current_dir(hydrated.path())
            .args(["for-each-ref"])
            .kill_on_drop(true);
        let out = cmd.output().await.unwrap();
        assert!(
            out.stdout.is_empty(),
            "expected no refs, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );

        // git clone must succeed against an empty repo and produce an empty
        // working tree at the configured default branch.
        let clone_target = TempDir::new().unwrap();
        let mut clone = Command::new("git");
        clone
            .args([
                "clone",
                "--quiet",
                hydrated.path().to_str().unwrap(),
                clone_target.path().to_str().unwrap(),
            ])
            .kill_on_drop(true);
        let cl_out = clone.output().await.unwrap();
        let stderr = String::from_utf8_lossy(&cl_out.stderr);
        // git emits "warning: You appear to have cloned an empty repository."
        // on stderr but exits 0. The exit code is the protocol-level signal.
        assert!(
            cl_out.status.success(),
            "empty clone failed (exit={:?}): {stderr}",
            cl_out.status.code()
        );
        eprintln!("✓ empty-repo clone succeeded; stderr: {stderr}");
        let _ = &pkey;
    }
}
