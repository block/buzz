//! Relay-operator community provisioning HTTP handler support.
//!
//! ## Authorization: operator, not owner
//!
//! Every other admin surface in this relay is community-scoped — the sender's
//! role is looked up in `relay_members (community_id, pubkey)` for the
//! host-resolved tenant. Community *creation* cannot work that way: its effect
//! is the creation of tenancy itself, so the authorizing identity must sit
//! above tenants. The gate here is the deployment-level
//! `RELAY_OPERATOR_PUBKEYS` allowlist (see `Config::relay_operator_pubkeys`).
//! An empty allowlist (the default) disables provisioning entirely.
//!
//! The public surface is `POST /operator/communities`, authenticated by NIP-98
//! and gated by the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist. The
//! endpoint is intentionally outside the Nostr event ingest data plane: no
//! relay-membership bypass, no special event kind, no storage or fan-out.
//!
//! ## Request shape
//!
//! ```json
//! { "host": "acme.communities.buzz.xyz", "initial_owner_pubkey": "<hex>" }
//! ```
//!
//! `initial_owner_pubkey` is optional. Existing-community convergence may
//! rotate ownership only while the community is active and has no durable
//! deletion intent. Archive freezes ownership.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info;

use buzz_core::tenant::{normalize_host, TenantContext};
use url::{Host, Url};

use crate::state::AppState;

/// Maximum accepted authority length. Matches `communities.host VARCHAR(255)`.
const MAX_HOST_LEN: usize = 255;

/// JSON body for `POST /operator/communities`.
#[derive(Debug, Deserialize)]
pub struct ProvisionCommunityRequest {
    /// Normalized authority for the community to ensure/create.
    pub host: String,
    /// Optional initial owner pubkey. When set on an existing community this
    /// rotates the owner through the same bootstrap path used at startup.
    #[serde(default)]
    pub initial_owner_pubkey: Option<String>,
    /// When true, atomically create the host and owner and reject an existing
    /// host instead of converging or rotating ownership.
    #[serde(default)]
    pub create_only: bool,
}

/// JSON response from `POST /operator/communities`.
#[derive(Debug, Serialize)]
pub struct ProvisionCommunityResponse {
    /// UUID of the ensured/created community.
    pub community_id: String,
    /// Canonical host stored on the community row.
    pub host: String,
    /// `created` when the host row was inserted, `existed` when it was already
    /// present and the request converged idempotently.
    pub status: &'static str,
    /// Echoes the validated owner pubkey when an owner bootstrap/rotation ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_pubkey: Option<String>,
}

pub(crate) fn validate_pubkey_hex(value: &str) -> Option<String> {
    let normalized = value.to_ascii_lowercase();
    (normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(normalized)
}

/// Validate a normalized host authority value for a community.
///
/// The host must already be in normalized shape (`normalize_host` is a
/// no-op on it): lowercase, no default port, no trailing dot. Requiring the
/// caller to send the normalized form keeps the stored `communities.host`
/// value byte-identical to what request-time host resolution will look up.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if normalize_host(host) != host {
        return Err(format!(
            "host is not normalized: expected {:?}",
            normalize_host(host)
        ));
    }
    validate_authority(host)
}

fn validate_authority(authority: &str) -> Result<(), String> {
    if authority
        .chars()
        .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("host contains invalid characters".to_string());
    }
    if authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
    {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    // Parse as an HTTP authority by wrapping it in a URL. This rejects empty
    // hosts, malformed bracketed IPv6 literals, and invalid ports while keeping
    // the accepted shape aligned with request `Host` authority syntax.
    let parsed = Url::parse(&format!("http://{authority}/"))
        .map_err(|_| "host is not a valid authority".to_string())?;
    let host = parsed
        .host()
        .ok_or_else(|| "host is not a valid authority".to_string())?;

    let serialized_host = match host {
        Host::Domain(domain) => {
            validate_domain_labels(domain)?;
            domain.to_string()
        }
        Host::Ipv4(addr) => addr.to_string(),
        Host::Ipv6(addr) => format!("[{addr}]"),
    };
    let canonical_authority = match parsed.port() {
        Some(port) => format!("{serialized_host}:{port}"),
        None => serialized_host,
    };

    if canonical_authority != authority {
        return Err(format!(
            "host is not a canonical authority: expected {canonical_authority:?}"
        ));
    }

    Ok(())
}

