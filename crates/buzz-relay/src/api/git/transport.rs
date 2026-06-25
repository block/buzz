//! Smart HTTP git transport for Buzz.
//!
//! Three endpoints implement the git Smart HTTP protocol:
//! - `GET  /git/{owner}/{repo}/info/refs?service={svc}` — ref advertisement
//! - `POST /git/{owner}/{repo}/git-upload-pack` — clone/fetch
//! - `POST /git/{owner}/{repo}/git-receive-pack` — push
//!
//! Auth: NIP-98 on all routes (clone + push). No public repos for v1.
//! Transport: shells out to `git --stateless-rpc` with `env_clear()`.

use std::path::Path;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use hex;
use serde::Deserialize;
use tokio::process::Command;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};

use super::cas_publish::{cas_publish, CasError, ParentState};
use super::hook::install_hook;
use super::hydrate::{
    hydrate_for_read, hydrate_for_write, load_manifest_for_read, HydrateError, HydratedRepo,
};
use super::manifest_event::{build_ref_state_event, RefStateInputs};
use crate::state::AppState;

/// Timeout for `info/refs` — ref advertisement is fast (essentially `git show-ref`).
const INFO_REFS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Timeout for pack operations (upload-pack, receive-pack) — large repos need time.
const PACK_OPS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// NIP-98 auth extractor for git routes.
///
/// Validates the `Authorization: Nostr <base64>` header before the request body
/// is read. Same pattern as `AuthenticatedUpload` in media.rs.
///
/// Authorization model: any authenticated pubkey can clone; push authorization
/// is handled by the pre-receive hook (calls back to the internal policy endpoint
/// which checks channel role + protection rules from kind:30617).
pub struct GitAuth {
    /// The authenticated user's public key, extracted from the NIP-98 event.
    pub pubkey: nostr::PublicKey,
}

impl axum::extract::FromRequestParts<Arc<AppState>> for GitAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let method = parts.method.as_str();

        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        "WWW-Authenticate",
                        format!("Nostr realm=\"buzz\", method=\"{method}\""),
                    )
                    .body(Body::from("missing Authorization header"))
                    .unwrap()
            })?;

        let token = auth_header.strip_prefix("Nostr ").ok_or_else(|| {
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(
                    "WWW-Authenticate",
                    format!("Nostr realm=\"buzz\", method=\"{method}\""),
                )
                .body(Body::from("expected Authorization: Nostr <base64>"))
                .unwrap()
        })?;

        let event_bytes = base64::engine::general_purpose::STANDARD
            .decode(token)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(token))
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid base64").into_response())?;
        let event_json = String::from_utf8(event_bytes)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid utf-8").into_response())?;

        // Use configured relay_url as canonical base (don't trust forwarded headers).
        let relay_url = &state.config.relay_url;
        let base_url = relay_url
            .replace("ws://", "http://")
            .replace("wss://", "https://");
        let base_url = base_url.trim_end_matches('/');
        let path_and_query = parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or(parts.uri.path());

        // Repo-root URL verification.
        //
        // The credential helper signs a NIP-98 token with:
        //   u = <repo-root>   (e.g., http://host/git/{owner}/{repo})
        //
        // Git's credential protocol does NOT pass query strings to helpers, so
        // service-scoping (`?service=...`) cannot be implemented at the NIP-98
        // level without protocol changes. The token is repo-scoped, not service-scoped.
        //
        // Security is still provided by:
        // - ±60s timestamp window (limits replay)
        // - HTTPS in production (prevents token theft)
        // - Pre-receive hook for push authorization (role + protection rules)
        // - Endpoint routing (clone/push are different HTTP paths)
        let repo_path = if let Some((prefix, _query)) = path_and_query.split_once("/info/refs") {
            prefix
        } else if let Some(prefix) = path_and_query.strip_suffix("/git-upload-pack") {
            prefix
        } else if let Some(prefix) = path_and_query.strip_suffix("/git-receive-pack") {
            prefix
        } else {
            return Err((StatusCode::BAD_REQUEST, "unrecognized git endpoint").into_response());
        };
        let expected_url = format!("{base_url}{repo_path}");

        // Skip HTTP method check for git routes.
        //
        // Git's credential helper signs with `method=GET` (the initial /info/refs request)
        // then reuses the token for POST (pack data). Method binding can't work here.
        //
        // Security is provided by: service-binding in the URL (clone vs push scoped),
        // ±60s timestamp, and the pre-receive hook for push authorization.
        // We pass the method from the event itself so verify_nip98_event always accepts.
        let event_method = serde_json::from_str::<serde_json::Value>(&event_json)
            .ok()
            .and_then(|v| {
                v["tags"]
                    .as_array()?
                    .iter()
                    .find(|t| t[0].as_str() == Some("method"))?[1]
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| method.to_owned());

        // SECURITY: method intentionally not verified for git routes. The tautological
        // check (event.method == event.method) is deliberate — see comment block above.
        // Git's credential protocol signs once with GET and reuses for POST. The URL tag
        // provides the real security boundary (±60s timestamp + URL lock + HTTPS).

        // body=None: can't buffer streaming pack data to verify payload hash.
        // Token is time-bounded (±60s) and URL-locked — acceptable trade-off.
        let pubkey =
            buzz_auth::nip98::verify_nip98_event(&event_json, &expected_url, &event_method, None)
                .map_err(|e| {
                warn!(error = %e, "git NIP-98 auth failed");
                (StatusCode::UNAUTHORIZED, "NIP-98 auth failed").into_response()
            })?;

        // NOTE: NIP-98 event-ID dedup intentionally NOT implemented here.
        // Git's credential protocol reuses one signed token across multiple requests
        // in a session (info_refs GET → upload-pack/receive-pack POST). Rejecting
        // replayed event IDs would break normal clone/push operations.
        // The ±60s timestamp window + URL scoping + HTTPS transport provide sufficient
        // replay protection for v1. Per-request signing requires protocol changes.

        // Relay membership gate (NIP-43).
        let auth_tag = parts
            .headers
            .get("x-auth-tag")
            .and_then(|v| v.to_str().ok());
        if crate::api::relay_members::enforce_relay_membership(state, pubkey.as_bytes(), auth_tag)
            .await
            .is_err()
        {
            warn!(pubkey = %pubkey.to_hex(), "git: relay membership denied");
            return Err((StatusCode::FORBIDDEN, "restricted: not a relay member").into_response());
        }

        Ok(GitAuth { pubkey })
    }
}

