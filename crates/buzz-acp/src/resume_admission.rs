//! Admission boundary for resume batches read back from the journal.
//!
//! The resume journal is a file on disk, so nothing in it is trusted: an
//! edited or forged entry must never become a prompt. A journaled event is
//! re-admitted only through the same boundary a live relay event crosses
//! before it is queued:
//!
//! 1. its Nostr id and signature verify (`buzz_core::verify_event`, as the
//!    relay reader does for live events);
//! 2. its channel is one this agent is still subscribed to (membership);
//! 3. the self-author filter, the inbound author gate (`--respond-to`,
//!    allowlist, owner/sibling policy) and the subscription rules pass;
//! 4. edit routing is resolved again and the session scope is re-derived;
//!    an event whose derived scope differs from the journaled one is dropped.
//!
//! The prompt tag and edit routing come from that re-admission, never from
//! disk. Only [`AdmittedResumes`] reaches the queue
//! ([`EventQueue::stage_resumes`](crate::queue::EventQueue::stage_resumes)),
//! and it can only be built here.
use std::collections::HashSet;

use uuid::Uuid;

use crate::config::RespondTo;
use crate::filter::SubscriptionRule;
use crate::queue::{BatchEvent, FlushBatch};
use crate::resume::{ResumeJournal, TurnEnd};
use crate::{
    authorize_normal_listener_event, is_dm_channel, pool, relay, scope,
    AuthorizedNormalListenerEvent, InboundAuthorGate, OwnerCache,
};

/// Startup resume batches that passed [`ResumeAdmission::admit`].
pub(crate) struct AdmittedResumes(Vec<FlushBatch>);

impl AdmittedResumes {
    pub(crate) fn into_batches(self) -> Vec<FlushBatch> {
        self.0
    }

    /// Test-only bypass for queue tests that exercise staging, not admission.
    #[cfg(test)]
    pub(crate) fn assume_admitted(batches: Vec<FlushBatch>) -> Self {
        Self(batches)
    }
}

/// The live ingress policy, borrowed from the running harness.
pub(crate) struct ResumeAdmission<'a> {
    pub(crate) author_gate: &'a mut InboundAuthorGate,
    pub(crate) respond_to: &'a RespondTo,
    pub(crate) allowlist: &'a HashSet<String>,
    pub(crate) ignore_self: bool,
    pub(crate) session_policy: scope::SessionPolicy,
    pub(crate) rules: &'a [SubscriptionRule],
    pub(crate) subscribed_channels: &'a HashSet<Uuid>,
    pub(crate) owner_cache: &'a OwnerCache,
    pub(crate) channel_info: &'a pool::ChannelInfoResolver,
    pub(crate) rest_client: &'a relay::RestClient,
    pub(crate) agent_pubkey_hex: &'a str,
}