fn validate_domain_labels(domain: &str) -> Result<(), String> {
    if domain.len() > 253 {
        return Err("domain name too long".to_string());
    }
    for label in domain.split('.') {
        if label.is_empty() {
            return Err("domain contains an empty label".to_string());
        }
        if label.len() > 63 {
            return Err("domain label too long".to_string());
        }
        let valid_label = label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid_label {
            return Err("domain label contains invalid characters".to_string());
        }
    }
    Ok(())
}

/// Normalize and validate a host supplied to read-only operator endpoints.
///
/// Unlike create, availability checks may accept non-canonical but normalizable
/// authority values (uppercase host, trailing dot, default port) so kgoose can
/// ask the relay for the canonical spelling before creating. Schemes, paths,
/// userinfo, whitespace/control characters, and oversized values are still
/// rejected.
pub(crate) fn normalize_candidate_host(host: &str) -> Result<String, String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("host contains invalid characters".to_string());
    }
    if host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    let normalized = normalize_host(host);
    validate_host(&normalized)?;
    Ok(normalized)
}

/// Deliver the membership list that owner persistence committed to live
/// subscribers. The list is already stored, so this is best-effort.
async fn fan_out_membership_snapshot(
    state: &Arc<AppState>,
    community: buzz_core::CommunityId,
    host: &str,
    snapshot: Option<buzz_core::StoredEvent>,
) {
    let tenant = TenantContext::resolved(community, host);
    crate::handlers::side_effects::fan_out_nip43_membership_list(&tenant, state, snapshot).await;
}

/// After a committed creation that bound issuers, drop this pod's cached
/// "not found" for the host so the creator's first login looks it up.
pub(crate) fn record_binding_creation(
    resolver: Option<&crate::nip_fi_resolver::DatabaseResolver>,
    host: &str,
    issuers: &[String],
) {
    if let (Some(resolver), false) = (resolver, issuers.is_empty()) {
        resolver.record_creation(host);
    }
}