/// Validate URL `(owner, repo)` parameters and return the canonical repo
/// id (= `repo` with any `.git` suffix stripped).
///
/// Security: allowlist characters on both owner (64 lower-hex pubkey) and
/// repo name (`[a-zA-Z0-9._-]{1,64}`, no leading dots, no `..`). The
/// filesystem-path canonicalization that the old persistent-repo
/// implementation needed is no longer relevant — git workspaces are
/// ephemeral tempdirs from `hydrate_for_{read,write}`, not paths under a
/// repo root — but the *name* validation stays because owner/repo are
/// still used as object-store key components via `manifest::pointer_key`.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
fn validate_repo_id<'a>(owner: &str, repo: &'a str) -> Result<&'a str, Response> {
    // Owner must be exactly 64 lowercase hex chars.
    if owner.len() != 64
        || !owner
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err((StatusCode::BAD_REQUEST, "invalid owner").into_response());
    }

    // Strip trailing .git if present.
    let repo_name = repo.strip_suffix(".git").unwrap_or(repo);

    // Repo name: [a-zA-Z0-9._-]{1,64}, no leading dots, no "..".
    if repo_name.is_empty()
        || repo_name.len() > 64
        || repo_name.starts_with('.')
        || repo_name.contains("..")
        || !repo_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid repo name").into_response());
    }

    Ok(repo_name)
}

/// Apply hardened environment to a git subprocess command.
///
/// Clears all inherited env vars, then sets only the minimum required:
/// - `PATH` — so git can find its own helpers
/// - `GIT_HTTP_EXPORT_ALL` — required for Smart HTTP
/// - `GIT_CONFIG_NOSYSTEM=1` — ignore system-wide gitconfig
/// - `GIT_CONFIG_GLOBAL=/dev/null` — prevent reading global gitconfig
/// - `HOME=/dev/null` — prevent reading ~/.gitconfig
pub(crate) fn harden_git_env(cmd: &mut Command) {
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("HOME", "/dev/null");
}

/// Acquire the global git-subprocess semaphore permit, or respond 503.
///
/// Bounds total in-flight git subprocesses across all routes. Returned
/// `OwnedSemaphorePermit` releases automatically on drop, so the caller
/// just binds it for the function scope.
#[allow(clippy::result_large_err)]
fn acquire_git_permit(
    state: &Arc<AppState>,
) -> Result<tokio::sync::OwnedSemaphorePermit, Response> {
    Arc::clone(&state.git_semaphore)
        .try_acquire_owned()
        .map_err(|_| {
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Retry-After", "5")
                .body(Body::from("git service busy"))
                .unwrap()
        })
}

/// Convert a [`HydrateError`] to the HTTP response shape the read+write
/// paths share. Below-pointer failure ⇒ 5xx; pointer-absent is signalled
/// via `Ok(None)` from [`hydrate_for_read`] and never reaches this fn.
fn hydrate_error_to_response(owner: &str, repo: &str, err: HydrateError) -> Response {
    error!(error = %err, owner = %owner, repo = %repo, "hydrate failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "git backend hydration failed",
    )
        .into_response()
}

#[derive(Deserialize)]
/// Query parameters for the `info/refs` endpoint.
pub struct InfoRefsQuery {
    service: String,
}

#[derive(Deserialize)]
/// Path parameters for git repo routes: `{owner}/{repo}`.
pub struct GitRepoParams {
    owner: String,
    repo: String,
}

/// Longest refname the fast path will emit. `is_safe_refname` enforces an
/// alphabet but no length bound; `pkt_line` encodes its payload length in a
/// 4-hex prefix that overflows past `0xffff`. Git's own refname limits sit far
/// below this, so any refname this long is pathological — degrade to the
/// subprocess path rather than risk a malformed length prefix. Generous bound:
/// `<oid> <refname>\n` plus the 4-byte pkt header must stay under `0xffff`, and
/// 4096 leaves vast headroom (git's de-facto practical ceiling is a few hundred
/// bytes total).
const MAX_FAST_PATH_REFNAME_LEN: usize = 4096;

/// Whether the `info/refs` fast path can serve this manifest without shelling
/// out. The manifest carries only `refname → oid`, so it cannot reproduce the
/// `^{}` peel line an **annotated** tag advertises (the peeled commit oid is
/// not stored — see RESEARCH/GIT_REF_ADVERTISEMENT_FORMAT.md). We cannot tell
/// an annotated tag from a lightweight one at the manifest level, so **any**
/// `refs/tags/*` forces the subprocess fallback for byte-correctness. We also
/// require the symbolic `head` to resolve to a ref we actually advertise —
/// otherwise the `symref=HEAD:<ref>` capability would point at a ref the
/// client never sees. The dominant clone case (branches only, HEAD→a branch)
/// takes the fast path; everything else stays exactly as it is today.
///
/// Eligibility is also a **safety gate**: the fast path emits manifest
/// refnames/oids straight into pkt-line bytes, so this predicate re-runs the
/// same `is_safe_refname`/`is_hex_oid` checks the hydrate path applies
/// (hydrate.rs) and the write path applies (`Manifest::validate`). The manifest
/// is already digest-verified against the pointer when loaded, so on every
/// normally-reachable path these re-checks are redundant — but keeping the
/// emit path symmetric with hydrate means an out-of-band-written manifest
/// (migration, manual S3 put, a future writer that skips `validate`) degrades
/// to the subprocess path instead of advertising unchecked bytes. Any failure
/// here → `false` → subprocess fallback, which surfaces the error correctly.
fn fast_path_eligible(manifest: &super::manifest::Manifest) -> bool {
    use super::manifest::{is_hex_oid, is_safe_refname};

    if manifest.refs.keys().any(|r| r.starts_with("refs/tags/")) {
        return false;
    }
    // HEAD must resolve to an advertised ref. (Detached HEAD — head not in
    // refs — can't be expressed as a symref; fall back.)
    if !manifest.refs.contains_key(&manifest.head) {
        return false;
    }
    // Safety re-check: every refname/oid we'd emit must be well-formed and
    // bounded. HEAD is a key in `refs` (checked above), so the loop covers it.
    manifest.refs.iter().all(|(refname, oid)| {
        is_safe_refname(refname) && refname.len() <= MAX_FAST_PATH_REFNAME_LEN && is_hex_oid(oid)
    })
}

