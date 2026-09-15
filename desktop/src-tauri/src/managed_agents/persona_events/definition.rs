//! Positive owner-authored persona heads, with no guards across signing.
use super::{build_persona_event, monotonic_created_at, persona_d_tag};
use crate::{
    active_user_signer::ActiveUserSigner,
    managed_agents::{
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
        AgentDefinition,
    },
};
use buzz_core_pkg::kind::KIND_PERSONA;
use nostr::{Event, EventBuilder, JsonUtil};
use std::path::{Path, PathBuf};

pub(crate) struct PreparedPersonaHead {
    _mutation: crate::managed_agents::definition_mutation::DefinitionMutation,
    db_path: PathBuf,
    signer: ActiveUserSigner,
    head: Option<(String, i64, String)>,
    builder: EventBuilder,
    persona: AgentDefinition,
}

fn head_token(head: Option<RetainedEvent>) -> Option<(String, i64, String)> {
    head.map(|head| (head.raw_event, head.created_at, head.content))
}

/// Resolve share choice from this scope's exact head, not the global disk flag.
/// Boot reconciliation preserves its historical no-validation policy; explicit
/// saves/shares validate public text before invoking a potentially remote signer.
pub(crate) fn prepare_persona_head(
    db_path: &Path,
    signer: &ActiveUserSigner,
    persona: &AgentDefinition,
    shared_override: Option<bool>,
    validate_shared: bool,
) -> Result<PreparedPersonaHead, String> {
    let conn = open_retention_db(db_path)?;
    let head = get_retained_event(
        &conn,
        KIND_PERSONA,
        &signer.public_key().to_hex(),
        &persona_d_tag(persona),
    )?;
    let mut persona = persona.clone();
    persona.shared = shared_override.unwrap_or_else(|| {
        head.as_ref()
            .and_then(|row| Event::from_json(&row.raw_event).ok())
            .is_some_and(|event| buzz_core_pkg::kind::event_is_shared(&event))
    });
    if validate_shared && persona.shared {
        crate::managed_agents::validate_agent_definition_text(
            &persona.display_name,
            &persona.system_prompt,
        )?;
        crate::managed_agents::validate_agent_description_text(persona.description.as_deref())?;
    }
    let builder = build_persona_event(&persona)?.custom_created_at(monotonic_created_at(
        head.as_ref().map(|head| head.created_at),
    ));
    Ok(PreparedPersonaHead {
        _mutation: crate::managed_agents::definition_mutation::DefinitionMutation::begin(
            db_path,
            &signer.public_key().to_hex(),
            KIND_PERSONA,
            &persona_d_tag(&persona),
        )?,
        db_path: db_path.to_owned(),
        signer: signer.clone(),
        head: head_token(head),
        builder,
        persona,
    })
}

impl PreparedPersonaHead {
    pub(crate) fn unchanged(&self) -> bool {
        self.head.as_ref().is_some_and(|(_, _, content)| {
            *content == self.builder.clone().build(self.signer.public_key()).content
        })
    }

    pub(crate) async fn sign(self) -> Result<SignedPersonaHead, String> {
        let event = self
            .signer
            .sign_event(self.builder.clone())
            .await
            .map_err(|e| format!("failed to sign persona event: {e}"))?;
        Ok(SignedPersonaHead {
            prepared: self,
            event,
        })
    }
}

pub(crate) struct SignedPersonaHead {
    prepared: PreparedPersonaHead,
    event: Event,
}

impl SignedPersonaHead {
    /// Called under the store lock with freshly read definitions. Preserve the
    /// original durable disk save on failure; never overwrite a concurrent head.
    pub(crate) fn commit(
        self,
        personas: &[AgentDefinition],
    ) -> Result<(Event, RetainedEvent, AgentDefinition), String> {
        let mut current = personas
            .iter()
            .find(|p| p.id == self.prepared.persona.id)
            .ok_or_else(|| "persona removed during definition signing".to_string())?
            .clone();
        current.shared = self.prepared.persona.shared;
        let projection = build_persona_event(&current)?.build(self.event.pubkey);
        if projection.content != self.event.content
            || projection.tags != self.event.tags
            || current.is_builtin != self.prepared.persona.is_builtin
        {
            return Err(
                "persona changed during definition signing; boot reconcile will retry".into(),
            );
        }
        let mut conn = open_retention_db(&self.prepared.db_path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let pubkey = self.event.pubkey.to_hex();
        let d_tag = persona_d_tag(&self.prepared.persona);
        let head = get_retained_event(&tx, KIND_PERSONA, &pubkey, &d_tag)?;
        if head_token(head) != self.prepared.head {
            return Err(
                "persona head changed during definition signing; boot reconcile will retry".into(),
            );
        }
        let retained = RetainedEvent {
            kind: KIND_PERSONA,
            pubkey,
            d_tag,
            content: self.event.content.clone(),
            created_at: self.event.created_at.as_secs() as i64,
            raw_event: self.event.as_json(),
            pending_sync: true,
        };
        retain_event(&tx, &retained)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((self.event, retained, self.prepared.persona))
    }
}
