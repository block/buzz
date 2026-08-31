//! Host transport is not delegation: only the registered owner may submit it.
use buzz_core::{host, tenant::TenantContext};
use nostr::Event;

use super::ingest::{IngestAuth, IngestError};
use crate::state::AppState;

fn invalid(error: String) -> IngestError {
    IngestError::Rejected(format!("invalid: {error}"))
}

fn envelope(event: &Event, auth: &IngestAuth, now: u64) -> Result<host::Envelope, IngestError> {
    let env = host::validate(event).map_err(invalid)?;
    if env.owner != *auth.pubkey() || auth.channel_ids().is_some() {
        return Err(IngestError::AuthFailed(
            "restricted: host events require their owner's global connection".into(),
        ));
    }
    if event.created_at.as_secs() > now.saturating_add(30) {
        return Err(invalid("host timestamp is in the future".into()));
    }
    if env.valid_until.is_some_and(|until| until <= now) {
        return Err(invalid("expired host report".into()));
    }
    Ok(env)
}

fn binding(env: &host::Envelope, registration: &Event) -> Result<(), IngestError> {
    let reg = host::validate(registration).map_err(invalid)?;
    if reg.label != "registration"
        || reg.host != env.host
        || reg.owner != env.owner
        || env.registration.as_deref() != Some(registration.id.to_hex().as_str())
    {
        return Err(invalid("host registration binding mismatch".into()));
    }
    Ok(())
}

pub(super) async fn authorize(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    auth: &IngestAuth,
) -> Result<(), IngestError> {
    let env = envelope(event, auth, nostr::Timestamp::now().as_secs())?;
    if let Some(id) = env.registration.as_deref() {
        let bytes = hex::decode(id).map_err(|e| invalid(e.to_string()))?;
        // A registration from another community or a deleted registration is
        // not authority. The owner check above runs before this existence lookup.
        let stored = state
            .db
            .get_event_by_id(tenant.community(), &bytes)
            .await
            .map_err(|e| IngestError::Internal(e.to_string()))?
            .ok_or_else(|| invalid("host registration not found".into()))?;
        binding(&env, &stored.event)?;
    }
    Ok(())
}

/// Owner-transported, host-signed pulse. Registration is NOT a host login grant.
pub(super) async fn authorize_presence(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    owner: nostr::PublicKey,
    channel_scoped: bool,
) -> Result<(), String> {
    let pulse = buzz_core::run_presence::parse_run(event, nostr::Timestamp::now().as_secs())?
        .ok_or("host presence requires a run")?;
    let id = pulse
        .registration
        .as_deref()
        .ok_or("host presence requires registration")?;
    if channel_scoped || pulse.location.is_some() {
        return Err("invalid host presence transport".into());
    }
    let stored = state
        .db
        .get_event_by_id(
            tenant.community(),
            &hex::decode(id).map_err(|_| "invalid registration")?,
        )
        .await
        .map_err(|_| "host registration lookup failed")?
        .ok_or("host registration not found")?;
    let binding = host::validate(&stored.event)?;
    if binding.label != "registration" || binding.owner != owner || binding.host != event.pubkey {
        return Err("host presence registration mismatch".into());
    }
    // The owner remains the authenticated principal. No host key admission,
    // general EVENT, REQ or HTTP privileges are created by this exception.
    Ok(())
}

pub(super) async fn authorize_execution(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    auth: &IngestAuth,
) -> Result<(), IngestError> {
    // Reject foreign/scoped callers before looking up private registration IDs.
    let owner = auth.pubkey().to_hex();
    if auth.channel_ids().is_some()
        || event
            .tags
            .iter()
            .filter(|t| t.as_slice() == ["p", owner.as_str()])
            .count()
            != 1
    {
        return Err(IngestError::AuthFailed(
            "restricted: execution requires owner transport".into(),
        ));
    }
    let ids: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().is_some_and(|s| s == "e"))
        .collect();
    let [id] = ids.as_slice() else {
        return Err(invalid("invalid execution registration".into()));
    };
    let tag = id.as_slice();
    if tag.len() != 2 || !buzz_core::host_execution::hex_id(&tag[1], 64) {
        return Err(invalid("invalid execution registration".into()));
    }
    let stored = state
        .db
        .get_event_by_id(
            tenant.community(),
            &hex::decode(&tag[1]).map_err(|_| invalid("invalid registration".into()))?,
        )
        .await
        .map_err(|_| IngestError::Internal("registration lookup failed".into()))?
        .ok_or_else(|| invalid("execution registration absent or revoked".into()))?;
    buzz_core::host_execution::validate_transport(event, &stored.event, *auth.pubkey())
        .map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_auth::Scope;
    use nostr::Keys;
    use uuid::Uuid;

    fn auth(keys: &Keys) -> IngestAuth {
        IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes: vec![Scope::UsersWrite],
            channel_ids: None,
            conn_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn transport_and_registration_binding_fail_closed() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let other = Keys::generate();
        let reg = host::registration(&owner, host.public_key(), 100).unwrap();
        let rep = host::report(
            &host,
            &reg,
            &host::Report {
                v: 1,
                name: "test".into(),
                os: "macos".into(),
                arch: "aarch64".into(),
                launcher_version: "test".into(),
                runtimes: vec![],
                accepts_start: false,
                provisioned: vec![],
            },
            101,
        )
        .unwrap();
        let owner_auth = auth(&owner);
        for event in [&reg, &rep] {
            assert!(envelope(event, &owner_auth, 101).is_ok());
            assert!(envelope(event, &auth(&other), 101).is_err());
            // Independent host connections are deliberately not enabled yet.
            assert!(envelope(event, &auth(&host), 101).is_err());
            let mut scoped = owner_auth.clone();
            if let IngestAuth::Nip42 { channel_ids, .. } = &mut scoped {
                *channel_ids = Some(vec![]);
            }
            assert!(envelope(event, &scoped, 101).is_err());
            assert!(envelope(event, &owner_auth, 0).is_err());
        }
        assert!(envelope(&rep, &owner_auth, 280).is_ok());
        assert!(envelope(&rep, &owner_auth, 281).is_err());
        let env = envelope(&rep, &owner_auth, 101).unwrap();
        assert!(binding(&env, &reg).is_ok());
        for wrong in [
            host::registration(&other, host.public_key(), 100).unwrap(),
            host::registration(&owner, other.public_key(), 100).unwrap(),
            host::registration(&owner, host.public_key(), 99).unwrap(),
            rep.clone(),
        ] {
            assert!(binding(&env, &wrong).is_err());
        }
        let mut tampered = rep;
        tampered.content.push('A');
        assert!(envelope(&tampered, &owner_auth, 101).is_err());
    }
}
