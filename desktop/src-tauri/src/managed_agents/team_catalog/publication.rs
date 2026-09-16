//! Prepared positive catalog heads and the narrow refresh/retract work item.
use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil};

use super::{
    build_team_catalog_event, AgentDefinition, CatalogTombstone, TeamRecord, KIND_TEAM_CATALOG,
};
use crate::{
    active_user_signer::ActiveUserSigner,
    managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    },
};

pub(crate) struct PreparedCatalogHead {
    db_path: PathBuf,
    pub(crate) d_tag: String,
    pubkey: String,
    prior: Option<(String, i64, String)>,
    builder: EventBuilder,
}

fn token(head: Option<&RetainedEvent>) -> Option<(String, i64, String)> {
    head.map(|head| {
        (
            head.raw_event.clone(),
            head.created_at,
            head.content.clone(),
        )
    })
}

/// Prepare against the head used to make the share/refresh decision, not a
/// second read that could silently adopt a concurrent unshare.
pub(crate) fn prepare_catalog_head_from_builder(
    db_path: &Path,
    pubkey: &str,
    d_tag: &str,
    head: Option<&RetainedEvent>,
    builder: EventBuilder,
) -> PreparedCatalogHead {
    PreparedCatalogHead {
        db_path: db_path.to_owned(),
        d_tag: d_tag.to_owned(),
        pubkey: pubkey.to_owned(),
        prior: token(head),
        builder: builder.custom_created_at(monotonic_created_at(head.map(|head| head.created_at))),
    }
}

pub(crate) fn prepare_catalog_head(
    db_path: &Path,
    pubkey: &str,
    team: &TeamRecord,
    members: &[AgentDefinition],
    shared_override: Option<bool>,
) -> Result<(PreparedCatalogHead, TeamRecord), String> {
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, KIND_TEAM_CATALOG, pubkey, &team.id)?;
    let mut team = team.clone();
    team.shared = shared_override.unwrap_or_else(|| {
        head.as_ref()
            .and_then(|head| Event::from_json(&head.raw_event).ok())
            .is_some_and(|event| buzz_core_pkg::kind::event_is_shared(&event))
    });
    let builder = build_team_catalog_event(&team, members, team.shared)?;
    Ok((
        prepare_catalog_head_from_builder(db_path, pubkey, &team.id, head.as_ref(), builder),
        team,
    ))
}

impl PreparedCatalogHead {
    pub(crate) async fn sign(self, signer: &ActiveUserSigner) -> Result<SignedCatalogHead, String> {
        if signer.public_key().to_hex() != self.pubkey {
            return Err("catalog signer does not own captured scope".into());
        }
        let event = signer
            .sign_event(self.builder.clone())
            .await
            .map_err(|e| format!("failed to sign team catalog event: {e}"))?;
        Ok(SignedCatalogHead {
            prepared: self,
            event,
        })
    }
}

pub(crate) struct SignedCatalogHead {
    prepared: PreparedCatalogHead,
    event: Event,
}

impl SignedCatalogHead {
    /// Caller revalidates CatalogInputs under the store lock before this call.
    pub(crate) fn commit(self) -> Result<(Event, RetainedEvent), String> {
        let mut conn = open_retention_db(&self.prepared.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let head = get_retained_event(
            &tx,
            KIND_TEAM_CATALOG,
            &self.prepared.pubkey,
            &self.prepared.d_tag,
        )?;
        if token(head.as_ref()) != self.prepared.prior {
            return Err("catalog head changed during signing; retry reconciliation".into());
        }
        let retained = RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: self.prepared.pubkey,
            d_tag: self.prepared.d_tag,
            content: self.event.content.clone(),
            created_at: self.event.created_at.as_secs() as i64,
            raw_event: self.event.as_json(),
            pending_sync: true,
        };
        retain_event(&tx, &retained)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((self.event, retained))
    }
}

/// Exactly the two catalog reconciliation outcomes; neither mutates at prepare.
pub(crate) enum CatalogPlan {
    Refresh(PreparedCatalogHead),
    Retract(CatalogTombstone),
}

impl From<CatalogTombstone> for CatalogPlan {
    fn from(plan: CatalogTombstone) -> Self {
        Self::Retract(plan)
    }
}

impl CatalogPlan {
    pub(crate) fn d_tag(&self) -> &str {
        match self {
            Self::Refresh(plan) => &plan.d_tag,
            Self::Retract(plan) => &plan.d_tag,
        }
    }
    pub(crate) async fn sign(self, signer: &ActiveUserSigner) -> Result<SignedCatalogPlan, String> {
        match self {
            Self::Refresh(plan) => Ok(SignedCatalogPlan::Refresh(plan.sign(signer).await?)),
            Self::Retract(plan) => Ok(SignedCatalogPlan::Retract(plan.sign(signer).await?)),
        }
    }
}

pub(crate) enum SignedCatalogPlan {
    Refresh(SignedCatalogHead),
    Retract(super::tombstone::SignedCatalogTombstone),
}
impl SignedCatalogPlan {
    pub(crate) fn commit(self) -> Result<(), String> {
        match self {
            Self::Refresh(plan) => plan.commit().map(|_| ()),
            Self::Retract(plan) => plan.commit(),
        }
    }
}