/// Largest payload a single pkt-line can carry: the 4-hex length prefix counts
/// itself, so the total frame is bounded to `0xffff` and the payload to
/// `0xffff - 4`.
const PKT_LINE_MAX_PAYLOAD: usize = 0xffff - 4;

/// Encode one pkt-line: 4-char lowercase-hex length prefix (counting itself)
/// followed by `payload`. Appends to `out`.
///
/// The 4-hex prefix can only express a frame length up to `0xffff`. A payload
/// past [`PKT_LINE_MAX_PAYLOAD`] would make `format!("{len:04x}")` emit *five*
/// hex digits — not a truncation but a silent stream corruption (the next
/// reader takes the first four as the length). Callers on the manifest fast
/// path are already gated by [`fast_path_eligible`]'s length cap, but this is
/// the construction boundary: rather than trust every present and future
/// caller, an overlong payload here is dropped (emitting an empty `0004`
/// pkt-line) and logged at `error`, instead of writing a malformed length.
/// Non-panicking in every build profile — the worst case is a ref-short
/// advertisement that fails cleanly client-side, never a corrupted stream.
fn pkt_line(out: &mut Vec<u8>, payload: &[u8]) {
    if payload.len() > PKT_LINE_MAX_PAYLOAD {
        tracing::error!(
            payload_len = payload.len(),
            limit = PKT_LINE_MAX_PAYLOAD,
            "pkt-line payload exceeds 0xffff-4 frame limit; dropping (bug: caller \
             bypassed the fast-path length gate)"
        );
        out.extend_from_slice(b"0004"); // empty pkt-line; never a 5-hex length
        return;
    }
    let len = payload.len() + 4;
    out.extend_from_slice(format!("{len:04x}").as_bytes());
    out.extend_from_slice(payload);
}

/// Build the **complete** `info/refs` HTTP body for `git-upload-pack` directly
/// from the published manifest — no hydrate, no subprocess. Byte-compatible
/// with `git upload-pack --advertise-refs` for the branches-only case that
/// [`fast_path_eligible`] gates on.
///
/// Layout (matches the subprocess oracle, git 2.51):
/// ```text
/// <pkt># service=git-upload-pack\n
/// 0000
/// <pkt><head-oid> HEAD\0<caps> symref=HEAD:<head-ref> object-format=<fmt> agent=buzz-git\n
/// <pkt><oid> <refname>\n        # each ref, sorted ascending (BTreeMap order)
/// 0000
/// ```
/// The advertised capabilities are a fixed conservative **offer**; the client
/// re-negotiates against the real `upload-pack` subprocess in its follow-up
/// POST, so any subset the real upload-pack supports is safe. `object-format`
/// is derived from the oid width (40 hex = sha1, 64 = sha256) rather than
/// hardcoded. Caller guarantees [`fast_path_eligible`] returned true.
fn build_upload_pack_advertisement(manifest: &super::manifest::Manifest) -> Vec<u8> {
    // head ref is guaranteed present by `fast_path_eligible`.
    let head_oid = &manifest.refs[&manifest.head];
    let object_format = if head_oid.len() == 64 {
        "sha256"
    } else {
        "sha1"
    };

    // Capability offer. Conservative, version-agnostic. The symref tells the
    // client which branch HEAD tracks (so `git clone` checks it out).
    let caps = format!(
        "multi_ack thin-pack side-band side-band-64k ofs-delta shallow \
         deepen-since deepen-not deepen-relative no-progress include-tag \
         multi_ack_detailed no-done symref=HEAD:{head} object-format={fmt} \
         agent=buzz-git",
        head = manifest.head,
        fmt = object_format,
    );

    let mut out = Vec::new();

    // 1. service header + flush.
    let svc_line = b"# service=git-upload-pack\n";
    pkt_line(&mut out, svc_line);
    out.extend_from_slice(b"0000");

    // 2. First line: HEAD with NUL-joined caps.
    let mut first = Vec::new();
    first.extend_from_slice(head_oid.as_bytes());
    first.extend_from_slice(b" HEAD\0");
    first.extend_from_slice(caps.as_bytes());
    first.push(b'\n');
    pkt_line(&mut out, &first);

    // 3. Each ref, sorted (BTreeMap iterates ascending — matches git).
    for (refname, oid) in &manifest.refs {
        let mut line = Vec::new();
        line.extend_from_slice(oid.as_bytes());
        line.push(b' ');
        line.extend_from_slice(refname.as_bytes());
        line.push(b'\n');
        pkt_line(&mut out, &line);
    }

    // 4. Trailing flush.
    out.extend_from_slice(b"0000");
    out
}

/// `GET /git/{owner}/{repo}/info/refs?service={service}`
///
/// Advertises refs for clone (git-upload-pack) or push (git-receive-pack).
///
/// **Track C fast path:** for `git-upload-pack` on a branches-only repo
/// ([`fast_path_eligible`]), the advertisement is built directly from the
/// published manifest — **no hydrate, no subprocess, no git semaphore
/// permit**. This is the dominant clone case, and it's exactly what the
/// W=20 permit used to serialize. Repos with any `refs/tags/*`, or the
/// `git-receive-pack` advertisement (different cap set), fall back to the
/// subprocess path — which still acquires a permit and hydrates, preserving
/// today's behavior byte-for-byte.
///
/// **Read fail-closed (Max's blocker):** pointer-absent → 404 (repo
/// never existed). *Any* below-pointer failure (manifest 404 under a
/// non-empty pointer, digest mismatch, pack 404, `index-pack` failure)
/// → 5xx via `HydrateError`. The legacy "leave disk as-is on hydrate
/// error" behavior is gone — A1 detectability holds on the read side too.
pub async fn info_refs(
    State(state): State<Arc<AppState>>,
    _auth: GitAuth,
    AxumPath(params): AxumPath<GitRepoParams>,
    Query(query): Query<InfoRefsQuery>,
) -> Result<Response, Response> {
    // Validate service parameter: exact allowlist.
    let service = match query.service.as_str() {
        "git-upload-pack" | "git-receive-pack" => &query.service,
        _ => return Err((StatusCode::BAD_REQUEST, "invalid service").into_response()),
    };
    let _repo_name = validate_repo_id(&params.owner, &params.repo)?;

    // Track C fast path: only for clone advertisement. The receive-pack
    // advertisement carries a different capability set (report-status,
    // delete-refs, atomic, …) that we don't reproduce, so it always takes
    // the subprocess path below.
    if service == "git-upload-pack" {
        // Load just the verified manifest — no object materialization, no
        // permit. `Ok(None)` = pointer absent = repo never existed → 404.
        match load_manifest_for_read(&state.git_store, &params.owner, &params.repo).await {
            Ok(Some(manifest)) if fast_path_eligible(&manifest) => {
                let body = build_upload_pack_advertisement(&manifest);
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(
                        header::CONTENT_TYPE,
                        "application/x-git-upload-pack-advertisement",
                    )
                    .header(header::CACHE_CONTROL, "no-cache")
                    .body(Body::from(body))
                    .unwrap());
            }
            // Eligible repo but has tags, or below-pointer failure handling:
            // a present-but-ineligible manifest falls through to the
            // subprocess path. Pointer-absent is a definitive 404 here —
            // no point hydrating a repo that doesn't exist.
            Ok(Some(_)) => { /* ineligible (has tags) → subprocess fallback */ }
            Ok(None) => return Err((StatusCode::NOT_FOUND, "repository not found").into_response()),
            Err(e) => return Err(hydrate_error_to_response(&params.owner, &params.repo, e)),
        }
    }

    // Subprocess path: receive-pack advertisement, or upload-pack for a
    // tagged repo. Acquires a permit and hydrates — today's behavior.
    info_refs_subprocess(&state, service, &params).await
}

