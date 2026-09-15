//! Owner-authored 30176 heads: capture, await signing, then revalidate.
use std::path::{Path, PathBuf};

use nostr::{Event, EventBuilder, JsonUtil};

use super::{build_team_event, TeamRecord, KIND_TEAM};
use crate::{
    active_user_signer::ActiveUserSigner,
    managed_agents::{
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    },
};

#[must_use = "sign outside the store lock and commit against fresh teams"]
pub(crate) struct PreparedTeamHead {
    _mutation: crate::managed_agents::definition_mutation::DefinitionMutation,
    db_path: PathBuf,
    signer: ActiveUserSigner,
    team: TeamRecord,
    head: Option<(String, i64, String)>,
    builder: EventBuilder,
}

fn head_token(head: Option<RetainedEvent>) -> Option<(String, i64, String)> {
    head.map(|head| (head.raw_event, head.created_at, head.content))
}

fn disk_token(team: &TeamRecord) -> Result<serde_json::Value, String> {
    let mut team = team.clone();
    // View projection, not disk authority.
    team.shared = false;
    serde_json::to_value(team).map_err(|e| e.to_string())
}

/// Capture immutable public inputs and the exact retained head, without signing.
pub(crate) fn prepare_team_head(
    db_path: &Path,
    signer: &ActiveUserSigner,
    team: &TeamRecord,
) -> Result<PreparedTeamHead, String> {
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(&conn, KIND_TEAM, &signer.public_key().to_hex(), &team.id)?;
    let builder = build_team_event(team)?.custom_created_at(monotonic_created_at(
        head.as_ref().map(|head| head.created_at),
    ));
    Ok(PreparedTeamHead {
        _mutation: crate::managed_agents::definition_mutation::DefinitionMutation::begin(
            db_path,
            &signer.public_key().to_hex(),
            KIND_TEAM,
            &team.id,
        )?,
        db_path: db_path.to_owned(),
        signer: signer.clone(),
        team: team.clone(),
        head: head_token(head),
        builder,
    })
}

impl PreparedTeamHead {
    pub(crate) fn unchanged(&self) -> bool {
        self.head.as_ref().is_some_and(|(_, _, content)| {
            *content == self.builder.clone().build(self.signer.public_key()).content
        })
    }

    pub(crate) async fn sign(self) -> Result<SignedTeamHead, String> {
        let event = self
            .signer
            .sign_event(self.builder.clone())
            .await
            .map_err(|e| format!("failed to sign team event: {e}"))?;
        Ok(SignedTeamHead {
            prepared: self,
            event,
        })
    }
}

pub(crate) struct SignedTeamHead {
    prepared: PreparedTeamHead,
    event: Event,
}

impl SignedTeamHead {
    /// Caller holds the store lock. Revalidate disk inputs before the atomic
    /// head comparison/write; pending_sync changes alone are not conflicts.
    pub(crate) fn commit(self, teams: &[TeamRecord]) -> Result<(), String> {
        let current = teams
            .iter()
            .find(|team| team.id == self.prepared.team.id)
            .ok_or_else(|| "team removed during definition signing".to_string())?;
        if disk_token(current)? != disk_token(&self.prepared.team)? {
            return Err("team changed during definition signing; boot reconcile will retry".into());
        }
        let mut conn = open_retention_db(&self.prepared.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let pubkey = self.prepared.signer.public_key().to_hex();
        let d_tag = self.prepared.team.id;
        if head_token(get_retained_event(&tx, KIND_TEAM, &pubkey, &d_tag)?) != self.prepared.head {
            return Err(
                "team head changed during definition signing; boot reconcile will retry".into(),
            );
        }
        retain_event(
            &tx,
            &RetainedEvent {
                kind: KIND_TEAM,
                pubkey,
                d_tag,
                content: self.event.content.clone(),
                created_at: self.event.created_at.as_secs() as i64,
                raw_event: self.event.as_json(),
                pending_sync: true,
            },
        )?;
        tx.commit().map_err(|e| e.to_string())
    }
}