/// Validate and execute a relay-operator community provisioning request.
///
/// The caller is an HTTP operator endpoint, not the Nostr event ingest path.
/// That keeps the tenant data-plane fences unchanged: no relay-membership
/// bypass, no special event kind, no command routed ahead of moderation/write
/// blocks. The endpoint authenticates its NIP-98 signer first, then passes the
/// signer here for the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist.
///
/// Idempotency and owner semantics: the request is idempotent on the host row
/// (re-sending it never duplicates a community). Initial owner bootstrap still
/// converges after a partial create, but rotating an existing owner requires an
/// active, non-archived community with no non-aborted deletion request.
pub async fn provision_community(
    state: &Arc<AppState>,
    operator_pubkey: &nostr::PublicKey,
    request: ProvisionCommunityRequest,
) -> Result<ProvisionCommunityResponse, String> {
    let operator_hex = operator_pubkey.to_hex();

    // Operator gate. Deliberately NOT a relay_members lookup: provisioning
    // authority spans tenants and lives in deployment config only. Empty
    // allowlist → everyone is rejected (fail closed).
    if !state
        .config
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &operator_hex)
    {
        return Err("actor not authorized: not a relay operator".to_string());
    }

    validate_host(&request.host)?;
    let nip_fi_policy = state.config.nip_fi_provision_binding.as_ref();
    // Unbound either way: `issuers_for_new_host` binds nothing on a reserved host.
    let nip_fi_issuers =
        nip_fi_policy.map_or(&[][..], |policy| policy.issuers_for_new_host(&request.host));

    let initial_owner = request
        .initial_owner_pubkey
        .as_deref()
        .map(|value| {
            validate_pubkey_hex(value).ok_or_else(|| {
                "invalid initial_owner_pubkey: expected 64-char hex pubkey".to_string()
            })
        })
        .transpose()?;

    if request.create_only {
        if nip_fi_policy.is_some_and(|policy| policy.is_reserved(&request.host)) {
            return Err(buzz_auth::BindRefusal::ReservedHost.to_string());
        }
        let owner_hex = initial_owner.as_deref().ok_or_else(|| {
            "initial_owner_pubkey is required when create_only is true".to_string()
        })?;
        let write = state
            .db
            .create_community_with_owner_and_nip_fi_issuers(
                &request.host,
                owner_hex,
                nip_fi_issuers,
                &state.relay_keypair,
            )
            .await
            .map_err(|e| format!("failed to create community: {e}"))?;
        let record = match write.outcome {
            buzz_db::CreateCommunityWithOwnerResult::Created(record) => record,
            buzz_db::CreateCommunityWithOwnerResult::HostExists => {
                return Err("community already exists".to_string());
            }
            buzz_db::CreateCommunityWithOwnerResult::LimitReached => {
                return Err("limit_reached: owner has reached the community limit".to_string());
            }
        };

        info!(
            operator = %operator_hex,
            community = %record.id,
            host = %record.host,
            owner = %owner_hex,
            "community created via operator endpoint"
        );
        record_binding_creation(
            state.nip_fi_resolver.as_deref(),
            &record.host,
            nip_fi_issuers,
        );
        fan_out_membership_snapshot(state, record.id, &record.host, write.snapshot).await;
        return Ok(ProvisionCommunityResponse {
            community_id: record.id.to_string(),
            host: record.host,
            status: "created",
            owner_pubkey: initial_owner,
        });
    }

    // Legacy convergence mode remains available to deployment operators and
    // startup tooling. Clients provisioning on behalf of end users must use
    // create_only so an existing owner can never be rotated by a create race.
    let record = state
        .db
        .ensure_configured_community_with_nip_fi_issuers(&request.host, nip_fi_issuers)
        .await
        .map_err(|e| format!("failed to create community: {e}"))?;
    if record.created {
        record_binding_creation(
            state.nip_fi_resolver.as_deref(),
            &record.host,
            nip_fi_issuers,
        );
    }

    if let Some(owner_hex) = &initial_owner {
        let write = state
            .db
            .provision_owner(record.id, owner_hex, &state.relay_keypair)
            .await
            .map_err(|e| format!("community provisioned but owner bootstrap failed: {e}"))?;
        match write.outcome {
            buzz_db::relay_members::ProvisionOwnerResult::Applied => {}
            buzz_db::relay_members::ProvisionOwnerResult::LifecycleConflict => {
                return Err(
                    "owner_conflict: community must be active to rotate ownership".to_string(),
                );
            }
            buzz_db::relay_members::ProvisionOwnerResult::DeletionPending => {
                return Err("owner_conflict: community deletion is pending".to_string());
            }
        }
        fan_out_membership_snapshot(state, record.id, &record.host, write.snapshot).await;
    }

    info!(
        operator = %operator_hex,
        community = %record.id,
        host = %record.host,
        owner = initial_owner.as_deref().unwrap_or("<none>"),
        created = record.created,
        "community provisioned via operator endpoint"
    );

    Ok(ProvisionCommunityResponse {
        community_id: record.id.to_string(),
        host: record.host,
        status: if record.created { "created" } else { "existed" },
        owner_pubkey: initial_owner,
    })
}

/// Startup: ensure the deployment community at `host`, bound only as the
/// provisioning policy allows. `Ok(None)` when `host` is empty.
pub async fn ensure_deployment_community(
    db: &buzz_db::Db,
    policy: Option<&buzz_auth::ProvisionBindingPolicy>,
    host: &str,
) -> Result<Option<buzz_db::EnsuredCommunityRecord>, buzz_db::DbError> {
    if host.is_empty() {
        return Ok(None);
    }
    let issuers = policy.map_or(&[][..], |policy| policy.issuers_for_new_host(host));
    db.ensure_configured_community_for_bootstrap(host, issuers)
        .await
        .map(Some)
}