/// Subprocess-backed `info/refs` advertisement: hydrate the published state
/// into an ephemeral bare repo and shell out to `git <svc> --advertise-refs`.
///
/// This is the fallback from the Track C fast path (tagged repos, and the
/// `git-receive-pack` advertisement). The advertisement is O(refs), not
/// O(pack), so it stays buffered — streaming would buy nothing and would
/// lose the clean timeout/error mapping that buffering gives us.
async fn info_refs_subprocess(
    state: &Arc<AppState>,
    service: &str,
    params: &GitRepoParams,
) -> Result<Response, Response> {
    let _permit = acquire_git_permit(state)?;

    let repo = match hydrate_for_read(&state.git_store, &params.owner, &params.repo).await {
        Ok(Some(repo)) => repo,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "repository not found").into_response()),
        Err(e) => return Err(hydrate_error_to_response(&params.owner, &params.repo, e)),
    };

    // Git's smart HTTP protocol uses service names like "git-upload-pack" and
    // "git-receive-pack", but the actual git subcommands are "upload-pack" and
    // "receive-pack" (without the "git-" prefix).
    let git_subcmd = service.strip_prefix("git-").unwrap_or(service);

    let mut cmd = Command::new("git");
    cmd.arg(git_subcmd)
        .arg("--stateless-rpc")
        .arg("--advertise-refs")
        .arg(repo.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    harden_git_env(&mut cmd);

    let child = cmd.spawn().map_err(|e| {
        error!(error = %e, "git subprocess failed to spawn");
        (StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response()
    })?;

    // kill_on_drop requires a Child handle — .output() doesn't expose one.
    let output = tokio::time::timeout(INFO_REFS_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            warn!(
                "git info_refs subprocess timed out ({}s)",
                INFO_REFS_TIMEOUT.as_secs()
            );
            (StatusCode::GATEWAY_TIMEOUT, "git operation timed out").into_response()
        })?
        .map_err(|e| {
            error!(error = %e, "git subprocess failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response()
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!(stderr = %stderr, "git --advertise-refs failed");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response());
    }
    // `repo` (the tempdir) must live until *after* the subprocess has read
    // its objects. Holding it until here is the structural lifetime that
    // guarantees that.
    drop(repo);

    // Build pkt-line response: service header + flush + git output.
    let svc_line = format!("# service={service}\n");
    let svc_pkt = format!("{:04x}{svc_line}", svc_line.len() + 4);
    let mut body = Vec::with_capacity(svc_pkt.len() + 4 + output.stdout.len());
    body.extend_from_slice(svc_pkt.as_bytes());
    body.extend_from_slice(b"0000"); // flush packet
    body.extend_from_slice(&output.stdout);

    let content_type = format!("application/x-{service}-advertisement");
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(body))
        .unwrap())
}

/// `POST /git/{owner}/{repo}/git-upload-pack`
///
/// Handles clone/fetch — client sends wants/haves, server sends pack data.
///
/// Reads from a tempdir bare repo hydrated from the published manifest;
/// the tempdir lives only for the duration of this request.
pub async fn upload_pack(
    State(state): State<Arc<AppState>>,
    _auth: GitAuth,
    AxumPath(params): AxumPath<GitRepoParams>,
    body: Body,
) -> Result<Response, Response> {
    let _ = validate_repo_id(&params.owner, &params.repo)?;
    let _permit = acquire_git_permit(&state)?;

    let repo = match hydrate_for_read(&state.git_store, &params.owner, &params.repo).await {
        Ok(Some(repo)) => repo,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "repository not found").into_response()),
        Err(e) => return Err(hydrate_error_to_response(&params.owner, &params.repo, e)),
    };

    // Track A: stream the subprocess stdout straight into the response body
    // instead of buffering the whole pack into RAM. `repo` (the hydrated
    // tempdir) is moved into the stream and stays alive until the last byte
    // is drained — the streaming analogue of the old `drop(repo)`. The
    // `_permit` (git semaphore) is bound for the function scope; binding it
    // here keeps it held only until the response head is built, not for the
    // whole drain. That's intentional — see Track C notes on permit hold time.
    stream_git_read(
        repo,
        "upload-pack",
        &[],
        body,
        Vec::new(),
        "application/x-git-upload-pack-result".to_string(),
    )
}

