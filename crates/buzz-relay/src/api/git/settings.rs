//! Default-branch management of the authoritative Git manifest.
//!
//! This is a Git control-plane operation, not a replaceable announcement:
//! the pointer CAS is the commit point, shared with receive-pack. A separate
//! strict NIP-98 request prevents reusable Smart HTTP credentials authorizing
//! metadata changes (URL, method, payload and replay are all checked).

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use buzz_core::TenantContext;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    binding::{resolve_repo_binding, RepoBinding},
    hydrate::load_pointer,
    manifest::{is_safe_refname, pointer_key, Manifest},
    manifest_event::{build_ref_state_event, RefStateInputs},
    store::{CasOutcome, ETag, GitStore, Precond},
    transport::{authorize_git_read, deny_banned_git_principal, validate_repo_id},
};
use crate::{
    api::{api_error, bridge, relay_members},
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

fn conflict() -> Response {
    error(
        StatusCode::CONFLICT,
        "repository changed concurrently; read the latest manifest and retry",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefaultBranch {
    branch: String,
    expected_manifest: String,
}

/// A loaded snapshot cannot be rebound to another pointer or refreshed at CAS.
struct DefaultBranchSnapshot {
    pointer: String,
    etag: ETag,
    digest: String,
    manifest: Manifest,
}

impl DefaultBranchSnapshot {
    async fn load(
        store: &GitStore,
        tenant: &TenantContext,
        owner: &str,
        repo: &str,
    ) -> Result<Self, Response> {
        let (etag, digest, manifest) = load_pointer(store, tenant, owner, repo)
            .await
            .map_err(backend)?
            .ok_or_else(|| {
                error(
                    StatusCode::NOT_FOUND,
                    "repository has no published Git state; push a branch first",
                )
            })?;
        Ok(Self {
            pointer: pointer_key(tenant.community(), owner, repo),
            etag,
            digest,
            manifest,
        })
    }

    fn response(&self) -> Value {
        json!({"branch": self.manifest.head.strip_prefix("refs/heads/"), "head": self.manifest.head, "manifest": self.digest})
    }

    async fn set(
        mut self,
        store: &GitStore,
        request: SetDefaultBranch,
    ) -> Result<(Self, bool), Response> {
        if self.digest != request.expected_manifest {
            return Err(conflict());
        }
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
        let changed = self.manifest.head != head;
        if changed {
            self.manifest.head = head;
            self.manifest.parent = Some(self.digest.clone());
            self.manifest.validate().map_err(backend)?;
            let bytes = self.manifest.canonical_bytes().map_err(backend)?;
            let key = store.put_manifest(&bytes).await.map_err(backend)?;
            self.digest = key
                .strip_prefix("manifests/")
                .ok_or_else(|| backend("invalid manifest key"))?
                .to_string();
        }
        // Even a no-op checks the observed ETag: concurrent deletion/push must
        // not be reported as a successful setting of a now-missing branch.
        match store
            .put_pointer(
                &self.pointer,
                self.digest.as_bytes(),
                Precond::IfMatch(self.etag.clone()),
            )
            .await
            .map_err(backend)?
        {
            CasOutcome::Won(etag) => self.etag = etag,
            CasOutcome::LostRace => return Err(conflict()),
        }
        Ok((self, changed))
    }
}

struct SettingsAuth {
    tenant: TenantContext,
    caller: nostr::PublicKey,
    delegated_owner: Option<nostr::PublicKey>,
}

async fn authenticate(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: Option<&[u8]>,
) -> Result<SettingsAuth, Response> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, host)
        .await
        .map_err(|_| error(StatusCode::NOT_FOUND, "repository not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let auth = bridge::verify_bridge_auth_with_options(
        headers,
        if body.is_some() { "POST" } else { "GET" },
        &url,
        body,
        true,
        body.is_some(),
    )
    .map_err(IntoResponse::into_response)?;
    bridge::enforce_http_admission(state, &tenant, &auth.pubkey)
        .await
        .map_err(IntoResponse::into_response)?;
    bridge::check_nip98_replay(state, &tenant, auth.event_id_bytes)
        .await
        .map_err(IntoResponse::into_response)?;
    let tag = relay_members::extract_auth_tag_header(headers);
    relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        auth.pubkey.as_bytes(),
        tag,
        auth.signed_created_at,
    )
    .await
    .map_err(IntoResponse::into_response)?;
    deny_banned_git_principal(
        &state.db,
        tenant.community(),
        &auth.pubkey,
        tag,
        auth.signed_created_at,
    )
    .await?;
    // Admission ignores kind= restrictions by design (NIP-AA). Repository
    // management must not turn a message-only credential into write authority.
    // This HTTP operation has no event kind: only kind-unrestricted credentials
    // may inherit management authority. Temporal clauses are still enforced.
    let delegated_owner = tag
        .filter(|tag| {
            serde_json::from_str::<Vec<String>>(tag)
                .ok()
                .and_then(|parts| parts.get(2).cloned())
                .is_some_and(|conditions| {
                    !conditions
                        .split('&')
                        .any(|clause| clause.starts_with("kind="))
                })
        })
        .and_then(|tag| {
            relay_members::extract_nip_oa_owner(
                auth.pubkey.as_bytes(),
                Some(tag),
                auth.signed_created_at,
            )
        });
    Ok(SettingsAuth {
        tenant,
        caller: auth.pubkey,
        delegated_owner,
    })
}

async fn authorize_management(
    state: &AppState,
    auth: &SettingsAuth,
    repo: &nostr::Event,
) -> Result<(), Response> {
    let RepoBinding::Bound(channel) = resolve_repo_binding(repo) else {
        return Err(error(StatusCode::NOT_FOUND, "repository not found"));
    };
    let community = auth.tenant.community();
    let bound = state
        .db
        .get_channel(community, channel)
        .await
        .map_err(backend)?;
    if bound.archived_at.is_some() {
        return Err(error(
            StatusCode::FORBIDDEN,
            "channel is archived (read-only)",
        ));
    }
    let named_manager = |key: &nostr::PublicKey| {
        *key == repo.pubkey
            || repo.tags.iter().any(|tag| {
                let values = tag.as_slice();
                values.first().is_some_and(|name| name == "maintainers")
                    && values
                        .iter()
                        .skip(1)
                        .any(|value| nostr::PublicKey::parse(value).ok().as_ref() == Some(key))
            })
    };
    // Direct authority is independent of an optional owner credential.
    if named_manager(&auth.caller)
        || state
            .db
            .is_agent_owner(community, repo.pubkey.as_bytes(), auth.caller.as_bytes())
            .await
            .map_err(backend)?
    {
        return Ok(());
    }
    if let Some(principal) = &auth.delegated_owner {
        let role = state
            .db
            .get_member_role(community, channel, principal.as_bytes())
            .await
            .map_err(backend)?;
        if role.is_some_and(|r| r.parse::<buzz_core::channel::MemberRole>().is_ok())
            && (named_manager(principal)
                || state
                    .db
                    .is_agent_owner(community, repo.pubkey.as_bytes(), principal.as_bytes())
                    .await
                    .map_err(backend)?)
        {
            return Ok(());
        }
    }
    Err(error(
        StatusCode::FORBIDDEN,
        "only the repository owner or a named maintainer may change its default branch",
    ))
}

async fn get_default_branch(
    State(state): State<Arc<AppState>>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, Response> {
    let path = format!("/git/{owner}/{repo}/default-branch");
    let repo_name = validate_repo_id(&owner, &repo)?;
    let auth = authenticate(&state, &headers, &path, None).await?;
    authorize_git_read(
        &state.db,
        auth.tenant.community(),
        &auth.caller,
        &owner,
        repo_name,
    )
    .await?;
    let snapshot =
        DefaultBranchSnapshot::load(&state.git_store, &auth.tenant, &owner, repo_name).await?;
    Ok(Json(snapshot.response()))
}

async fn set_default_branch(
    State(state): State<Arc<AppState>>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, Response> {
    let path = format!("/git/{owner}/{repo}/default-branch");
    let repo_name = validate_repo_id(&owner, &repo)?;
    let auth = authenticate(&state, &headers, &path, Some(&body)).await?;
    let announcement = authorize_git_read(
        &state.db,
        auth.tenant.community(),
        &auth.caller,
        &owner,
        repo_name,
    )
    .await?;
    authorize_management(&state, &auth, &announcement).await?;
    let request: SetDefaultBranch = serde_json::from_slice(&body).map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "expected branch and expected_manifest strings",
        )
    })?;
    let snapshot =
        DefaultBranchSnapshot::load(&state.git_store, &auth.tenant, &owner, repo_name).await?;
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
    serving_write.verify().await.map_err(backend)?;
    let (snapshot, changed) = serving_write
        .protect(snapshot.set(&state.git_store, request))
        .await
        .map_err(backend)??;
    let publication = async {
        if changed {
            let actor = auth.caller.to_hex();
            let event = build_ref_state_event(
                &RefStateInputs {
                    repo_id: repo_name,
                    head: &snapshot.manifest.head,
                    refs: &snapshot.manifest.refs,
                    actor_pubkey_hex: &actor,
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
    serving_write.finish().await.map_err(backend)?;
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
