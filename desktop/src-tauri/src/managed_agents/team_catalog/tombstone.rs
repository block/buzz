//! PREPARE / SIGN / COMMIT for 30178 deletion. No signing capability is used
//! by either synchronous storage phase. Conflicts return without any writes;
//! the retained head remains the retry worklist for the next reconciliation.
use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil};

use super::{build_team_catalog_delete, AgentDefinition, TeamRecord, KIND_TEAM_CATALOG};
use crate::active_user_signer::ActiveUserSigner;
use crate::managed_agents::{
    persona_events::monotonic_created_at,
    retention::{
        get_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
        RetainedEvent,
    },
};

/// The exact disk decision inputs: optional team, and each referenced member
/// (including missing members). Unrelated personas are neither projected nor
/// used as a revision counter. Serialized records include all authored fields.
#[derive(Debug, PartialEq)]
pub(crate) struct CatalogInputs(serde_json::Value);

impl CatalogInputs {
    pub(crate) fn capture(
        id: &str,
        teams: &[TeamRecord],
        personas: &[AgentDefinition],
    ) -> Result<Self, String> {
        let mut team = teams.iter().find(|team| team.id == id).cloned();
        // This field is a retained-head view projection, not disk authority.
        if let Some(team) = &mut team {
            team.shared = false;
        }
        let team = team.as_ref();
        let members: Vec<_> = team
            .into_iter()
            .flat_map(|team| &team.persona_ids)
            .map(|id| (id, personas.iter().find(|persona| &persona.id == id)))
            .collect();
        serde_json::to_value((team, members))
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

/// An inert prepared deletion. Dropping it (including cancellation) changes
/// nothing. The head token is the raw signed event identity, never a timestamp.
#[derive(Debug)]
pub(crate) struct CatalogTombstone {
    db_path: PathBuf,
    pub(crate) d_tag: String,
    pubkey: String,
    prior_id: Option<nostr::EventId>,
    builder: EventBuilder,
}

fn head_id(head: Option<&RetainedEvent>) -> Result<Option<nostr::EventId>, String> {
    head.map(|head| {
        Event::from_json(&head.raw_event)
            .map(|event| event.id)
            .map_err(|e| format!("invalid retained catalog head: {e}"))
    })
    .transpose()
}

/// Short synchronous PREPARE. Caller holds its store lock while deciding and
/// capturing disk inputs, but must release that lock before `sign` is awaited.
pub(crate) fn prepare_team_catalog_tombstone(
    db_path: &Path,
    pubkey: &str,
    d_tag: &str,
) -> Result<CatalogTombstone, String> {
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, KIND_TEAM_CATALOG, pubkey, d_tag)?;
    prepare_team_catalog_tombstone_from_head(db_path, pubkey, d_tag, head.as_ref())
}

/// Prepare from the exact head already used for a refresh/orphan decision.
/// Re-reading here could silently adopt a concurrent unshare/replacement.
pub(crate) fn prepare_team_catalog_tombstone_from_head(
    db_path: &Path,
    pubkey: &str,
    d_tag: &str,
    head: Option<&RetainedEvent>,
) -> Result<CatalogTombstone, String> {
    Ok(CatalogTombstone {
        db_path: db_path.to_owned(),
        d_tag: d_tag.to_owned(),
        pubkey: pubkey.to_owned(),
        prior_id: head_id(head)?,
        builder: build_team_catalog_delete(d_tag, pubkey)?
            .custom_created_at(monotonic_created_at(head.map(|head| head.created_at))),
    })
}

impl CatalogTombstone {
    /// SIGN has no connection or store guard. The caller carries the captured
    /// owner capability, not Keys, and cannot retarget this coordinate.
    pub(crate) async fn sign(
        self,
        signer: &ActiveUserSigner,
    ) -> Result<SignedCatalogTombstone, String> {
        if signer.public_key().to_hex() != self.pubkey {
            return Err("catalog tombstone signer does not own captured scope".into());
        }
        let event = signer.sign_event(self.builder.clone()).await?;
        Ok(SignedCatalogTombstone { plan: self, event })
    }
}

/// Signed but not yet committed. Production callers reacquire the store lock
/// and re-read/compare CatalogInputs BEFORE calling commit under that lock.
pub(crate) struct SignedCatalogTombstone {
    plan: CatalogTombstone,
    event: Event,
}

impl SignedCatalogTombstone {
    /// Short COMMIT: revalidate presence AND exact raw head ID, then atomically
    /// purge/enqueue. Any failure rolls back both writes. No await occurs here.
    pub(crate) fn commit(self) -> Result<(), String> {
        let Self { plan, event } = self;
        let mut conn = open_retention_db(&plan.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| format!("failed to begin catalog tombstone transaction: {e}"))?;
        let head = get_retained_event(&tx, KIND_TEAM_CATALOG, &plan.pubkey, &plan.d_tag)?;
        if head_id(head.as_ref())? != plan.prior_id {
            return Err(
                "catalog tombstone conflict: retained head changed; retry reconciliation".into(),
            );
        }
        conn_purge(&tx, &plan.pubkey, &plan.d_tag)?;
        retain_event(
            &tx,
            &RetainedEvent {
                kind: 5,
                pubkey: plan.pubkey,
                d_tag: tombstone_retention_d_tag(KIND_TEAM_CATALOG, &plan.d_tag),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )?;
        tx.commit()
            .map_err(|e| format!("failed to commit catalog tombstone: {e}"))
    }
}

fn conn_purge(conn: &rusqlite::Connection, pubkey: &str, d_tag: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM persona_events WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        rusqlite::params![KIND_TEAM_CATALOG, pubkey, d_tag],
    )
    .map(|_| ())
    .map_err(|e| format!("failed to purge retained 30178 head: {e}"))
}