/// `POST /git/{owner}/{repo}/git-receive-pack`
///
/// Handles push — client sends ref updates + pack data.
///
/// Authorization: NIP-98 authenticates the pusher. The pre-receive hook
/// installed into the hydrated tempdir calls back to the internal policy
/// endpoint for ref-level authorization (channel role + protection rules
/// from kind:30617).
///
/// **Push flow (spec §Push steps 1–8):**
/// 1. Validate ids; acquire global git permit (bounds concurrent
///    subprocesses; **no per-repo lock** — writer serialization is the
///    pointer CAS, per spec).
/// 2. `hydrate_for_write` → `(HydratedRepo, ParentState)`. The
///    `ParentState` is the *same* observed pointer state the workspace
///    was hydrated from; it's the CAS predicate at step 7 below, which
///    is what makes `Inv_RefDerivedFromParent` structural rather than a
///    code-review property.
/// 3. `install_hook(repo.path())` — drop the pre-receive script + chmod.
///    Same script, same env contract, same policy callback as today;
///    only the on-disk path is ephemeral.
/// 4. Run `receive-pack --stateless-rpc` against the tempdir. The hook
///    enforces fast-forward + branch protection in-process.
/// 5. `finalize_push(PushContext { pack, parent_state, repo, ... })` is
///    the only path that builds a push `Response`. It calls
///    `cas_publish` (§Push steps 2–7) and emits kind:30618 on `Won`,
///    *only then* builds the 2xx.
pub async fn receive_pack(
    State(state): State<Arc<AppState>>,
    auth: GitAuth,
    AxumPath(params): AxumPath<GitRepoParams>,
    body: Body,
) -> Result<Response, Response> {
    let repo_name = validate_repo_id(&params.owner, &params.repo)?;
    let pusher_hex = hex::encode(auth.pubkey.to_bytes());
    let _permit = acquire_git_permit(&state)?;

    // **No per-repo advisory lock — by design.** Writer serialization is
    // the pointer CAS at `cas_publish` step 7 (`Inv_NoFork` proves this
    // sufficient). The previous code held a per-repo `tokio::Mutex`, but
    // that only spanned one process; once we run >1 relay instance
    // (which is the point of "no local stateful disk"), it spans nothing
    // and CAS is the only serialization that holds. The named tradeoff:
    // two concurrent same-repo pushes each hydrate + run receive-pack,
    // and the loser's CPU/IO is thrown away on `Conflict`. **Accepted
    // for v1** — same-ref contention is rare, and a cross-instance lock
    // would be a distributed-lock service we explicitly don't want.
    // If contention shows up in metrics, the fix is a short local
    // best-effort lock as a *latency optimization*, never a correctness
    // dependency. (Eva's call, on record in #proj-git-on-s3 with the
    // ParentState seam review.)

    // Hydrate parent state + workspace in one round-trip. ParentState
    // travels with the workspace into finalize_push so the CAS predicates
    // on the same pointer ETag the workspace was hydrated from.
    let (repo, parent_state) = hydrate_for_write(&state.git_store, &params.owner, &params.repo)
        .await
        .map_err(|e| hydrate_error_to_response(&params.owner, &params.repo, e))?;

    // Install the pre-receive hook into the ephemeral workspace. The
    // hook script is fixed per-deployment; per-push state (callback URL,
    // HMAC secret, pusher pubkey) rides in env at exec time.
    install_hook(repo.path()).await.map_err(|e| {
        error!(error = %e, "install pre-receive hook into hydrated workspace");
        (StatusCode::INTERNAL_SERVER_ERROR, "git hook install failed").into_response()
    })?;

    // Build hook env vars for the pre-receive hook.
    let hook_url = format!(
        "http://127.0.0.1:{}/internal/git/policy",
        state.config.bind_addr.port()
    );
    let hooks_dir = repo.path().join("hooks").display().to_string();
    let hook_env = vec![
        ("BUZZ_HOOK_URL", hook_url),
        (
            "BUZZ_HOOK_SECRET",
            state.config.git_hook_hmac_secret.clone(),
        ),
        ("BUZZ_REPO_ID", repo_name.to_string()),
        ("BUZZ_REPO_OWNER", params.owner.clone()),
        ("BUZZ_PUSHER_PUBKEY", pusher_hex.clone()),
        // Override any repo-local core.hooksPath setting; defense in
        // depth even though the hydrated workspace has no inherited
        // config.
        ("GIT_CONFIG_COUNT", "1".to_string()),
        ("GIT_CONFIG_KEY_0", "core.hooksPath".to_string()),
        ("GIT_CONFIG_VALUE_0", hooks_dir),
    ];

    // Run receive-pack against the tempdir. Returns the *owned* subprocess
    // output (PackOutput) — crucially NOT a Response, so the post-push
    // fence in finalize_push can sequence the CAS before any 2xx exists.
    let pack = run_git_at(repo.path(), "receive-pack", body, &hook_env).await?;

    let ctx = PushContext {
        pack,
        parent_state,
        owner: params.owner.clone(),
        repo: params.repo.clone(),
        repo_id: repo_name.to_string(),
        pusher: auth.pubkey,
        repo_handle: repo,
    };
    Ok(finalize_push(&state, ctx).await)
}

/// Buffered output of a `git --stateless-rpc` subprocess.
///
/// The handler holds this as an owned value between subprocess completion
/// and response construction — this is the *structural seam* the post-push
/// fence relies on (see §Implementation Correspondence in
/// `docs/git-on-object-storage.md`): nothing reaches the client until
/// [`finalize_push`] has decided to build a `Response` from these bytes.
pub(crate) struct PackOutput {
    pub stdout: Vec<u8>,
}

