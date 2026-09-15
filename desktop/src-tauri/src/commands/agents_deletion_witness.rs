//! Agent deletion witnesses are prepared before disk removal. Signing failure is
//! an outcome, not deletion denial: commit retains the original best-effort policy.
use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil};

use crate::{
    active_user_signer::ActiveUserSigner,
    managed_agents::{
        agent_events::build_agent_delete,
        persona_events::monotonic_created_at,
        retention::{
            delete_retained_event, get_retained_event, open_retention_db, retain_event,
            tombstone_retention_d_tag, RetainedEvent,
        },
    },
};
use buzz_core_pkg::kind::{KIND_IA_ARCHIVE_REQUEST, KIND_MANAGED_AGENT};

/// Inert prepare result. No connection or guard survives preparation.
pub(crate) struct AgentTombstone {
    db_path: PathBuf,
    owner: String,
    agent: String,
    // Compare the full signed bytes (including raw event ID), presence, and
    // the two columns used to build the witnesses. Timestamps alone are not a fence.
    head: Option<(String, i64, String)>,
    tombstone: EventBuilder,
    archive_persona: Option<Option<String>>,
    signer: ActiveUserSigner,
    kind: u32,
}

fn head_token(head: Option<RetainedEvent>) -> Option<(String, i64, String)> {
    head.map(|head| (head.raw_event, head.created_at, head.content))
}

pub(crate) fn prepare_agent_tombstone(
    db_path: &Path,
    signer: &ActiveUserSigner,
    agent: &str,
) -> Result<AgentTombstone, String> {
    let owner = signer.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, agent)?;
    let persona_id = head
        .as_ref()
        .and_then(|head| super::pending::persona_id_from_head(&head.content));
    Ok(AgentTombstone {
        db_path: db_path.to_owned(),
        tombstone: build_agent_delete(agent, &owner)?.custom_created_at(monotonic_created_at(
            head.as_ref().map(|head| head.created_at),
        )),
        kind: KIND_MANAGED_AGENT,
        archive_persona: Some(persona_id),
        signer: signer.clone(),
        head: head_token(head),
        owner,
        agent: agent.to_owned(),
    })
}

/// The plan survives failed signing so its head can still be revalidated before
/// destructive commit. The pair is atomic in the base implementation: archive
/// signing is attempted only if tombstone signing succeeds.
pub(crate) struct AgentDeletionWitness {
    plan: AgentTombstone,
    outcome: Result<(Event, Option<Event>), String>,
}

impl AgentTombstone {
    pub(crate) async fn sign(self, signer: &ActiveUserSigner) -> AgentDeletionWitness {
        let outcome = async {
            self.signer.check_valid()?;
            if signer.public_key().to_hex() != self.owner
                || signer.generation() != self.signer.generation()
            {
                return Err("agent tombstone signer does not own captured scope".into());
            }
            let signer = &self.signer;
            // Authorize before signing or destructive commit, with no SQLite lock.
            let archive_builder = match &self.archive_persona {
                Some(persona) => Some(
                    super::pending::build_agent_archive_request(
                        &self.signer,
                        &self.agent,
                        persona.as_deref(),
                    )
                    .await?,
                ),
                None => None,
            };
            let tombstone = signer
                .sign_event(self.tombstone.clone())
                .await
                .map_err(|e| format!("failed to sign managed-agent tombstone: {e}"))?;
            let archive = match archive_builder {
                Some(builder) => Some(
                    signer
                        .sign_event(builder)
                        .await
                        .map_err(|e| format!("failed to sign archive request: {e}"))?,
                ),
                None => None,
            };
            Ok((tombstone, archive))
        }
        .await;
        AgentDeletionWitness {
            plan: self,
            outcome,
        }
    }
}

/// Prepare errors also remain best-effort outcomes. Call under the store lock,
/// capturing records/cascade inputs in that same critical section.
pub(crate) fn prepare_agent_deletion<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &crate::app_state::AppState,
    agent: &str,
) -> Result<(AgentTombstone, ActiveUserSigner), String> {
    let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
    let signer = scope.owner_signer();
    let plan = prepare_agent_tombstone(&scope.db_path, &signer, agent)?;
    Ok((plan, signer))
}

pub(crate) async fn sign_agent_deletion(
    work: Result<(AgentTombstone, ActiveUserSigner), String>,
) -> Result<AgentDeletionWitness, String> {
    match work {
        Ok((plan, signer)) => Ok(plan.sign(&signer).await),
        Err(error) => Err(error),
    }
}