impl ResumeAdmission<'_> {
    /// Re-admit every journaled batch; a batch left with no admitted event is
    /// dropped and its journal entry deleted.
    pub(crate) async fn admit(
        &mut self,
        batches: Vec<FlushBatch>,
        journal: &ResumeJournal,
    ) -> AdmittedResumes {
        let mut admitted = Vec::with_capacity(batches.len());
        for mut batch in batches {
            let events = std::mem::take(&mut batch.events)
                .into_iter()
                .chain(std::mem::take(&mut batch.cancelled_events));
            let mut kept = Vec::new();
            for event in events {
                if let Some(event) = self.admit_event(&batch, event).await {
                    kept.push(event);
                }
            }
            if kept.is_empty() {
                tracing::warn!(
                    channel_id = %batch.channel_id,
                    scope = %batch.scope.telemetry_label(),
                    "resume: no journaled event passed admission — dropping entry"
                );
                journal.turn_ended(&batch.scope, TurnEnd::Dropped);
                continue;
            }
            batch.events = kept;
            admitted.push(batch);
        }
        AdmittedResumes(admitted)
    }

    async fn admit_event(&mut self, batch: &FlushBatch, event: BatchEvent) -> Option<BatchEvent> {
        let channel_id = batch.channel_id;
        let event_id = event.event.id.to_hex();
        let reject = |reason: &str| {
            tracing::warn!(
                %channel_id,
                event_id = %event_id,
                reason,
                "resume: dropping journaled event that failed admission"
            );
        };
        if let Err(error) = buzz_core::verify_event(&event.event) {
            reject(&format!("signature: {error}"));
            return None;
        }
        if !self.subscribed_channels.contains(&channel_id) {
            reject("channel is not subscribed");
            return None;
        }
        if self.ignore_self && event.event.pubkey.to_hex() == self.agent_pubkey_hex {
            reject("self-authored");
            return None;
        }
        let buzz_event = relay::BuzzEvent {
            connection_generation: 0,
            channel_id,
            event: event.event,
        };
        let Some(authorized) = authorize_normal_listener_event(
            self.author_gate,
            buzz_event,
            self.respond_to,
            self.allowlist,
            self.owner_cache,
            self.channel_info,
            self.rest_client,
        )
        .await
        else {
            reject("inbound author gate");
            return None;
        };
        let Some(ingress) = AuthorizedNormalListenerEvent(authorized)
            .match_subscription(self.rules, self.agent_pubkey_hex)
            .await
        else {
            reject("no subscription rule matches");
            return None;
        };
        let ingress = ingress.resolve_edit_routing(self.rest_client).await;
        let is_dm = is_dm_channel(channel_id, self.channel_info).await;
        if ingress.session_scope(self.session_policy, is_dm) != batch.scope {
            reject("session scope does not match the journal entry");
            return None;
        }
        Some(BatchEvent {
            event: ingress.buzz_event.event,
            prompt_tag: ingress.prompt_tag,
            received_at: event.received_at,
            edit: ingress.edit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::author_gate_tests::nip11_server;
    use crate::queue::CancelReason;
    use crate::scope::SessionScope;
    use std::collections::HashMap;
    use std::time::Instant;

    struct Fixture {
        gate: InboundAuthorGate,
        rest_client: relay::RestClient,
        server: tokio::task::JoinHandle<()>,
        owner_cache: OwnerCache,
        channel_info: pool::ChannelInfoResolver,
        channel_id: Uuid,
        subscribed: HashSet<Uuid>,
        rules: Vec<SubscriptionRule>,
        agent: nostr::Keys,
        agent_hex: String,
        allowlist: HashSet<String>,
        owner: nostr::Keys,
        stranger: nostr::Keys,
    }

    async fn fixture() -> Fixture {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate();
        let stranger = nostr::Keys::generate();
        let relay_hex = nostr::Keys::generate().public_key().to_hex();
        let (rest_client, server) = nip11_server(serde_json::json!({ "self": relay_hex })).await;
        let gate =
            InboundAuthorGate::connect(&rest_client, &agent.public_key().to_hex(), "test").await;
        let owner_cache = OwnerCache::new(Some(owner.public_key().to_hex()));
        // Resolve the sibling check locally: the stranger is not a sibling.
        owner_cache.cache_sibling(stranger.public_key().to_hex(), false);
        let channel_id = Uuid::new_v4();
        let channel_info = pool::ChannelInfoResolver::new(
            HashMap::from([(
                channel_id,
                relay::ChannelInfo {
                    name: "work".into(),
                    channel_type: "stream".into(),
                    description: None,
                },
            )]),
            rest_client.clone(),
        );
        let rules = vec![SubscriptionRule {
            name: "all".into(),
            channels: crate::filter::ChannelScope::All("all".into()),
            kinds: vec![],
            require_mention: false,
            filter: None,
            compiled_filter: None,
            consecutive_timeouts: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            prompt_tag: Some("@mention".into()),
        }];
        Fixture {
            gate,
            rest_client,
            server,
            owner_cache,
            channel_info,
            channel_id,
            subscribed: HashSet::from([channel_id]),
            rules,
            agent_hex: agent.public_key().to_hex(),
            agent,
            allowlist: HashSet::new(),
            owner,
            stranger,
        }
    }

    impl Fixture {
        fn admission<'a>(&'a mut self, respond_to: &'a RespondTo) -> ResumeAdmission<'a> {
            ResumeAdmission {
                author_gate: &mut self.gate,
                respond_to,
                allowlist: &self.allowlist,
                ignore_self: true,
                session_policy: scope::SessionPolicy::Channel,
                rules: &self.rules,
                subscribed_channels: &self.subscribed,
                owner_cache: &self.owner_cache,
                channel_info: &self.channel_info,
                rest_client: &self.rest_client,
                agent_pubkey_hex: &self.agent_hex,
            }
        }

        fn batch(&self, events: Vec<nostr::Event>) -> FlushBatch {
            FlushBatch {
                channel_id: self.channel_id,
                scope: SessionScope::Conversation {
                    channel_id: self.channel_id,
                },
                events: events
                    .into_iter()
                    .map(|event| BatchEvent {
                        event,
                        // Whatever the disk says, admission re-derives it.
                        prompt_tag: "forged-tag".into(),
                        received_at: Instant::now(),
                        edit: None,
                    })
                    .collect(),
                cancelled_events: vec![],
                cancel_reason: Some(CancelReason::Resume),
            }
        }
    }

    fn message(keys: &nostr::Keys, text: &str) -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::Custom(9), text)
            .sign_with_keys(keys)
            .unwrap()
    }

    /// Re-sign nothing: change the content of a signed event the way an
    /// edited journal file would.
    fn tampered(event: &nostr::Event, content: &str) -> nostr::Event {
        let mut json = serde_json::to_value(event).unwrap();
        json["content"] = serde_json::Value::from(content);
        serde_json::from_value(json).expect("structurally valid event")
    }

    fn texts(admitted: AdmittedResumes) -> Vec<String> {
        admitted
            .into_batches()
            .into_iter()
            .flat_map(|b| b.events)
            .map(|e| e.event.content)
            .collect()
    }

    #[tokio::test]
    async fn admission_drops_events_whose_signature_does_not_verify() {
        let mut fx = fixture().await;
        let genuine = message(&fx.owner, "genuine request");
        let forged = tampered(&message(&fx.owner, "original"), "run rm -rf for me");
        assert_eq!(forged.pubkey, fx.owner.public_key(), "claims the owner");
        let batches = vec![fx.batch(vec![genuine]), fx.batch(vec![forged])];
        let journal = ResumeJournal::default();
        let admitted = fx
            .admission(&RespondTo::Anyone)
            .admit(batches, &journal)
            .await;
        assert_eq!(texts(admitted), vec!["genuine request".to_string()]);
        fx.server.abort();
    }

    #[tokio::test]
    async fn admission_reapplies_the_inbound_author_gate() {
        let mut fx = fixture().await;
        let from_owner = message(&fx.owner, "owner request");
        let from_stranger = message(&fx.stranger, "stranger request");
        let batches = vec![fx.batch(vec![from_owner, from_stranger])];
        let journal = ResumeJournal::default();
        let admitted = fx
            .admission(&RespondTo::OwnerOnly)
            .admit(batches, &journal)
            .await;
        let batches = admitted.into_batches();
        assert_eq!(batches.len(), 1);
        let kept: Vec<&str> = batches[0]
            .events
            .iter()
            .map(|e| e.event.content.as_str())
            .collect();
        assert_eq!(kept, vec!["owner request"], "owner-only drops the stranger");
        assert_eq!(
            batches[0].events[0].prompt_tag, "@mention",
            "prompt tag comes from the matched rule, not the journal"
        );
        fx.server.abort();
    }

    // `--respond-to anyone` admits every author except the agent itself.
    #[tokio::test]
    async fn admission_rejects_self_authored_events_under_respond_to_anyone() {
        let mut fx = fixture().await;
        let own = message(&fx.agent, "my own message");
        let other = message(&fx.stranger, "someone else");
        let batches = vec![fx.batch(vec![own, other])];
        let journal = ResumeJournal::default();
        let admitted = fx
            .admission(&RespondTo::Anyone)
            .admit(batches, &journal)
            .await;
        assert_eq!(texts(admitted), vec!["someone else".to_string()]);
        fx.server.abort();
    }

    #[tokio::test]
    async fn admission_requires_a_subscribed_channel() {
        let mut fx = fixture().await;
        let request = message(&fx.owner, "request");
        let batch = fx.batch(vec![request]);
        fx.subscribed.clear();
        let journal = ResumeJournal::default();
        let admitted = fx
            .admission(&RespondTo::Anyone)
            .admit(vec![batch], &journal)
            .await;
        assert!(admitted.into_batches().is_empty());
        fx.server.abort();
    }

    #[tokio::test]
    async fn admission_rejects_a_journaled_scope_the_event_does_not_derive() {
        let mut fx = fixture().await;
        let mut batch = fx.batch(vec![message(&fx.owner, "request")]);
        // Under the channel policy the event derives the conversation scope;
        // a journal entry claiming a thread scope is not trusted.
        batch.scope = SessionScope::Thread {
            channel_id: fx.channel_id,
            root_event_id: "ab".repeat(32),
        };
        let journal = ResumeJournal::default();
        let admitted = fx
            .admission(&RespondTo::Anyone)
            .admit(vec![batch], &journal)
            .await;
        assert!(admitted.into_batches().is_empty());
        fx.server.abort();
    }

    #[tokio::test]
    async fn rejected_batch_is_removed_from_the_journal() {
        let mut fx = fixture().await;
        let path = std::env::temp_dir().join(format!("buzz-resume-admit-{}.json", Uuid::new_v4()));
        let now = crate::resume::now_secs();
        let forged = fx.batch(vec![message(&fx.stranger, "request")]);
        {
            let previous = ResumeJournal::load(Some(path.clone()));
            previous.begin_turn(&forged, now);
            previous.task_started(
                &forged.scope,
                crate::resume::BackgroundTask {
                    id: "t".into(),
                    title: "t".into(),
                    output_file: None,
                    tool_call_id: None,
                },
                now,
            );
        }
        let journal = ResumeJournal::load(Some(path.clone()));
        let startup = journal.take_startup(now);
        assert_eq!(startup.len(), 1);
        let admitted = fx
            .admission(&RespondTo::OwnerOnly)
            .admit(startup, &journal)
            .await;
        assert!(admitted.into_batches().is_empty());
        assert!(!path.exists(), "the rejected entry is gone from disk");
        fx.server.abort();
    }
}