/// Why the reserved-host startup check refused.
#[derive(Debug, thiserror::Error)]
pub enum ReservedHostsError {
    /// A reserved host holds an issuer binding.
    #[error(
        "reserved hosts hold NIP-FI issuer bindings (a command audience must never be a \
         community audience): {0:?}; remove each with `buzz-admin nip-fi unbind --host <h> \
         --issuer <i>`, or revert the command audience"
    )]
    Bound(Vec<String>),
    /// The check could not read the database.
    #[error("failed to check reserved NIP-FI hosts: {0}")]
    Check(buzz_db::DbError),
}

/// Refuse startup when a reserved host (a NIP-FI command audience) holds an
/// issuer binding. An unbound reserved community row is valid.
pub async fn verify_reserved_hosts_unbound(
    db: &buzz_db::Db,
    policy: &buzz_auth::ProvisionBindingPolicy,
) -> Result<(), ReservedHostsError> {
    let bound = db
        .nip_fi_bound_hosts(&policy.reserved_hosts())
        .await
        .map_err(ReservedHostsError::Check)?;
    if bound.is_empty() {
        Ok(())
    } else {
        Err(ReservedHostsError::Bound(bound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_verifies_reserved_hosts_before_seeding_the_deployment_community() {
        let main = include_str!("../main.rs");
        let verify = main
            .find("verify_reserved_hosts_unbound(&db, policy).await?;")
            .expect("startup must refuse a bound reserved host");
        let seed = main
            .find("match ensure_deployment_community(&db,")
            .expect("startup seeds the deployment community");
        assert!(verify < seed, "the reserved-host check must run first");
    }

    #[test]
    fn host_valid_bare_domain() {
        assert!(validate_host("acme.communities.buzz.xyz").is_ok());
    }

    #[test]
    fn host_valid_with_port() {
        assert!(validate_host("localhost:3000").is_ok());
    }

    #[test]
    fn host_rejects_empty() {
        assert!(validate_host("").is_err());
    }

    #[test]
    fn host_rejects_uppercase() {
        assert!(validate_host("Acme.example").is_err());
    }

    #[test]
    fn host_rejects_default_port() {
        assert!(validate_host("acme.example:443").is_err());
        assert!(validate_host("acme.example:80").is_err());
    }

    #[test]
    fn host_rejects_trailing_dot() {
        assert!(validate_host("acme.example.").is_err());
    }

    #[test]
    fn host_rejects_scheme_path_userinfo() {
        assert!(validate_host("wss://acme.example").is_err());
        assert!(validate_host("acme.example/path").is_err());
        assert!(validate_host("user@acme.example").is_err());
        assert!(validate_host("acme.example?x=1").is_err());
        assert!(validate_host("acme.example#frag").is_err());
    }

    #[test]
    fn host_rejects_invalid_authorities() {
        assert!(validate_host(":").is_err());
        assert!(validate_host("example..com").is_err());
        assert!(validate_host("foo_bar.example").is_err());
        assert!(validate_host("-bad.example").is_err());
        assert!(validate_host("bad-.example").is_err());
        assert!(validate_host("example.com:99999").is_err());
        assert!(validate_host("[::1").is_err());
        assert!(validate_host("[not-ipv6]").is_err());
        assert!(validate_host(&format!("{}.example", "a".repeat(64))).is_err());
    }

    #[test]
    fn host_rejects_whitespace_and_control() {
        assert!(validate_host("acme .example").is_err());
        assert!(validate_host("acme\n.example").is_err());
    }

    #[test]
    fn host_rejects_oversized() {
        let long = format!("{}.example", "a".repeat(260));
        assert!(validate_host(&long).is_err());
    }

    #[test]
    fn host_accepts_ipv6_bracket_literal() {
        assert!(validate_host("[::1]:3000").is_ok());
    }

    #[test]
    fn candidate_host_normalizes_safe_variants() {
        assert_eq!(
            normalize_candidate_host("Acme.Example:443").unwrap(),
            "acme.example"
        );
        assert_eq!(
            normalize_candidate_host("acme.example.").unwrap(),
            "acme.example"
        );
    }

    #[test]
    fn candidate_host_rejects_non_authorities() {
        assert!(normalize_candidate_host("https://acme.example").is_err());
        assert!(normalize_candidate_host("acme.example/path").is_err());
        assert!(normalize_candidate_host("acme .example").is_err());
    }
}