/// Revalidate all available witness heads BEFORE the original destructive body.
/// The caller revalidates disk/cascade inputs under its store lock first. One
/// IMMEDIATE transaction per database fences head changes through disk mutation
/// and witness persistence; cascade witnesses retain independent best-effort
/// outcomes, with a savepoint preserving each tombstone+archive pair's atomicity.
/// No signing awaits occur here. A conflict is non-destructive, including after
/// a failed signature. Other retention errors keep the base log-and-delete policy.
pub(crate) fn commit_agent_deletions<T>(
    work: Vec<Result<AgentDeletionWitness, String>>,
    delete: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut databases: Vec<(PathBuf, rusqlite::Connection)> = Vec::new();
    let mut ready = Vec::new();
    let mut captured_signers = Vec::new();
    for witness in work {
        let witness = match witness {
            Ok(witness) => witness,
            Err(error) => {
                ready.push(Err(error));
                continue;
            }
        };
        witness.plan.signer.check_valid()?;
        captured_signers.push(witness.plan.signer.clone());
        if witness.plan.signer.generation().is_some() {
            witness.outcome.as_ref().map_err(Clone::clone)?;
        }
        let path = &witness.plan.db_path;
        let index = match databases.iter().position(|(existing, _)| existing == path) {
            Some(index) => index,
            None => {
                let opened = open_retention_db(path).and_then(|conn| {
                    conn.execute_batch("BEGIN IMMEDIATE")
                        .map_err(|e| e.to_string())?;
                    Ok(conn)
                });
                match opened {
                    Ok(conn) => {
                        databases.push((path.clone(), conn));
                        databases.len() - 1
                    }
                    Err(error) => {
                        ready.push(Err(error));
                        continue;
                    }
                }
            }
        };
        let plan = &witness.plan;
        match get_retained_event(&databases[index].1, plan.kind, &plan.owner, &plan.agent) {
            Ok(head) => {
                if head_token(head) != plan.head {
                    return Err(
                        "agent deletion conflict: retained head changed; retry deletion".into(),
                    );
                }
                ready.push(Ok((index, witness)));
            }
            Err(error) => ready.push(Err(error)),
        }
    }
    // SQLite busy waits can outlive the initial admission check. Fence again
    // immediately before the irreversible synchronous body (including plans
    // whose retention I/O failed under the existing best-effort policy).
    for signer in captured_signers {
        signer.check_valid()?;
    }
    let result = delete()?;
    for witness in ready {
        let outcome = witness.and_then(|(index, witness)| {
            let conn = &databases[index].1;
            conn.execute_batch("SAVEPOINT agent_deletion_witness")
                .map_err(|e| e.to_string())?;
            let outcome = persist(conn, witness);
            if outcome.is_err() {
                let _ = conn.execute_batch("ROLLBACK TO agent_deletion_witness");
            }
            conn.execute_batch("RELEASE agent_deletion_witness")
                .map_err(|e| e.to_string())?;
            outcome
        });
        if let Err(error) = outcome {
            eprintln!("buzz-desktop: agent-tombstone: {error}");
        }
    }
    for (_, conn) in databases {
        if let Err(error) = conn.execute_batch("COMMIT") {
            eprintln!("buzz-desktop: agent-tombstone: failed to commit: {error}");
        }
    }
    Ok(result)
}

fn persist(conn: &rusqlite::Connection, witness: AgentDeletionWitness) -> Result<(), String> {
    let plan = witness.plan;
    let (tombstone, archive) = witness.outcome?;
    delete_retained_event(conn, plan.kind, &plan.owner, &plan.agent)?;
    let mut events = vec![(
        5,
        tombstone_retention_d_tag(plan.kind, &plan.agent),
        tombstone,
    )];
    if let Some(archive) = archive {
        events.push((KIND_IA_ARCHIVE_REQUEST, plan.agent, archive));
    }
    for (kind, d_tag, event) in events {
        retain_event(
            conn,
            &RetainedEvent {
                kind,
                pubkey: plan.owner.clone(),
                d_tag,
                content: event.content.clone(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )?;
    }
    Ok(())
}

/// Prepare a persona/team/catalog coordinate deletion under the store lock.
/// The same transaction coordinator handles these and agent cascade witnesses,
/// avoiding nested SQLite writers on the same captured scope.
pub(crate) fn prepare_definition_tombstone(
    db_path: &Path,
    signer: &ActiveUserSigner,
    kind: u32,
    d_tag: &str,
) -> Result<AgentTombstone, String> {
    use crate::managed_agents::{
        persona_events::build_persona_delete, team_catalog::build_team_catalog_delete,
        team_events::build_team_delete,
    };
    use buzz_core_pkg::kind::{KIND_PERSONA, KIND_TEAM, KIND_TEAM_CATALOG};
    let owner = signer.public_key().to_hex();
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, kind, &owner, d_tag)?;
    let builder = match kind {
        KIND_PERSONA => build_persona_delete(d_tag, &owner)?,
        KIND_TEAM => build_team_delete(d_tag, &owner)?,
        KIND_TEAM_CATALOG => build_team_catalog_delete(d_tag, &owner)?,
        _ => return Err("unsupported definition tombstone kind".into()),
    };
    Ok(AgentTombstone {
        db_path: db_path.to_owned(),
        owner,
        agent: d_tag.to_owned(),
        kind,
        tombstone: builder
            .custom_created_at(monotonic_created_at(head.as_ref().map(|h| h.created_at))),
        archive_persona: None,
        signer: signer.clone(),
        head: head_token(head),
    })
}

impl AgentDeletionWitness {
    /// Strict standalone commit for boot recovery and test adapters. Direct
    /// deletions use the best-effort batch coordinator before disk mutation.
    pub(crate) fn commit(self) -> Result<(), String> {
        self.plan.signer.check_valid()?;
        let mut conn = open_retention_db(&self.plan.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        // SQLite acquisition can wait behind another writer while native auth
        // is canceled or replaced. Fence the captured session after that wait.
        self.plan.signer.check_valid()?;
        if head_token(get_retained_event(
            &tx,
            self.plan.kind,
            &self.plan.owner,
            &self.plan.agent,
        )?) != self.plan.head
        {
            return Err("definition deletion conflict: retained head changed".into());
        }
        persist(&tx, self)?;
        tx.commit().map_err(|e| e.to_string())
    }
}
