//! Default-branch management of the authoritative Git manifest.
//!
//! This is a Git control-plane operation, not a replaceable announcement: it
//! commits through the governed land, shared with receive-pack. A separate
//! strict NIP-98 request prevents reusable Smart HTTP credentials authorizing
//! metadata changes (URL, method, payload and replay are all checked).

// Response is the natural error type for axum handlers.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use buzz_core::{governance::RepoCoordinate, TenantContext};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    manifest::{is_safe_refname, next_published_at, Manifest},
    manifest_event::{build_ref_state_event, RefStateInputs},
    store::{ETag, GitStore},
    transport::{authorize_git_read, deny_banned_git_principal, validate_repo_id},
};
use crate::{
    api::{api_error, bridge, relay_members},
    nip_fi_http::admit_nip_fi_http_on_state,
    state::AppState,
};

fn error(status: StatusCode, message: &str) -> Response {
    api_error(status, message).into_response()
}

fn backend(error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "git settings backend failure");
    self::error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "git settings backend unavailable; read the default branch before retrying",
    )
}

const STALE: &str = "repository changed concurrently; read the latest manifest and retry";

fn conflict() -> Response {
    error(StatusCode::CONFLICT, STALE)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefaultBranch {
    branch: String,
    expected_manifest: String,
}

/// The committed state a request read. It cannot be rebound to another
/// repository or refreshed before the land, which commits only if the
/// repository is still at `digest`.
struct DefaultBranchSnapshot {
    tenant: TenantContext,
    repo: RepoCoordinate,
    etag: ETag,
    digest: String,
    manifest: Manifest,
}

impl DefaultBranchSnapshot {
    async fn load(
        state: &AppState,
        tenant: &TenantContext,
        repo: RepoCoordinate,
    ) -> Result<Self, Response> {
        let (etag, digest, manifest) = super::land::committed_for_write(state, tenant, &repo)
            .await?
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "repository has no published Git state; push a branch first",
                )
            })?;
        Ok(Self {
            tenant: tenant.clone(),
            repo,
            etag,
            digest,
            manifest,
        })
    }

    /// Record that `reader` is shown this snapshot.
    async fn record_read(
        &self,
        state: &AppState,
        reader: &nostr::PublicKey,
    ) -> Result<(), Response> {
        super::transport::record_read(
            &state.db,
            self.tenant.community(),
            &self.repo,
            reader,
            super::reads::Door::State,
        )
        .await
    }

    fn response(&self) -> Value {
        json!({"branch": self.manifest.head.strip_prefix("refs/heads/"), "head": self.manifest.head, "manifest": self.digest})
    }

    /// The manifest naming `request.branch` as the default, stored but not
    /// committed; unchanged when it already is the default, or when the
    /// request is stale, which the land refuses.
    async fn compose(
        &self,
        store: &GitStore,
        request: &SetDefaultBranch,
    ) -> Result<(String, Manifest), Response> {
        let head = format!("refs/heads/{}", request.branch);
        if request.branch.is_empty()
            || request.branch.len() > 1024
            || request.branch.starts_with('-')
            || !is_safe_refname(&head)
            || request.branch.ends_with('.')
            || request
                .branch
                .split('/')
                .any(|part| part.starts_with('.') || part.ends_with(".lock"))
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "invalid branch name; use a short branch name such as main or release/v1",
            ));
        }
        if !self.manifest.refs.contains_key(&head) {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "default branch must name an existing published branch",
            ));
        }
        if self.manifest.head == head || self.digest != request.expected_manifest {
            return Ok((self.digest.clone(), self.manifest.clone()));
        }
        let mut manifest = self.manifest.clone();
        manifest.published_at = Some(next_published_at(
            &self.manifest,
            nostr::Timestamp::now().as_secs(),
        ));
        manifest.head = head;
        manifest.parent = Some(self.digest.clone());
        manifest.validate().map_err(backend)?;
        let bytes = manifest.canonical_bytes().map_err(backend)?;
        let key = store.put_manifest(&bytes).await.map_err(backend)?;
        let digest = key
            .strip_prefix("manifests/")
            .ok_or_else(|| backend("invalid manifest key"))?
            .to_string();
        Ok((digest, manifest))
    }

    /// Make `request.branch` the default through the land, as `caller`.
    /// Even a no-op lands: a concurrent push or deletion, or a lost admin
    /// role, must not be reported as a successful setting.
    async fn set(
        self,
        state: &AppState,
        serving_write: &buzz_deletion::ServingWriteGuard,
        caller: &[u8],
        grant: Option<&buzz_core::governance::GitGrant>,
        request: SetDefaultBranch,
    ) -> Result<(Self, bool), Response> {
        let (digest, manifest) = serving_write
            .protect(self.compose(&state.git_store, &request))
            .await
            .map_err(backend)??;
        // A stale request is refused in the land, after the role, so the
        // refusal is receipted and a non-admin learns only that it may not.
        let stale = self.digest != request.expected_manifest;
        let verdict = super::land::land(
            state,
            serving_write.lease(),
            super::land::Change {
                kind: "default_branch",
                repo: &self.repo,
                actor: caller,
                grant,
                seen: Some(&self.digest),
                new: &digest,
                updates: &[],
            },
            async |_tx, authority| {
                Ok(super::gate::Decision {
                    verdict: match authority.role {
                        Some(role) if role.has_at_least(buzz_core::channel::MemberRole::Admin) => {
                            match stale {
                                false => Ok(()),
                                true => Err(refusal(STALE)),
                            }
                        }
                        _ => Err(refusal(MANAGE_DENIED)),
                    },
                    counted: Default::default(),
                    heads: None,
                })
            },
        )
        .await?;
        match verdict {
            super::land::Verdict::Landed => {}
            super::land::Verdict::Superseded => return Err(conflict()),
            super::land::Verdict::Refused(denials)
                if denials.first().is_some_and(|d| d.reason == STALE) =>
            {
                return Err(conflict())
            }
            super::land::Verdict::Refused(denials) => {
                let reason = denials.first().map_or(MANAGE_DENIED, |d| d.reason.as_str());
                return Err(error(StatusCode::FORBIDDEN, reason));
            }
        }
        let changed = digest != self.digest;
        if changed {
            super::land::write_cache(
                &state.git_store,
                &self.tenant,
                &self.repo,
                Some(&self.etag),
                &digest,
            )
            .await;
        }
        Ok((
            Self {
                digest,
                manifest,
                ..self
            },
            changed,
        ))
    }
}

