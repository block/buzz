//! Opt-in IFC integration for the harness's recent DM history read.
//! Relay I/O stays in RestClient; policy and retained flow state stay in buzz-ifc.

use std::collections::{BTreeSet, HashMap};

use anyhow::{bail, ensure, Context, Result};
use buzz_core::kind::{
    KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use buzz_ifc::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, CommunityId, ConversationKind,
    DomainFacts, ExecutionDomain, IfcSession, OperationEffect, Principal, ResourceLabel,
};
use nostr::{Event, PublicKey};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{queue::FlushBatch, relay::RestClient, scope::SessionScope};

const READ_HISTORY: &str = "channel.read";

/// Trusted harness configuration, read once at startup. No agent API is added.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadConfig {
    pub(crate) channel_id: Uuid,
    community_id: Uuid,
    relay_pubkey: PublicKey,
}

impl ReadConfig {
    /// Load the optional hook from the trusted harness environment.
    pub(crate) fn from_env() -> Result<Option<Self>> {
        match std::env::var("BUZZ_ACP_IFC_READ") {
            Ok(value) => Ok(Some(
                serde_json::from_str(&value).context("invalid BUZZ_ACP_IFC_READ configuration")?,
            )),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(error).context("invalid BUZZ_ACP_IFC_READ configuration"),
        }
    }

    /// Check the retained domain and resource before returning history to the
    /// existing prompt formatter. A failed read never releases a partial page.
    pub(crate) async fn read_history(
        &self,
        rest: &RestClient,
        batch: &FlushBatch,
        owner: Option<PublicKey>,
        sessions: &mut HashMap<SessionScope, IfcSession>,
        limit: u32,
    ) -> Result<Value> {
        ensure!(
            batch.channel_id == self.channel_id && batch.scope.channel_id() == self.channel_id,
            "DM is outside IFC scope"
        );
        let mut requesters = BTreeSet::new();
        for input in batch.events.iter().chain(&batch.cancelled_events) {
            ensure!(
                input.event.verify().is_ok()
                    && exact_tag(&input.event, "h", &self.channel_id.to_string()),
                "unverified DM trigger"
            );
            requesters.insert(Principal::from_public_key(&input.event.pubkey)?);
        }
        let (domain, label) = self.snapshot(rest, owner, &requesters).await?;
        let session = sessions.entry(batch.scope.clone()).or_insert_with(|| {
            let mut session = IfcSession::enter(domain.clone());
            // Memory, instructions, triggers, and tools are not all mediated.
            session.mark_unknown_input();
            session
        });
        check_read(session, &domain, &label)?;

        let history = rest
            .query_raw(&[json!({
                "kinds": [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2],
                "#h": [self.channel_id.to_string()], "limit": limit
            })])
            .await
            .context("could not read DM history")?;
        let events: Vec<Event> =
            serde_json::from_value(history).context("invalid DM history events")?;
        ensure!(
            events.len() <= limit as usize,
            "DM history exceeds requested limit"
        );
        for event in &events {
            self.verify_message(event)?;
        }

        // Re-derive from current signed facts after reading; never cache membership.
        let (current_domain, current_label) = self.snapshot(rest, owner, &requesters).await?;
        check_read(session, &current_domain, &current_label)?;
        serde_json::to_value(events).context("could not format verified DM history")
    }

    async fn snapshot(
        &self,
        rest: &RestClient,
        owner: Option<PublicKey>,
        requesters: &BTreeSet<Principal>,
    ) -> Result<(ExecutionDomain, ResourceLabel)> {
        let response = rest
            .query_raw(&[json!({
                "kinds": [KIND_NIP29_GROUP_METADATA, KIND_NIP29_GROUP_MEMBERS],
                "authors": [self.relay_pubkey.to_hex()],
                "#d": [self.channel_id.to_string()], "limit": 2, "consistency": "strong"
            })])
            .await
            .context("could not fetch current DM policy")?;
        let events: Vec<Event> =
            serde_json::from_value(response).context("invalid DM policy events")?;
        let (mut metadata, mut membership) = (None, None);
        for event in &events {
            ensure!(
                event.pubkey == self.relay_pubkey
                    && event.verify().is_ok()
                    && exact_tag(event, "d", &self.channel_id.to_string()),
                "unverified DM policy event"
            );
            let slot = match u32::from(event.kind.as_u16()) {
                KIND_NIP29_GROUP_METADATA => &mut metadata,
                KIND_NIP29_GROUP_MEMBERS => &mut membership,
                _ => bail!("unexpected DM policy event"),
            };
            ensure!(slot.replace(event).is_none(), "duplicate DM policy event");
        }
        let metadata = metadata.context("missing DM metadata")?;
        ensure!(
            exact_tag(metadata, "t", "dm")
                && metadata
                    .tags
                    .iter()
                    .any(|tag| tag.as_slice() == ["private"])
                && !metadata.tags.iter().any(|tag| {
                    matches!(
                        tag.as_slice().first().map(String::as_str),
                        Some("public" | "archived")
                    )
                }),
            "channel is not an active private DM"
        );
        let membership = membership.context("missing DM membership")?;
        let members = membership
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().is_some_and(|name| name == "p"))
            .map(|tag| {
                Principal::from_hex(tag.as_slice().get(1).context("invalid DM member")?)
                    .map_err(anyhow::Error::from)
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let community = CommunityId::from_uuid(self.community_id);
        let label = ResourceLabel::from_conversation(
            community,
            self.channel_id,
            ConversationKind::DirectMessage,
            members.iter().copied(),
        )?;
        let capabilities =
            CapabilitySet::from_operations([(READ_HISTORY, OperationEffect::NonEgressing)]);
        let domain = derive_execution_domain(
            DomainFacts {
                community,
                channel_id: self.channel_id,
                kind: ConversationKind::DirectMessage,
                members,
                executing_agent: Principal::from_public_key(&rest.keys.public_key())?,
                requesters: requesters.clone(),
                owner: owner.as_ref().map(Principal::from_public_key).transpose()?,
                system_principal: Some(Principal::from_public_key(&self.relay_pubkey)?),
            },
            &CapabilityPolicy::new(capabilities.clone(), capabilities),
        )?;
        Ok((domain, label))
    }

    fn verify_message(&self, event: &Event) -> Result<()> {
        ensure!(
            matches!(
                u32::from(event.kind.as_u16()),
                KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2
            ) && event.verify().is_ok()
                && exact_tag(event, "h", &self.channel_id.to_string()),
            "unverified DM message"
        );
        Ok(())
    }
}

fn check_read(
    session: &IfcSession,
    current: &ExecutionDomain,
    label: &ResourceLabel,
) -> Result<()> {
    ensure!(
        session.domain() == current,
        "DM policy changed; a fresh ACP session is required"
    );
    session.call(READ_HISTORY)?;
    session.read(label)?;
    Ok(())
}

fn exact_tag(event: &Event, name: &str, value: &str) -> bool {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|key| key == name));
    tags.next()
        .is_some_and(|tag| tag.as_slice() == [name, value])
        && tags.next().is_none()
}

#[cfg(test)]
mod tests;