/// Spawn a `git --stateless-rpc <service>` subprocess against the given
/// path, stream the request body to stdin, and return the buffered
/// stdout/stderr/exit status as a [`PackOutput`].
///
/// Critically returns the **owned** subprocess output rather than a
/// `Response`, so callers can sequence post-subprocess work (the push
/// fence) before any byte reaches the client.
async fn run_git_at(
    repo_path: &Path,
    service: &str,
    body: Body,
    extra_env: &[(&str, String)],
) -> Result<PackOutput, Response> {
    let mut cmd = Command::new("git");
    cmd.arg(service)
        .arg("--stateless-rpc")
        .arg(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    harden_git_env(&mut cmd);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().map_err(|e| {
        error!(error = %e, service = %service, "git subprocess failed to spawn");
        (StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response()
    })?;

    // Stream request body to git stdin.
    let mut stdin = child.stdin.take().unwrap();
    let body_task = tokio::spawn(async move {
        use futures_util::StreamExt;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if tokio::io::AsyncWriteExt::write_all(&mut stdin, &bytes)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        drop(stdin); // close stdin → EOF for git
    });
    let body_abort = body_task.abort_handle();

    let timeout_result = tokio::time::timeout(PACK_OPS_TIMEOUT, async {
        let _ = body_task.await;
        child.wait_with_output().await
    })
    .await;

    let output = match timeout_result {
        Err(_elapsed) => {
            body_abort.abort();
            warn!(service = %service, timeout_secs = PACK_OPS_TIMEOUT.as_secs(), "git subprocess timed out");
            return Err((StatusCode::GATEWAY_TIMEOUT, "git operation timed out").into_response());
        }
        Ok(Err(e)) => {
            error!(error = %e, service = %service, "git subprocess failed");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response());
        }
        Ok(Ok(out)) => out,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(stderr = %stderr, service = %service, "git subprocess exited with error");
        // Still return output — git protocol errors are communicated in-band.
    }

    Ok(PackOutput {
        stdout: output.stdout,
    })
}

/// Keeps the git subprocess and its hydrated workspace alive for exactly as
/// long as the response body is being streamed.
///
/// The HTTP body is a [`tokio_util::io::ReaderStream`] over the child's
/// stdout. That borrows nothing it can keep alive on its own: the `Child`
/// (whose `kill_on_drop` would otherwise reap the process mid-stream) and the
/// [`HydratedRepo`] tempdir (whose objects the subprocess is still reading)
/// must outlive the last byte. We park both here and drop them only when the
/// stream is exhausted or the client disconnects — the structural analogue of
/// the `drop(repo)` the buffered path did after `wait_with_output`.
///
/// Why streaming is safe here but **not** on the push path: these are
/// read-only operations (`upload-pack`, `info/refs --advertise-refs`). They
/// never mutate published state, so there is no fence to preserve — contrast
/// `receive_pack`, which must buffer into [`PackOutput`] so [`finalize_push`]
/// can sequence the pointer CAS *before* any 2xx byte exists.
struct StreamingGit {
    inner: tokio_util::io::ReaderStream<tokio::process::ChildStdout>,
    /// Held purely to extend lifetime. `kill_on_drop(true)` means dropping
    /// this after the stream completes reaps any lingering process; on the
    /// happy path the child has already exited by then.
    _child: tokio::process::Child,
    /// The ephemeral bare repo the subprocess reads objects from. Must not be
    /// removed from disk until the subprocess is done — i.e. until the stream
    /// ends.
    _repo: HydratedRepo,
}

impl futures_util::Stream for StreamingGit {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.inner).poll_next(cx)
    }
}

/// Spawn a read-only `git --stateless-rpc <service>` subprocess and return a
/// streaming [`Response`] whose body is the child's stdout, optionally
/// preceded by `prefix` bytes (used by `info/refs` for the
/// `# service=…\n0000` pkt-line header).
///
/// The request `body` is pumped to the child's stdin concurrently. The
/// returned response owns the child + the hydrated workspace via
/// [`StreamingGit`], so neither is torn down until the body is fully drained.
///
/// **Read-path only.** Errors after the response head is sent cannot change
/// the status code (it's already 200), which is exactly git's smart-HTTP
/// contract: protocol-level failures are reported in-band within the pack
/// stream, not via HTTP status. The buffered [`run_git_at`] stays the push
/// path's runner precisely because the fence needs the bytes in hand before
/// committing to a status.
#[allow(clippy::result_large_err)]
fn stream_git_read(
    repo: HydratedRepo,
    service: &'static str,
    extra_args: &[&str],
    body: Body,
    prefix: Vec<u8>,
    content_type: String,
) -> Result<Response, Response> {
    let mut cmd = Command::new("git");
    cmd.arg(service).arg("--stateless-rpc");
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.arg(repo.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    harden_git_env(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        error!(error = %e, service = %service, "git subprocess failed to spawn");
        (StatusCode::INTERNAL_SERVER_ERROR, "git error").into_response()
    })?;

    // Pump the request body into git's stdin, then close it (EOF). Detached:
    // the task ends on its own when the body ends or the write fails.
    let mut stdin = child.stdin.take().expect("stdin piped");
    tokio::spawn(async move {
        use futures_util::StreamExt;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if tokio::io::AsyncWriteExt::write_all(&mut stdin, &bytes)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        drop(stdin); // close stdin → EOF for git
    });

    let stdout = child.stdout.take().expect("stdout piped");
    let git_stream = StreamingGit {
        inner: tokio_util::io::ReaderStream::new(stdout),
        _child: child,
        _repo: repo,
    };

    // Prepend any protocol header (info/refs) ahead of git's stdout. The
    // prefix is a single ready chunk; the rest streams from the subprocess.
    let prefix_stream =
        futures_util::stream::once(
            async move { Ok::<_, std::io::Error>(bytes::Bytes::from(prefix)) },
        );
    let body_stream = futures_util::StreamExt::chain(prefix_stream, git_stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(body_stream))
        .unwrap())
}

/// Build the canonical `application/x-git-{service}-result` response from
/// a completed subprocess. For the push path this is **only** reached via
/// [`finalize_push`], which is the unique constructor of a push 2xx — the
/// structural fence.
fn build_git_response(service: &str, output: PackOutput) -> Response {
    let content_type = format!("application/x-git-{service}-result");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(output.stdout))
        .unwrap()
}

/// Per-push state captured between subprocess completion and response
/// construction. Constructing a `PushContext` is the only path from a
/// push subprocess to a 2xx push response (see [`finalize_push`]) — the
/// structural fence (spec Theorem 1).
pub(crate) struct PushContext {
    pub pack: PackOutput,
    /// Parent pointer state observed at hydrate time. The CAS in
    /// `cas_publish` predicates on `parent_state.if_match`, so a
    /// concurrent writer that advanced the pointer between hydrate and
    /// CAS surfaces as `Conflict`/HTTP 409 — `Inv_RefDerivedFromParent`
    /// is structural, not a code-review property.
    pub parent_state: ParentState,
    pub owner: String,
    /// Raw URL repo segment (may include `.git`).
    pub repo: String,
    /// Stripped repo id (= `repo` with any `.git` suffix removed). Used
    /// as the `d` tag on kind:30618 — must match the kind:30617 author's
    /// `d` exactly.
    pub repo_id: String,
    pub pusher: nostr::PublicKey,
    /// The hydrated workspace handle. Held until response construction
    /// (which happens *after* `cas_publish` returns) so the tempdir
    /// outlives the receive-pack subprocess and the CAS publish.
    pub repo_handle: HydratedRepo,
}