struct SettingsAuth {
    tenant: TenantContext,
    caller: nostr::PublicKey,
    grant: Option<buzz_core::governance::GitGrant>,
}

async fn authenticate(
    state: &Arc<AppState>,
    slot: &crate::access::AccessSlot,
    headers: &HeaderMap,
    path: &str,
    body: Option<&[u8]>,
) -> Result<SettingsAuth, Response> {
    let tenant = crate::nip_fi_shadow::bind_tenant(state, headers)
        .await
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "repository not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let method = if body.is_some() { "POST" } else { "GET" };
    let require_payload = body.is_some();

    // NIP-FI admission: runs NIP-98 extraction, assertion verification, and
    // key pairing in fixed order.  The control-plane route has strict binding:
    //   - `require_auth_token = true`  (NIP-98 always required; no X-Pubkey fallback)
    //   - `require_payload = body.is_some()` (POST bodies must be hash-bound)
    //
    // The router assertion guard already verified the assertion cryptographically;
    // this call pairs the proven NIP-98 pubkey with the assertion's claimed key
    // and checks the deny map.  [FI-TRACE-AUTHORITY-UNIFORM]
    let admission = admit_nip_fi_http_on_state(
        state,
        headers,
        slot,
        bridge::make_nip98_closure_for_admission(
            slot.clone(),
            headers.clone(),
            method,
            url,
            body.map(|b| b.to_vec()),
            true, // require_auth_token: always required for control-plane
            require_payload,
        ),
    )
    .await?;
    let mut caller = bridge::ProvenCaller::new(admission, slot, &tenant);
    let (pubkey, signed_created_at, token_owner) =
        (caller.pubkey, caller.signed_at, caller.token_owner);

    caller
        .rate_limit_and_replay(state, &tenant)
        .await
        .map_err(IntoResponse::into_response)?;
    // One owner, and one grant, per request, as smart Git transport proves them.
    let tag = relay_members::extract_auth_tag_header(headers);
    let membership = relay_members::enforce_git_membership(
        state,
        &tenant,
        &pubkey,
        tag,
        signed_created_at,
        token_owner,
    )
    .await;
    let (owner, grant) = caller
        .check(membership, relay_members::MembershipRefusal::auth_result)
        .map_err(IntoResponse::into_response)?;
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_http_ban_recheck(tenant.community()).await;
    let ban =
        deny_banned_git_principal(&state.db, tenant.community(), &pubkey, owner, token_owner).await;
    caller
        .admit(ban, |refused| {
            crate::access::AuthResult::Denied(match refused.status() {
                StatusCode::FORBIDDEN => crate::access::AuthReason::Banned,
                _ => crate::access::AuthReason::BanCheckUnavailable,
            })
        })
        .map_err(|legacy| match token_owner {
            // A verified agent token gets only NIP-FI's fixed denial, as smart
            // Git transport does, so the response never shows which step refused.
            Some(_) => crate::nip_fi_core::http_denial(if legacy.status().is_server_error() {
                buzz_auth::DenialClass::AuthorizationUnavailable
            } else {
                buzz_auth::DenialClass::AuthorizationDenied
            }),
            None => legacy,
        })?;
    Ok(SettingsAuth {
        tenant,
        caller: pubkey,
        grant,
    })
}

