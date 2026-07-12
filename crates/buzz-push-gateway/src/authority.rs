//! Durable installation authority and relay delegation state machine.
//!
//! App Attest authenticates the app instance and exact request transcript. It
//! does not prove an Apple-issued binding between that key and the APNs token;
//! accepting the directly submitted token is the protocol's explicit bootstrap
//! assumption.
use crate::model::AppProfile;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub id: Uuid,
    pub value: [u8; 32],
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInstallation {
    pub id: Uuid,
    pub app_attest_key_id: Vec<u8>,
    pub app_attest_public_key: Vec<u8>,
    pub assertion_counter: u32,
    pub profile: AppProfile,
    pub token_ciphertext: Vec<u8>,
    pub token_fingerprint: [u8; 32],
    pub endpoint_epoch: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub id: Uuid,
    pub app_attest_key_id: Vec<u8>,
    pub app_attest_public_key: Vec<u8>,
    pub assertion_counter: u32,
    pub profile: AppProfile,
    pub token_ciphertext: Vec<u8>,
    pub token_fingerprint: [u8; 32],
    pub endpoint_epoch: i64,
    pub expires_at: i64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    pub id: Uuid,
    pub installation_id: Uuid,
    pub relay_pubkey: String,
    pub endpoint_epoch: i64,
    pub generation: i64,
    pub not_before: i64,
    pub expires_at: i64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAuthority {
    pub delegation_id: Uuid,
    pub installation_id: Uuid,
    pub relay_pubkey: String,
    pub profile: AppProfile,
    pub token_ciphertext: Vec<u8>,
    pub endpoint_epoch: i64,
    pub generation: i64,
    pub expires_at: i64,
}

#[derive(Debug)]
pub struct DeliveryPermit {
    pub authority: DeliveryAuthority,
    pub relay_pubkey: String,
    pub request_id: Uuid,
}

impl DeliveryPermit {
    pub fn new(authority: DeliveryAuthority, relay_pubkey: String, request_id: Uuid) -> Self {
        Self {
            authority,
            relay_pubkey,
            request_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryDisposition {
    Terminal,
    Retryable,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    #[error("authority state rejected the request")]
    Rejected,
    #[error("authority store unavailable")]
    Unavailable,
}

/// Every mutating method is an atomic store operation. Implementations must
/// serialize installation assertion counters and delegation generations.
#[async_trait]
pub trait AuthorityStore: Send + Sync {
    /// Readiness must fail closed when durable authority cannot participate.
    async fn ready(&self) -> Result<(), AuthorityError>;
    async fn put_challenge(&self, challenge: Challenge) -> Result<(), AuthorityError>;
    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError>;
    async fn create_installation(
        &self,
        installation: NewInstallation,
    ) -> Result<(), AuthorityError>;
    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError>;
    async fn advance_assertion_counter(
        &self,
        installation_id: Uuid,
        previous: u32,
        next: u32,
    ) -> Result<(), AuthorityError>;
    async fn upsert_delegation(&self, delegation: Delegation) -> Result<(), AuthorityError>;
    async fn rotate_endpoint(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
        token_ciphertext: Vec<u8>,
        token_fingerprint: [u8; 32],
    ) -> Result<(), AuthorityError>;
    async fn revoke_delegation(
        &self,
        installation_id: Uuid,
        relay_pubkey: &str,
        new_generation: i64,
    ) -> Result<(), AuthorityError>;
    async fn revoke_installation(
        &self,
        installation_id: Uuid,
        expected_epoch: i64,
        new_epoch: i64,
    ) -> Result<(), AuthorityError>;
    /// Atomically validate and lock installation then delegation authority,
    /// reserve quota/replay state, and commit. That durable commit is the
    /// delivery send-begin linearization point seen by revocation.
    #[allow(clippy::too_many_arguments)]
    async fn authorize_delivery(
        &self,
        delegation_id: Uuid,
        relay_pubkey: &str,
        endpoint_epoch: i64,
        generation: i64,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError>;
    /// Retain terminal request ids; release retryable request ids while always
    /// retaining the one-use auth event admitted by `authorize_delivery`.
    async fn finish_delivery(
        &self,
        permit: DeliveryPermit,
        disposition: DeliveryDisposition,
    ) -> Result<(), AuthorityError>;
    /// Delete only data whose safety retention window has elapsed.
    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError>;
}

#[derive(Default)]
struct MemoryState {
    challenges: HashMap<Uuid, Challenge>,
    installations: HashMap<Uuid, Installation>,
    token_owners: HashMap<(AppProfile, [u8; 32]), Uuid>,
    delegations: HashMap<(Uuid, String), Delegation>,
    delegation_ids: HashMap<Uuid, (Uuid, String)>,
    delivery_auth_replays: HashMap<(String, String), i64>,
    delivery_request_replays: HashMap<(String, Uuid), i64>,
    endpoint_quotas: HashMap<[u8; 32], (i64, i64)>,
}

/// Executable reference store used by conformance tests. Production uses the
/// PostgreSQL implementation; this lock deliberately gives the model a single
/// linearization point for every authority transition.
#[derive(Default)]
pub struct MemoryAuthorityStore(Mutex<MemoryState>);

#[async_trait]
impl AuthorityStore for MemoryAuthorityStore {
    async fn ready(&self) -> Result<(), AuthorityError> {
        self.0
            .lock()
            .map(|_| ())
            .map_err(|_| AuthorityError::Unavailable)
    }

    async fn put_challenge(&self, challenge: Challenge) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        if s.challenges.insert(challenge.id, challenge).is_some() {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }

    async fn consume_challenge(
        &self,
        id: Uuid,
        value: [u8; 32],
        now: i64,
    ) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let challenge = s.challenges.remove(&id).ok_or(AuthorityError::Rejected)?;
        if challenge.value != value || challenge.expires_at < now {
            return Err(AuthorityError::Rejected);
        }
        Ok(())
    }

    async fn create_installation(&self, n: NewInstallation) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let token_key = (n.profile, n.token_fingerprint);
        if s.installations.contains_key(&n.id) || s.token_owners.contains_key(&token_key) {
            // Token possession alone never supersedes a live installation.
            return Err(AuthorityError::Rejected);
        }
        s.token_owners.insert(token_key, n.id);
        s.installations.insert(
            n.id,
            Installation {
                id: n.id,
                app_attest_key_id: n.app_attest_key_id,
                app_attest_public_key: n.app_attest_public_key,
                assertion_counter: n.assertion_counter,
                profile: n.profile,
                token_ciphertext: n.token_ciphertext,
                token_fingerprint: n.token_fingerprint,
                endpoint_epoch: n.endpoint_epoch,
                expires_at: n.expires_at,
                revoked: false,
            },
        );
        Ok(())
    }

    async fn installation(&self, id: Uuid, now: i64) -> Result<Installation, AuthorityError> {
        let s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let i = s
            .installations
            .get(&id)
            .filter(|i| !i.revoked && i.expires_at >= now)
            .ok_or(AuthorityError::Rejected)?;
        Ok(i.clone())
    }

    async fn advance_assertion_counter(
        &self,
        id: Uuid,
        previous: u32,
        next: u32,
    ) -> Result<(), AuthorityError> {
        if next <= previous {
            return Err(AuthorityError::Rejected);
        }
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let i = s
            .installations
            .get_mut(&id)
            .ok_or(AuthorityError::Rejected)?;
        if i.revoked || i.assertion_counter != previous {
            return Err(AuthorityError::Rejected);
        }
        i.assertion_counter = next;
        Ok(())
    }

    async fn upsert_delegation(&self, d: Delegation) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let i = s
            .installations
            .get(&d.installation_id)
            .ok_or(AuthorityError::Rejected)?;
        if i.revoked
            || i.endpoint_epoch != d.endpoint_epoch
            || d.generation < 1
            || d.not_before >= d.expires_at
            || d.expires_at > i.expires_at
        {
            return Err(AuthorityError::Rejected);
        }
        let key = (d.installation_id, d.relay_pubkey.clone());
        if s.delegations
            .get(&key)
            .is_some_and(|old| d.generation <= old.generation)
            || s.delegation_ids.contains_key(&d.id)
        {
            return Err(AuthorityError::Rejected);
        }
        s.delegation_ids.insert(d.id, key.clone());
        s.delegations.insert(key, d);
        Ok(())
    }

    async fn rotate_endpoint(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
        ciphertext: Vec<u8>,
        fingerprint: [u8; 32],
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let (profile, old_fingerprint) = {
            let i = s.installations.get(&id).ok_or(AuthorityError::Rejected)?;
            if i.revoked || i.endpoint_epoch != expected {
                return Err(AuthorityError::Rejected);
            }
            (i.profile, i.token_fingerprint)
        };
        let token_key = (profile, fingerprint);
        if s.token_owners
            .get(&token_key)
            .is_some_and(|owner| *owner != id)
        {
            return Err(AuthorityError::Rejected);
        }
        s.token_owners.remove(&(profile, old_fingerprint));
        s.token_owners.insert(token_key, id);
        let i = s
            .installations
            .get_mut(&id)
            .ok_or(AuthorityError::Rejected)?;
        i.endpoint_epoch = new;
        i.token_ciphertext = ciphertext;
        i.token_fingerprint = fingerprint;
        Ok(())
    }

    async fn revoke_delegation(
        &self,
        id: Uuid,
        relay: &str,
        generation: i64,
    ) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let key = (id, relay.to_owned());
        let old = s
            .delegations
            .get_mut(&key)
            .ok_or(AuthorityError::Rejected)?;
        if generation <= old.generation {
            return Err(AuthorityError::Rejected);
        }
        old.generation = generation;
        old.revoked = true;
        Ok(())
    }

    async fn revoke_installation(
        &self,
        id: Uuid,
        expected: i64,
        new: i64,
    ) -> Result<(), AuthorityError> {
        if new != expected.checked_add(1).ok_or(AuthorityError::Rejected)? {
            return Err(AuthorityError::Rejected);
        }
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let i = s
            .installations
            .get_mut(&id)
            .ok_or(AuthorityError::Rejected)?;
        if i.revoked || i.endpoint_epoch != expected {
            return Err(AuthorityError::Rejected);
        }
        i.endpoint_epoch = new;
        i.revoked = true;
        Ok(())
    }

    async fn authorize_delivery(
        &self,
        delegation_id: Uuid,
        relay: &str,
        epoch: i64,
        generation: i64,
        event_id: &str,
        request_id: Uuid,
        request_expires_at: i64,
        quota_window_seconds: i64,
        quota_max_deliveries: i64,
        now: i64,
    ) -> Result<DeliveryPermit, AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        let key = s
            .delegation_ids
            .get(&delegation_id)
            .cloned()
            .ok_or(AuthorityError::Rejected)?;
        let d = s.delegations.get(&key).ok_or(AuthorityError::Rejected)?;
        let i = s
            .installations
            .get(&d.installation_id)
            .ok_or(AuthorityError::Rejected)?;
        if d.revoked
            || i.revoked
            || d.id != delegation_id
            || d.relay_pubkey != relay
            || d.endpoint_epoch != epoch
            || d.generation != generation
            || i.endpoint_epoch != epoch
            || now < d.not_before
            || now > d.expires_at
            || now > i.expires_at
            || request_expires_at < now
            || request_expires_at > d.expires_at
        {
            return Err(AuthorityError::Rejected);
        }
        let authority = DeliveryAuthority {
            delegation_id,
            installation_id: i.id,
            relay_pubkey: relay.to_owned(),
            profile: i.profile,
            token_ciphertext: i.token_ciphertext.clone(),
            endpoint_epoch: epoch,
            generation,
            expires_at: d.expires_at,
        };
        let fingerprint = i.token_fingerprint;
        let auth_key = (relay.to_owned(), event_id.to_owned());
        let request_key = (relay.to_owned(), request_id);
        if s.delivery_auth_replays.contains_key(&auth_key)
            || s.delivery_request_replays.contains_key(&request_key)
        {
            return Err(AuthorityError::Rejected);
        }
        let quota = s.endpoint_quotas.entry(fingerprint).or_insert((now, 0));
        if now.saturating_sub(quota.0) >= quota_window_seconds {
            *quota = (now, 0);
        }
        if quota.1 >= quota_max_deliveries {
            return Err(AuthorityError::Rejected);
        }
        quota.1 += 1;
        s.delivery_auth_replays.insert(auth_key, request_expires_at);
        s.delivery_request_replays
            .insert(request_key, request_expires_at);
        Ok(DeliveryPermit::new(authority, relay.to_owned(), request_id))
    }

    async fn finish_delivery(
        &self,
        permit: DeliveryPermit,
        disposition: DeliveryDisposition,
    ) -> Result<(), AuthorityError> {
        if disposition == DeliveryDisposition::Retryable {
            self.0
                .lock()
                .map_err(|_| AuthorityError::Unavailable)?
                .delivery_request_replays
                .remove(&(permit.relay_pubkey, permit.request_id));
        }
        Ok(())
    }

    async fn reap_expired(&self, now: i64) -> Result<(), AuthorityError> {
        let mut s = self.0.lock().map_err(|_| AuthorityError::Unavailable)?;
        s.challenges
            .retain(|_, challenge| challenge.expires_at >= now);
        s.delivery_auth_replays
            .retain(|_, expires_at| *expires_at >= now);
        s.delivery_request_replays
            .retain(|_, expires_at| *expires_at >= now);
        s.endpoint_quotas
            .retain(|_, (started_at, _)| now.saturating_sub(*started_at) < 86_400);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn admitted(
        store: &MemoryAuthorityStore,
        event: &str,
        request: Uuid,
    ) -> Result<DeliveryPermit, AuthorityError> {
        store
            .authorize_delivery(
                Uuid::from_u128(2),
                &"11".repeat(32),
                1,
                1,
                event,
                request,
                1_100,
                60,
                10,
                1_000,
            )
            .await
    }

    async fn store() -> MemoryAuthorityStore {
        let store = MemoryAuthorityStore::default();
        store
            .create_installation(NewInstallation {
                id: Uuid::from_u128(1),
                app_attest_key_id: vec![1],
                app_attest_public_key: vec![2; 33],
                assertion_counter: 0,
                profile: AppProfile::BuzzIosProduction,
                token_ciphertext: vec![3],
                token_fingerprint: [4; 32],
                endpoint_epoch: 1,
                expires_at: 2_000,
            })
            .await
            .unwrap();
        store
            .upsert_delegation(Delegation {
                id: Uuid::from_u128(2),
                installation_id: Uuid::from_u128(1),
                relay_pubkey: "11".repeat(32),
                endpoint_epoch: 1,
                generation: 1,
                not_before: 900,
                expires_at: 1_500,
                revoked: false,
            })
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn retry_releases_request_id_but_burns_auth_event() {
        let store = store().await;
        let request = Uuid::new_v4();
        let first_event = "22".repeat(32);
        let permit = admitted(&store, &first_event, request).await.unwrap();
        store
            .finish_delivery(permit, DeliveryDisposition::Retryable)
            .await
            .unwrap();

        assert!(admitted(&store, &first_event, Uuid::new_v4())
            .await
            .is_err());
        admitted(&store, &"33".repeat(32), request)
            .await
            .expect("fresh auth event may retry stable request id");
    }

    #[tokio::test]
    async fn terminal_outcome_burns_request_id() {
        let store = store().await;
        let request = Uuid::new_v4();
        let permit = admitted(&store, &"22".repeat(32), request).await.unwrap();
        store
            .finish_delivery(permit, DeliveryDisposition::Terminal)
            .await
            .unwrap();
        assert!(admitted(&store, &"33".repeat(32), request).await.is_err());
    }
}