/// Finalize a push request: CAS-commit the new state into the object
/// store, derive kind:30618 from the committed manifest, and only then
/// build the success response.
///
/// **The fence (Theorem 1):** the success response is constructed only
/// after `cas_publish` returns `Ok(CasSuccess)`. Lost-race / conflict /
/// backend failure all return *without* a 2xx. This is the unique
/// constructor of a push 2xx, so the seam is structural (not by
/// convention).
async fn finalize_push(state: &Arc<AppState>, ctx: PushContext) -> Response {
    // Step 7 (CAS). The PushContext binds `parent_state` (observed at
    // hydrate) to the CAS predicate here — no re-reading of the pointer
    // between hydrate and CAS.
    let success = match cas_publish(
        &state.git_store,
        ctx.repo_handle.path(),
        &ctx.owner,
        &ctx.repo,
        &ctx.parent_state,
    )
    .await
    {
        Ok(s) => s,
        Err(CasError::Conflict {
            winner_manifest_key,
            ..
        }) => {
            warn!(
                owner = %ctx.owner,
                repo = %ctx.repo,
                winner = %winner_manifest_key,
                "push lost CAS race; tempdir dropped, returning 409"
            );
            return (
                StatusCode::CONFLICT,
                "push superseded by a concurrent writer; pull and retry",
            )
                .into_response();
        }
        Err(CasError::ManifestInvalid(e)) => {
            // 4xx-class: the workspace produced refs/HEAD/oids the
            // manifest validator rejects (unsafe refname, malformed oid,
            // empty head, malformed parent). Pre-CAS — no pointer was
            // written.
            warn!(
                owner = %ctx.owner,
                repo = %ctx.repo,
                error = %e,
                "push rejected: manifest validation failed"
            );
            return (
                StatusCode::BAD_REQUEST,
                "push produced invalid manifest state",
            )
                .into_response();
        }
        Err(e) => {
            // 5xx-class: ManifestReadFailed (parent corruption),
            // Backend, PackCapture. The tempdir drops on scope exit; no
            // pointer was written (or, on rare ManifestReadFailed during
            // winner-fetch, the winner is already installed and the
            // loser's data is unrelated).
            error!(
                owner = %ctx.owner,
                repo = %ctx.repo,
                error = %e,
                "push failed pre-response"
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, "git backend error").into_response();
        }
    };

    // Derived after CAS: kind:30618 ref-state event over the *committed*
    // manifest's refs/head. Spec §Implementation Correspondence:
    // "kind:30618 is derived after CAS, never the commit." We emit only
    // when the committed manifest differs from the parent — a true no-op
    // push pays no 30618 cost.
    //
    // **Strict no-op detection.** We emit unless the canonical manifest
    // is byte-identical to the parent (Dawn's `canonical_bytes` is
    // deterministic, so equal published state ⇒ equal digest by
    // construction). The cases this differs from "refs+head equality":
    // pack-only changes (rare; internal recompaction, or a push that
    // produces a new pack covering existing tips with different deltas)
    // would emit a 30618 with identical `(refs, head)`. The relay DB's
    // `Ok((_, false))` arm below dedups it for free — one extra DB
    // round-trip per pack-only push, which clients don't normally
    // generate. Tightening to refs+head equality is a future
    // micro-optimization only if that dedup cost becomes visible.
    let parent_digest_str: Option<&str> = ctx.parent_state.parent_digest.as_deref();
    let after_digest = success.manifest_key.strip_prefix("manifests/");
    let manifest_changed = match (parent_digest_str, after_digest) {
        (Some(before), Some(after)) => before != after,
        _ => true, // first push (parent None) or impossible-shape after key → publish
    };
    if manifest_changed {
        let inputs = RefStateInputs {
            repo_id: &ctx.repo_id,
            head: &success.manifest.head,
            refs: &success.manifest.refs,
            actor_pubkey_hex: &hex::encode(ctx.pusher.to_bytes()),
        };
        match build_ref_state_event(&inputs, &state.relay_keypair) {
            Ok(event) => match state.db.insert_event(&event, None).await {
                Ok((stored, true)) => {
                    // Routed through the guarded send path for uniformity; the
                    // access gate no-ops for this globally-scoped
                    // (channel_id = None) ref-state event.
                    crate::handlers::event::fan_out_event_to_local_subscribers(state, &stored)
                        .await;
                    info!(
                        owner = %ctx.owner,
                        repo = %ctx.repo_id,
                        manifest = %success.manifest_key,
                        "kind:30618 published (derived after CAS)"
                    );
                }
                Ok((_, false)) => {
                    info!(
                        owner = %ctx.owner,
                        repo = %ctx.repo_id,
                        "kind:30618 deduplicated by relay db"
                    );
                }
                Err(e) => {
                    warn!(
                        owner = %ctx.owner,
                        repo = %ctx.repo_id,
                        error = %e,
                        "kind:30618 insert failed; push remains durable in object store"
                    );
                }
            },
            Err(e) => {
                warn!(
                    owner = %ctx.owner,
                    repo = %ctx.repo_id,
                    error = %e,
                    "kind:30618 build failed; push remains durable in object store"
                );
            }
        }
    }

    // Only now — after CAS commit and (optional) 30618 emission — build
    // the 2xx. The tempdir's lifetime is tied to `ctx.repo_handle`, which
    // we drop after building the response so the subprocess output bytes
    // are fully consumed before the on-disk objects vanish.
    let response = build_git_response("receive-pack", ctx.pack);
    drop(ctx.repo_handle);
    response
}

/// Build the git sub-router with its own body limit.
///
/// Mounted at `/git/{owner}/{repo}/...` with a configurable max pack size.
pub fn git_router(state: Arc<AppState>) -> Router {
    let body_limit = state.config.git_max_pack_bytes as usize;

    Router::new()
        .route("/git/{owner}/{repo}/info/refs", get(info_refs))
        .route("/git/{owner}/{repo}/git-upload-pack", post(upload_pack))
        .route("/git/{owner}/{repo}/git-receive-pack", post(receive_pack))
        .layer(RequestBodyLimitLayer::new(body_limit))
        .with_state(state)
}

#[cfg(test)]
mod track_c_tests {
    use super::*;
    use crate::api::git::manifest::Manifest;
    use std::collections::BTreeMap;

    fn oid_sha1() -> String {
        "cb09a769da1c01f458fa6959d4e8eded38fac8d3".to_string()
    }