fn refusal(reason: &str) -> Vec<buzz_core::git_perms::Denial> {
    vec![buzz_core::git_perms::Denial {
        ref_name: "HEAD".into(),
        reason: reason.into(),
    }]
}

const MANAGE_DENIED: &str =
    "only an admin of the repository's channel may change its default branch";

async fn get_default_branch(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, Response> {
    let path = format!("/git/{owner}/{repo}/default-branch");
    let repo_name = validate_repo_id(&owner, &repo)?;
    let auth = authenticate(&state, &slot, &headers, &path, None).await?;
    authorize_git_read(
        &state.db,
        auth.tenant.community(),
        &auth.caller,
        auth.grant.as_ref(),
        &owner,
        repo_name,
        !state.config.require_relay_membership,
    )
    .await?;
    let repo = super::land::coordinate(&owner, repo_name);
    let snapshot = DefaultBranchSnapshot::load(&state, &auth.tenant, repo).await?;
    snapshot.record_read(&state, &auth.caller).await?;
    Ok(Json(snapshot.response()))
}

async fn set_default_branch(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, Response> {
    let path = format!("/git/{owner}/{repo}/default-branch");
    let repo_name = validate_repo_id(&owner, &repo)?;
    let auth = authenticate(&state, &slot, &headers, &path, Some(&body)).await?;
    // Readers may ask; only the land decides, so every refusal is receipted.
    authorize_git_read(
        &state.db,
        auth.tenant.community(),
        &auth.caller,
        auth.grant.as_ref(),
        &owner,
        repo_name,
        !state.config.require_relay_membership,
    )
    .await?;
    let request: SetDefaultBranch = serde_json::from_slice(&body).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "expected branch and expected_manifest strings",
        )
    })?;
    let repo = super::land::coordinate(&owner, repo_name);
    let snapshot = DefaultBranchSnapshot::load(&state, &auth.tenant, repo).await?;
    snapshot.record_read(&state, &auth.caller).await?;
    let serving_write = buzz_deletion::acquire_serving_write(
        &state.db,
        auth.tenant.community(),
        "git_default_branch",
    )
    .await
    .map_err(|_| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "community writes are fenced",
        )
    })?;
    let (snapshot, changed) = snapshot
        .set(
            &state,
            &serving_write,
            &auth.caller.to_bytes(),
            auth.grant.as_ref(),
            request,
        )
        .await?;
    let publication = async {
        if changed {
            let actor = auth.caller.to_hex();
            let event = build_ref_state_event(
                &RefStateInputs {
                    repo_id: repo_name,
                    head: &snapshot.manifest.head,
                    refs: &snapshot.manifest.refs,
                    actor_pubkey_hex: &actor,
                    published_at: snapshot.manifest.published_at,
                },
                &state.relay_keypair,
            )
            .map_err(backend)?;
            let (stored, inserted) = state
                .db
                .insert_event_with_serving_write_guard(serving_write.lease(), &event, None)
                .await
                .map_err(backend)?;
            if inserted {
                crate::handlers::event::fan_out_event_to_local_subscribers(
                    &state,
                    auth.tenant.community(),
                    &stored,
                )
                .await;
            }
        }
        Ok::<(), Response>(())
    }
    .await;
    if let Err(error) = serving_write.finish().await {
        tracing::warn!(%error, "failed to release community serving lease after default-branch publication");
    }
    // Publication failure is not mistaken for a rolled-back manifest.
    if publication.is_err() {
        return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "default branch committed but notification failed; read the current default branch before retrying"));
    }
    let mut response = snapshot.response();
    response["changed"] = json!(changed);
    Ok(Json(response))
}

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/git/{owner}/{repo}/default-branch",
            get(get_default_branch).post(set_default_branch),
        )
        .layer(DefaultBodyLimit::max(4096))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
