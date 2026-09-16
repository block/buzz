//! Owner definition publication: short prepare, unlocked signing, fenced retain.
use std::path::{Path, PathBuf};

use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
use nostr::{Event, EventBuilder, JsonUtil};

use crate::active_user_signer::ActiveUserSigner;
use crate::managed_agents::{
    agent_events::build_agent_event,
    persona_events::monotonic_created_at,
    retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    ManagedAgentRecord,
};

pub(crate) type AgentRetentionWork = Result<Option<PreparedAgentRecord>, String>;

pub(crate) struct PreparedAgentRecord {
    _mutation: crate::managed_agents::definition_mutation::DefinitionMutation,
    db_path: PathBuf,
    signer: ActiveUserSigner,
    agent: String,
    head: Option<(String, i64, String)>,
    builder: EventBuilder,
}

fn head_token(head: Option<RetainedEvent>) -> Option<(String, i64, String)> {
    head.map(|head| (head.raw_event, head.created_at, head.content))
}

/// Snapshot the serialized projection and monotonic timestamp under the caller's
/// store lock. Runtime-only changes remain true no-ops, including pending state.
pub(crate) fn prepare_agent_record(
    db_path: &Path,
    signer: &ActiveUserSigner,
    record: &ManagedAgentRecord,
) -> AgentRetentionWork {
    let owner = signer.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &record.pubkey)?;
    let builder = build_agent_event(record)?.custom_created_at(monotonic_created_at(
        head.as_ref().map(|head| head.created_at),
    ));
    if head
        .as_ref()
        .is_some_and(|head| head.content == builder.clone().build(signer.public_key()).content)
    {
        return Ok(None);
    }
    Ok(Some(PreparedAgentRecord {
        _mutation: crate::managed_agents::definition_mutation::DefinitionMutation::begin(
            db_path,
            &signer.public_key().to_hex(),
            KIND_MANAGED_AGENT,
            &record.pubkey,
        )?,
        db_path: db_path.to_owned(),
        signer: signer.clone(),
        agent: record.pubkey.clone(),
        head: head_token(head),
        builder,
    }))
}

pub(crate) struct SignedAgentRecord {
    prepared: PreparedAgentRecord,
    event: Event,
}

impl PreparedAgentRecord {
    pub(crate) async fn sign(self) -> Result<SignedAgentRecord, String> {
        let event = self
            .signer
            .sign_event(self.builder.clone())
            .await
            .map_err(|e| format!("failed to sign agent event: {e}"))?;
        Ok(SignedAgentRecord {
            prepared: self,
            event,
        })
    }
}

impl SignedAgentRecord {
    /// Caller holds the store lock across the disk read and this commit. Head
    /// validation and upsert share one transaction; an ACK-only change is safe.
    pub(crate) fn commit(self, records: &[ManagedAgentRecord]) -> Result<bool, String> {
        let current = records
            .iter()
            .find(|record| record.pubkey == self.prepared.agent)
            .ok_or_else(|| "agent removed during definition signing".to_string())?;
        if build_agent_event(current)?.build(self.event.pubkey).content != self.event.content {
            return Err(
                "agent changed during definition signing; boot reconcile will retry".into(),
            );
        }
        let mut conn = open_retention_db(&self.prepared.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let owner = self.event.pubkey.to_hex();
        let head = get_retained_event(&tx, KIND_MANAGED_AGENT, &owner, &self.prepared.agent)?;
        if head_token(head) != self.prepared.head {
            return Err(
                "agent head changed during definition signing; boot reconcile will retry".into(),
            );
        }
        retain_event(
            &tx,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: owner,
                d_tag: self.prepared.agent,
                content: self.event.content.clone(),
                created_at: self.event.created_at.as_secs() as i64,
                raw_event: self.event.as_json(),
                pending_sync: true,
            },
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// Complete one disk-authoritative publication. Failure/cancellation leaves the
/// saved disk definition intact for boot reconciliation, never a stale signed row.
pub(crate) async fn finish_agent_record(
    work: AgentRetentionWork,
    base_dir: &Path,
    store_lock: &std::sync::Mutex<()>,
) -> Result<bool, String> {
    let Some(prepared) = work? else {
        return Ok(false);
    };
    let signed = prepared.sign().await?;
    let _guard = store_lock.lock().map_err(|e| e.to_string())?;
    signed.commit(&super::read_agent_records(base_dir)?)
}