    fn branches_only_manifest() -> Manifest {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/feature".to_string(), oid_sha1());
        refs.insert("refs/heads/main".to_string(), oid_sha1());
        Manifest {
            version: 1,
            head: "refs/heads/main".to_string(),
            refs,
            packs: vec!["packs/deadbeef".to_string()],
            parent: None,
        }
    }

    /// Split a pkt-line stream into `(len_prefix, payload)` frames, validating
    /// that each 4-hex length counts itself and that `0000` is a flush.
    fn parse_pkt_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let len_hex = std::str::from_utf8(&bytes[i..i + 4]).unwrap();
            let len = usize::from_str_radix(len_hex, 16).unwrap();
            if len == 0 {
                out.push(Vec::new()); // flush marker
                i += 4;
                continue;
            }
            assert!(len >= 4, "pkt-line length must count its own 4 bytes");
            let payload = bytes[i + 4..i + len].to_vec();
            out.push(payload);
            i += len;
        }
        assert_eq!(i, bytes.len(), "pkt-line stream must consume exactly");
        out
    }

    #[test]
    fn fast_path_eligible_branches_only() {
        assert!(fast_path_eligible(&branches_only_manifest()));
    }

    #[test]
    fn fast_path_rejects_any_tag() {
        let mut m = branches_only_manifest();
        m.refs.insert("refs/tags/v1".to_string(), oid_sha1());
        // Any tag → subprocess fallback (can't peel annotated tags from manifest).
        assert!(!fast_path_eligible(&m));
    }

    #[test]
    fn fast_path_rejects_head_not_in_refs() {
        let mut m = branches_only_manifest();
        m.head = "refs/heads/nonexistent".to_string();
        // HEAD must resolve to an advertised ref to emit symref=HEAD:<ref>.
        assert!(!fast_path_eligible(&m));
    }

    #[test]
    fn fast_path_rejects_unsafe_refname() {
        let mut m = branches_only_manifest();
        // A pkt-line-injecting refname (newline) must never reach the emit path;
        // eligibility is the safety gate → subprocess fallback re-validates.
        m.refs.insert("refs/heads/evil\nx".to_string(), oid_sha1());
        assert!(!fast_path_eligible(&m));
    }

    #[test]
    fn fast_path_rejects_malformed_oid() {
        let mut m = branches_only_manifest();
        // A non-hex / wrong-length oid must degrade to subprocess, not be emitted.
        m.refs
            .insert("refs/heads/bad".to_string(), "not-a-valid-oid".to_string());
        assert!(!fast_path_eligible(&m));
    }

    #[test]
    fn fast_path_rejects_overlong_refname() {
        let mut m = branches_only_manifest();
        // A refname past MAX_FAST_PATH_REFNAME_LEN would overflow the 4-hex
        // pkt-line length prefix; degrade to subprocess instead.
        let long = format!("refs/heads/{}", "a".repeat(MAX_FAST_PATH_REFNAME_LEN));
        m.refs.insert(long, oid_sha1());
        assert!(!fast_path_eligible(&m));
    }

    #[test]
    fn pkt_line_encodes_max_payload_without_overflow() {
        // The largest payload that still fits a 4-hex frame length emits a
        // single valid `ffff` (= 0xffff) frame — the boundary the guard
        // protects, exercised on the safe side.
        let payload = vec![b'a'; PKT_LINE_MAX_PAYLOAD];
        let mut out = Vec::new();
        pkt_line(&mut out, &payload);
        assert_eq!(&out[..4], b"ffff", "frame length prefix");
        assert_eq!(out.len(), 4 + PKT_LINE_MAX_PAYLOAD, "no truncation");
        let frames = parse_pkt_lines(&out);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), PKT_LINE_MAX_PAYLOAD);
    }

    // The overlong-payload guard degrades to an empty `0004` pkt-line in every
    // build profile (no 5-hex length, stream stays parseable) and logs at
    // `error`. Non-panicking, so it's testable directly.
    #[test]
    fn pkt_line_overlong_payload_degrades_to_empty_frame() {
        let payload = vec![b'a'; PKT_LINE_MAX_PAYLOAD + 1];
        let mut out = Vec::new();
        pkt_line(&mut out, &payload);
        // Empty pkt-line, payload dropped — never a malformed 5-hex length.
        assert_eq!(out, b"0004");
    }

    #[test]
    fn advertisement_framing_matches_git_oracle_shape() {
        let body = build_upload_pack_advertisement(&branches_only_manifest());
        let frames = parse_pkt_lines(&body);

        // Layout: [service header, flush, HEAD line, feature, main, flush]
        assert_eq!(frames.len(), 6, "frame count");

        // 0: service header
        assert_eq!(frames[0], b"# service=git-upload-pack\n");
        // 1: flush after service header
        assert!(frames[1].is_empty());

        // 2: HEAD line — "<oid> HEAD\0<caps>\n"
        let head = &frames[2];
        let nul = head.iter().position(|&b| b == 0).expect("NUL in HEAD line");
        assert_eq!(&head[..nul], format!("{} HEAD", oid_sha1()).as_bytes());
        let caps = std::str::from_utf8(&head[nul + 1..head.len() - 1]).unwrap();
        assert_eq!(*head.last().unwrap(), b'\n');
        // symref + object-format are the load-bearing caps.
        assert!(caps.contains("symref=HEAD:refs/heads/main"));
        assert!(caps.contains("object-format=sha1"));
        assert!(caps.contains("side-band-64k"));

        // 3,4: refs sorted ascending — feature before main (BTreeMap order),
        // each "<oid> <refname>\n", no NUL.
        assert_eq!(
            frames[3],
            format!("{} refs/heads/feature\n", oid_sha1()).into_bytes()
        );
        assert_eq!(
            frames[4],
            format!("{} refs/heads/main\n", oid_sha1()).into_bytes()
        );

        // 5: trailing flush
        assert!(frames[5].is_empty());
    }

    #[test]
    fn advertisement_picks_sha256_from_oid_width() {
        let mut m = branches_only_manifest();
        let oid256 = "a".repeat(64);
        m.refs.insert("refs/heads/main".to_string(), oid256.clone());
        m.refs.insert("refs/heads/feature".to_string(), oid256);
        let body = build_upload_pack_advertisement(&m);
        let caps = String::from_utf8_lossy(&body);
        assert!(caps.contains("object-format=sha256"));
    }
}
