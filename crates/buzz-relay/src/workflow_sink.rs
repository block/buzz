//! Relay-side implementation of [`ActionSink`] for workflow actions.
//!
//! Builds Nostr events, persists them, and delegates post-persist side effects
//! (WebSocket fan-out, Redis pub/sub, search indexing, audit logging) to the
//! existing [`dispatch_persistent_event`] helper.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};

use buzz_core::kind::{KIND_STREAM_MESSAGE, KIND_WORKFLOW_AGENT_WAKE, KIND_WORKFLOW_DEF};
use buzz_core::tenant::CommunityId;
use buzz_db::event::EventQuery;
use buzz_pubsub::EventTopic;
use buzz_workflow::action_sink::{ActionSink, ActionSinkError, DoorbellContext};
use buzz_workflow::executor::WorkflowCause;
use chrono::Utc;
use nostr::{EventBuilder, Kind, Tag};
use tracing::info;
use uuid::Uuid;

use crate::handlers::event::{dispatch_persistent_event, fan_out_event_to_local_subscribers};
use crate::state::AppState;

/// Resolves `@Name` mentions in workflow message text to the pubkeys of the
/// channel members they name, so the emitted kind:9 carries the `p` tags that
/// ACP agent-wake (`event_mentions_agent`) is gated on.
///
/// The client resolves mentions to `p` tags at compose time from an interactive
/// autocomplete pick; the workflow path has only free text, so this reverse-parse
/// *defines* the matching contract. It is deliberately conservative to avoid
/// waking the wrong agent:
///
/// - **Members only.** Candidates are the destination channel's members; global
///   users are never matched.
/// - **Exact display name.** No substring, prefix, or fuzzy matching. Names may
///   contain spaces/punctuation (`"Will Pfleger"`, `"Lep (Subagent)"`), so the
///   match is anchored on `@` and terminated by a non-name boundary rather than
///   whitespace.
/// - **Greedy-longest, non-overlapping.** Longer names are matched first and
///   consume their span, so `@Will Pfleger` binds *Pfleger* and a bare `@Will`
///   does not match the member `"Will Pfleger"`.
/// - **Ambiguous names wake no one.** If two or more members share the matched
///   display name, no `p` tag is emitted for it — arbitrary selection would
///   silently misroute and tagging all of them is a false-wake firehose.
///
/// Returns deduplicated pubkey hexes, in first-appearance order in `text`.
fn resolve_mention_pubkeys(text: &str, members: &[(String, String)]) -> Vec<String> {
    // Name → pubkey, folding case (client matches case-insensitively). A name
    // that maps to more than one distinct pubkey is ambiguous → wake no one.
    let mut by_name: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for (name, pubkey) in members {
        if name.trim().is_empty() {
            continue;
        }
        by_name
            .entry(name.to_lowercase())
            .and_modify(|slot| {
                if slot.as_deref() != Some(pubkey.as_str()) {
                    *slot = None; // ambiguous
                }
            })
            .or_insert_with(|| Some(pubkey.clone()));
    }

    // Match longest names first so a longer name consumes its span before a
    // shorter substring name can claim part of it.
    let mut names: Vec<&(String, String)> = members.iter().collect();
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.chars().count()));

    let chars: Vec<char> = text.chars().collect();
    let mut consumed = vec![false; chars.len()];

    // Case-insensitivity folds *both* sides through `char::to_lowercase`, which
    // can change length: `İ` (U+0130) lowercases to two code points (`i` +
    // U+0307 combining dot). Comparing a pre-lowercased copy of the whole text
    // against a lowercased name by index silently desyncs once any earlier char
    // expands. Instead, fold on the fly: walk the original `chars` at the
    // candidate `@`, folding each char, and match against the folded-name char
    // stream — tracking how many *original* chars were consumed so
    // boundary/`consumed` accounting stays in original coordinates. `None` = no
    // match; `Some(n)` = matched, consuming `n` original chars after the `@`.
    let match_name_len = |start: usize, folded_name: &[char]| -> Option<usize> {
        let mut ci = start;
        let mut ni = 0;
        while ni < folded_name.len() {
            let c = *chars.get(ci)?;
            for fc in c.to_lowercase() {
                if folded_name.get(ni) != Some(&fc) {
                    return None;
                }
                ni += 1;
            }
            ci += 1;
        }
        Some(ci - start)
    };

    // A mention is anchored on `@` at a left boundary (start / whitespace / `(`)
    // and the matched name must not be followed by a name-continuation char —
    // otherwise `@Will` would match inside `@Willow`. Combined with matching the
    // longest member name first, this is the whole rule: no punctuation allowlist
    // to get wrong, and it is unicode-safe (em-dash, emoji all terminate a name).
    let is_left_boundary = |i: usize| i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(';
    let extends_name = |c: char| c.is_alphanumeric() || c == '_';

    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut hits: Vec<(usize, String)> = Vec::new();

    for (name, _) in &names {
        let folded_name: Vec<char> = name.to_lowercase().chars().collect();
        if folded_name.is_empty() {
            continue;
        }
        let mut at = 0;
        while at < chars.len() {
            // Anchor on `@` at a left boundary and an unconsumed span; only then
            // attempt the fold-match. `name_len` is measured in *original* chars,
            // so `at + 1 + name_len` is the true position just past the name.
            let name_len = (chars[at] == '@' && is_left_boundary(at) && !consumed[at])
                .then(|| match_name_len(at + 1, &folded_name))
                .flatten()
                .filter(|&n| {
                    chars[at + 1 + n..]
                        .first()
                        .is_none_or(|&c| !extends_name(c))
                });
            if let Some(name_len) = name_len {
                let span = 1 + name_len;
                if let Some(Some(pubkey)) = by_name.get(&name.to_lowercase()) {
                    hits.push((at, pubkey.clone()));
                }
                for slot in consumed.iter_mut().skip(at).take(span) {
                    *slot = true;
                }
                at += span;
            } else {
                at += 1;
            }
        }
    }

    hits.sort_by_key(|(at, _)| *at);
    for (_, pubkey) in hits {
        if seen.insert(pubkey.clone()) {
            out.push(pubkey);
        }
    }
    out
}

/// Producer-side rollout gate for the durable delivery protocol.
fn ensure_workflow_agent_delivery_enabled(enabled: bool) -> Result<(), ActionSinkError> {
    if enabled {
        Ok(())
    } else {
        Err(ActionSinkError::InvalidInput(
            "workflow agent delivery is disabled until ACP harnesses support durable wakes".into(),
        ))
    }
}

/// Relay-side action sink — executes workflow side-effects directly.
///
/// Holds a **weak** reference to `AppState` to avoid an `Arc` reference cycle:
/// `AppState` → `WorkflowEngine` → `ActionSink` → `AppState`. Using `Weak`
/// breaks the cycle so all structs can be dropped on shutdown.
///
/// Post-persist side effects are delegated to [`dispatch_persistent_event`]
/// for consistency with the REST/WebSocket paths.
pub struct RelayActionSink {
    state: Weak<AppState>,
}

impl RelayActionSink {
    /// Create a new `RelayActionSink` from the shared application state.
    pub fn new(state: &Arc<AppState>) -> Self {
        Self {
            state: Arc::downgrade(state),
        }
    }
}

impl ActionSink for RelayActionSink {
    #[allow(clippy::too_many_arguments)]
    fn send_message(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        step_id: &str,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        doorbell: &DoorbellContext,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>> {
        let step_id = step_id.to_owned();
        let channel_id = channel_id.to_owned();
        let text = text.to_owned();
        let author_pubkey = author_pubkey.to_owned();
        let doorbell = doorbell.clone();
        let reply_to = reply_to.map(str::to_owned);

        Box::pin(async move {
            // 0. Upgrade weak reference — fails only during shutdown.
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| ActionSinkError::Database("relay is shutting down".into()))?;

            // This producer-side rollout fence is intentionally before all
            // workflow-message side effects. Legacy ACPs cannot recognize the
            // durable protocol marker and `respond-to=anyone` would execute a
            // visible relay-authored kind:9 without claiming its delivery row.
            // Operators enable the protocol only after upgrading ACP harnesses.
            ensure_workflow_agent_delivery_enabled(state.config.workflow_agent_delivery_enabled)?;

            // The run carries its owning community (`community_id`); the
            // relay-signed kind:9 message belongs to *that* community, never the
            // deployment default. Re-deriving the tenant from `config.relay_url`
            // would post a community-B workflow's output into the deployment/
            // default community under N>1. Read the community's host back to
            // form a complete TenantContext (host is for labelling only — the
            // community is already fixed and is never re-derived from it). Fail
            // closed if the community no longer maps to a host.
            let host = state
                .db
                .lookup_community_host(community_id)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?
                .ok_or_else(|| {
                    ActionSinkError::Database(format!(
                        "workflow run community {community_id} is not mapped to a host"
                    ))
                })?;
            let tenant = buzz_core::tenant::TenantContext::resolved(community_id, host);

            // 1. Validate content is not empty/whitespace-only
            if text.trim().is_empty() {
                return Err(ActionSinkError::EmptyContent);
            }

            // 2. Parse and validate channel — canonicalize UUID immediately
            let channel_uuid = Uuid::parse_str(&channel_id)
                .map_err(|e| ActionSinkError::InvalidInput(format!("invalid UUID: {e}")))?;
            let channel_id_canonical = channel_uuid.to_string();

            let channel = state
                .db
                .get_channel(tenant.community(), channel_uuid)
                .await
                .map_err(|e| match &e {
                    buzz_db::DbError::ChannelNotFound(_) | buzz_db::DbError::NotFound(_) => {
                        ActionSinkError::ChannelNotFound(channel_id_canonical.clone())
                    }
                    _ => ActionSinkError::Database(e.to_string()),
                })?;

            if channel.archived_at.is_some() {
                return Err(ActionSinkError::ChannelArchived(
                    channel_id_canonical.clone(),
                ));
            }

            let author_pubkey = nostr::PublicKey::from_hex(&author_pubkey).map_err(|e| {
                ActionSinkError::InvalidInput(format!("invalid author pubkey: {e}"))
            })?;
            let author_pubkey_bytes = author_pubkey.to_bytes().to_vec();
            let author_pubkey_hex = author_pubkey.to_hex();
            // The referenced kind:30620 event is the managed agent's signed
            // authority artifact. Resolve that immutable revision by exact ID;
            // a human-owner management event may be the current coordinate
            // replacement, but it must never become execution authority.
            let mut definition_query = EventQuery::for_community(tenant.community());
            definition_query.channel_id = Some(channel_uuid);
            definition_query.kinds = Some(vec![KIND_WORKFLOW_DEF as i32]);
            definition_query.ids = Some(vec![hex::decode(&doorbell.definition_event_id).map_err(
                |_| ActionSinkError::InvalidInput("invalid workflow definition event id".into()),
            )?]);
            let definition = state
                .db
                .query_events(&definition_query)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    ActionSinkError::Database(format!(
                        "owner-signed workflow definition {workflow_id} is unavailable"
                    ))
                })?;
            let definition_event_id = definition.event.id.to_hex();
            if !definition_event_id.eq_ignore_ascii_case(&doorbell.definition_event_id) {
                return Err(ActionSinkError::Database(
                    "workflow definition changed while the run was executing".into(),
                ));
            }
            // Mention routing comes only from the owner-signed template, never
            // from values rendered out of a trigger event or webhook payload.
            // This preserves explicit workflow targets without allowing source
            // text such as `{{trigger.text}}` to wake a different agent.
            let (signed_workflow, _) =
                buzz_workflow::WorkflowEngine::parse_yaml(&definition.event.content).map_err(
                    |e| ActionSinkError::InvalidInput(format!("invalid workflow definition: {e}")),
                )?;
            let routing_text = signed_workflow
                .steps
                .iter()
                .find(|step| step.id == step_id)
                .and_then(|step| match &step.action {
                    buzz_workflow::schema::ActionDef::SendMessage { text, .. } => {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    ActionSinkError::InvalidInput(format!(
                        "workflow step {step_id} is not a send_message action"
                    ))
                })?;

            let is_member = state
                .is_member_cached(tenant.community(), channel_uuid, &author_pubkey_bytes)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            if !is_member && channel.visibility != "open" {
                return Err(ActionSinkError::InvalidInput(
                    "workflow owner does not have access to destination channel".into(),
                ));
            }

            // 3. Build kind:9 Nostr event
            //    - Signed by relay keypair (event.pubkey = relay pubkey)
            //    - `workflow-owner` identifies the claimed principal, but ACP
            //      grants authority only from the referenced owner-signed definition
            //    - `workflow-definition` binds the exact kind:30620 event and step
            //    - the owner is always p-tagged; extra wake targets are resolved
            //      only from the owner-signed template, never rendered cause data
            //    - `h` tag scopes to the channel (NIP-29, canonical UUID)
            //    - `buzz:workflow` tag prevents recursive workflow triggering
            let cause_tag = match &doorbell.cause {
                WorkflowCause::Event(id) => ["workflow-cause", "event", id.as_str()],
                WorkflowCause::Schedule(slot) => ["workflow-cause", "schedule", slot.as_str()],
                WorkflowCause::Command(id) => ["workflow-cause", "command", id.as_str()],
                WorkflowCause::Webhook => ["workflow-cause", "webhook", ""],
            };
            let reply_ancestry = match reply_to.as_deref() {
                Some(parent_hex) => Some(
                    crate::handlers::ingest::resolve_relay_reply_thread_meta(
                        tenant.community(),
                        parent_hex,
                        channel_uuid,
                        &state,
                    )
                    .await
                    .map_err(ActionSinkError::InvalidInput)?,
                ),
                None => None,
            };

            let mut tags = vec![
                Tag::parse(["workflow-run", doorbell.run_id.to_string().as_str()])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow-run tag: {e}")))?,
                Tag::parse(["workflow-step", step_id.as_str()])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow-step tag: {e}")))?,
                Tag::parse(["workflow-owner", &author_pubkey_hex])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow-owner tag: {e}")))?,
                Tag::parse(["workflow-definition", &definition_event_id, &step_id]).map_err(
                    |e| ActionSinkError::EventBuild(format!("workflow-definition tag: {e}")),
                )?,
                Tag::parse(cause_tag)
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow-cause tag: {e}")))?,
                Tag::parse(["p", &author_pubkey_hex])
                    .map_err(|e| ActionSinkError::EventBuild(format!("p tag: {e}")))?,
                Tag::parse(["h", &channel_id_canonical])
                    .map_err(|e| ActionSinkError::EventBuild(format!("h tag: {e}")))?,
                Tag::parse(["buzz:workflow", "message-v1"])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow tag: {e}")))?,
            ];

            // Resolve only owner-signed `@Name` mentions to member pubkeys.
            // Dynamic trigger/webhook values are deliberately excluded from
            // routing, even though ACP may render them into the local prompt.
            let members = state
                .db
                .get_members(tenant.community(), channel_uuid)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            let member_pubkeys: Vec<Vec<u8>> = members.iter().map(|m| m.pubkey.clone()).collect();
            let users = state
                .db
                .get_users_bulk(tenant.community(), &member_pubkeys)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            let named_members: Vec<(String, String)> = users
                .into_iter()
                .filter_map(|u| {
                    let name = u.display_name?;
                    Some((name, nostr::PublicKey::from_slice(&u.pubkey).ok()?.to_hex()))
                })
                .collect();
            if let Some(ancestry) = &reply_ancestry {
                let root_hex = ancestry.root_hex();
                let parent_hex = ancestry.parent_hex();
                if root_hex == parent_hex {
                    tags.push(
                        Tag::parse(["e", &root_hex, "", "reply"]).map_err(|e| {
                            ActionSinkError::EventBuild(format!("reply e tag: {e}"))
                        })?,
                    );
                } else {
                    tags.push(
                        Tag::parse(["e", &root_hex, "", "root"])
                            .map_err(|e| ActionSinkError::EventBuild(format!("root e tag: {e}")))?,
                    );
                    tags.push(
                        Tag::parse(["e", &parent_hex, "", "reply"]).map_err(|e| {
                            ActionSinkError::EventBuild(format!("reply e tag: {e}"))
                        })?,
                    );
                }
            }

            let mut wake_targets = vec![author_pubkey_hex.clone()];
            for mentioned in resolve_mention_pubkeys(&routing_text, &named_members) {
                if mentioned == author_pubkey_hex {
                    continue;
                }
                tags.push(
                    Tag::parse(["p", &mentioned])
                        .map_err(|e| ActionSinkError::EventBuild(format!("mention p tag: {e}")))?,
                );
                wake_targets.push(mentioned);
            }

            // The owner is always the first durable target. Consult that
            // identity before signing or publishing so a retry returns the
            // canonical visible message instead of creating an orphan duplicate.
            let (delivery_identity_lock, existing_message_event_id) = state
                .db
                .lock_workflow_agent_delivery_identity(
                    tenant.community(),
                    doorbell.run_id,
                    &step_id,
                    &author_pubkey_bytes,
                )
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            let kind_u32 = KIND_STREAM_MESSAGE;
            let (event, event_id_bytes, event_created_at) = if let Some((message_id, created_at)) =
                existing_message_event_id
            {
                let stored = state
                    .db
                    .get_event_by_id(tenant.community(), &message_id)
                    .await
                    .map_err(|e| ActionSinkError::Database(e.to_string()))?
                    .ok_or_else(|| {
                        ActionSinkError::Database(
                            "workflow delivery references a missing visible event".into(),
                        )
                    })?;
                wake_targets = stored
                    .event
                    .tags
                    .iter()
                    .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("p"))
                    .filter_map(|tag| tag.as_slice().get(1).cloned())
                    .collect();
                (None, message_id, created_at)
            } else {
                let event = EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), text.clone())
                    .tags(tags)
                    .sign_with_keys(&state.relay_keypair)
                    .map_err(|e| ActionSinkError::EventBuild(format!("signing: {e}")))?;
                let created_at =
                    chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                        .unwrap_or_else(Utc::now);
                let id = event.id.as_bytes().to_vec();
                (Some(event), id, created_at)
            };
            let event_id_hex = nostr::EventId::from_slice(&event_id_bytes)
                .map(|id| id.to_hex())
                .map_err(|e| {
                    ActionSinkError::Database(format!(
                        "stored workflow delivery has invalid message id: {e}"
                    ))
                })?;
            info!(event_id = %event_id_hex, channel_id = %channel_id_canonical, author = %author_pubkey,
                "Workflow SendMessage: reconciling kind {kind_u32} event and targets");

            let run = state
                .db
                .get_workflow_run(tenant.community(), doorbell.run_id)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            if run.workflow_id != workflow_id {
                return Err(ActionSinkError::Database(
                    "workflow delivery run does not belong to workflow".into(),
                ));
            }
            let expires_at = Utc::now()
                + chrono::Duration::seconds(buzz_core::workflow_delivery::ROW_LIFETIME_SECONDS);
            let targets = wake_targets
                .into_iter()
                .map(|pubkey| {
                    let key = nostr::PublicKey::from_hex(&pubkey).map_err(|e| {
                        ActionSinkError::InvalidInput(format!("invalid wake target: {e}"))
                    })?;
                    Ok((
                        pubkey,
                        buzz_db::workflow::WorkflowAgentDeliveryTarget {
                            id: Uuid::new_v4(),
                            pubkey: key.to_bytes().to_vec(),
                        },
                    ))
                })
                .collect::<Result<Vec<_>, ActionSinkError>>()?;
            let thread_meta_owned = match (event.as_ref(), reply_ancestry) {
                (Some(_), Some(ancestry)) => Some(ancestry.into_thread_meta(
                    event_id_bytes.clone(),
                    event_created_at,
                    channel_uuid,
                )),
                _ => None,
            };
            let top_level_thread_meta =
                event
                    .as_ref()
                    .map(|_| buzz_db::event::ThreadMetadataParams {
                        event_id: &event_id_bytes,
                        event_created_at,
                        channel_id: channel_uuid,
                        parent_event_id: None,
                        parent_event_created_at: None,
                        root_event_id: None,
                        root_event_created_at: None,
                        depth: 0,
                        broadcast: false,
                    });
            let thread_meta = thread_meta_owned
                .as_ref()
                .map(|owned| owned.as_params())
                .or(top_level_thread_meta);
            let (stored_event, created_ids) = state
                .db
                .commit_workflow_agent_deliveries(
                    delivery_identity_lock,
                    tenant.community(),
                    event.as_ref(),
                    &event_id_bytes,
                    event_created_at,
                    thread_meta,
                    workflow_id,
                    doorbell.run_id,
                    &step_id,
                    definition.event.id.as_bytes(),
                    channel_uuid,
                    &targets
                        .iter()
                        .map(
                            |(_, target)| buzz_db::workflow::WorkflowAgentDeliveryTarget {
                                id: target.id,
                                pubkey: target.pubkey.clone(),
                            },
                        )
                        .collect::<Vec<_>>(),
                    &run.execution_trace,
                    run.trigger_context.as_ref(),
                    expires_at,
                )
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            let created_ids = created_ids
                .into_iter()
                .collect::<std::collections::HashSet<_>>();
            for (target, delivery) in targets {
                let delivery_id = delivery.id;
                if !created_ids.contains(&delivery_id) {
                    continue;
                }

                let wake = EventBuilder::new(Kind::from(KIND_WORKFLOW_AGENT_WAKE as u16), "")
                    .tags([
                        Tag::parse(["p", target.as_str()])
                            .map_err(|e| ActionSinkError::EventBuild(format!("wake p tag: {e}")))?,
                        Tag::parse(["h", channel_id_canonical.as_str()])
                            .map_err(|e| ActionSinkError::EventBuild(format!("wake h tag: {e}")))?,
                        Tag::parse(["delivery", delivery_id.to_string().as_str()]).map_err(
                            |e| ActionSinkError::EventBuild(format!("wake delivery tag: {e}")),
                        )?,
                        Tag::parse(["workflow-definition", definition_event_id.as_str()]).map_err(
                            |e| ActionSinkError::EventBuild(format!("wake definition tag: {e}")),
                        )?,
                        Tag::parse(["workflow-run", doorbell.run_id.to_string().as_str()])
                            .map_err(|e| {
                                ActionSinkError::EventBuild(format!("wake run tag: {e}"))
                            })?,
                        Tag::parse(["workflow-step", step_id.as_str()]).map_err(|e| {
                            ActionSinkError::EventBuild(format!("wake step tag: {e}"))
                        })?,
                        Tag::parse(["message", event_id_hex.as_str()]).map_err(|e| {
                            ActionSinkError::EventBuild(format!("wake message tag: {e}"))
                        })?,
                    ])
                    .sign_with_keys(&state.relay_keypair)
                    .map_err(|e| ActionSinkError::EventBuild(format!("wake signing: {e}")))?;
                state.mark_local_event(tenant.community(), &wake.id);
                state
                    .pubsub
                    .publish_event(&tenant, EventTopic::Channel(channel_uuid), &wake)
                    .await
                    .map_err(|e| ActionSinkError::Database(e.to_string()))?;
                let stored_wake = buzz_core::StoredEvent::new(wake, Some(channel_uuid));
                fan_out_event_to_local_subscribers(&state, tenant.community(), &stored_wake).await;
            }

            // Post-commit side effects only for the newly inserted canonical event.
            if let Some(stored_event) = stored_event.as_ref() {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    stored_event,
                    kind_u32,
                    &author_pubkey_hex,
                    None,
                )
                .await;

                if let Some(owned) = &thread_meta_owned {
                    crate::handlers::side_effects::emit_live_thread_summary(
                        &tenant,
                        &state,
                        channel_uuid,
                        owned.root_event_id.clone(),
                    );
                }
            }

            Ok(event_id_hex)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, pubkey: &str) -> (String, String) {
        (name.to_string(), pubkey.to_string())
    }

    // A 64-char hex pubkey built from a single repeated nibble, for readable tests.
    fn pk(nibble: char) -> String {
        std::iter::repeat_n(nibble, 64).collect()
    }

    #[test]
    fn resolves_exact_member_name() {
        let members = vec![m("Robby", &pk('a'))];
        assert_eq!(
            resolve_mention_pubkeys("heads up @Robby — please take a look", &members),
            vec![pk('a')]
        );
    }

    #[test]
    fn matches_case_insensitively() {
        let members = vec![m("Robby", &pk('a'))];
        assert_eq!(
            resolve_mention_pubkeys("ping @robby", &members),
            vec![pk('a')]
        );
    }

    #[test]
    fn ignores_non_member_and_bare_at() {
        let members = vec![m("Robby", &pk('a'))];
        assert!(resolve_mention_pubkeys("hey @Stranger and @", &members).is_empty());
    }

    #[test]
    fn greedy_longest_binds_full_name_not_prefix() {
        // Both "Will" and "Will Pfleger" are members. `@Will Pfleger` must bind
        // Pfleger's key only; a bare `@Will` binds Will.
        let members = vec![m("Will", &pk('1')), m("Will Pfleger", &pk('2'))];
        assert_eq!(
            resolve_mention_pubkeys("cc @Will Pfleger on this", &members),
            vec![pk('2')]
        );
        assert_eq!(
            resolve_mention_pubkeys("cc @Will on this", &members),
            vec![pk('1')]
        );
    }

    #[test]
    fn at_mid_token_does_not_match() {
        // `@` must sit at a left boundary (start / whitespace / `(`). An email-ish
        // or mid-token `@` (`alice@Robby`) must not wake Robby.
        let members = vec![m("Robby", &pk('a'))];
        assert!(resolve_mention_pubkeys("alice@Robby", &members).is_empty());
    }

    #[test]
    fn prefix_member_does_not_match_inside_longer_word() {
        // "Sam" is a member; `@Sami` (no "Sami" member) must not wake Sam.
        let members = vec![m("Sam", &pk('3'))];
        assert!(resolve_mention_pubkeys("hi @Sami", &members).is_empty());
    }

    #[test]
    fn name_with_spaces_and_punctuation() {
        let members = vec![m("Lep (Subagent)", &pk('4'))];
        assert_eq!(
            resolve_mention_pubkeys("@Lep (Subagent) take it", &members),
            vec![pk('4')]
        );
    }

    #[test]
    fn em_dash_terminates_name() {
        // Generated prose often writes `@Name—text` with no space.
        let members = vec![m("Robby", &pk('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@Robby—please look", &members),
            vec![pk('a')]
        );
    }

    #[test]
    fn non_ascii_member_name() {
        let members = vec![m("Zoë", &pk('5'))];
        assert_eq!(
            resolve_mention_pubkeys("welcome @Zoë!", &members),
            vec![pk('5')]
        );
    }

    #[test]
    fn lowercase_expansion_does_not_shift_later_mentions() {
        // Regression (Wren's redteam counterexample): `İ` (U+0130) lowercases to
        // TWO code points (`i` + U+0307). A design that pre-lowercases the whole
        // text and indexes it in parallel with the original chars desyncs after
        // the expansion, dropping every later valid mention. `@İ @Robby` must
        // resolve BOTH members, in order.
        let members = vec![m("İ", &pk('c')), m("Robby", &pk('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@İ @Robby", &members),
            vec![pk('c'), pk('a')]
        );
    }

    #[test]
    fn sharp_s_matches_case_insensitively() {
        // `ẞ` (U+1E9E capital sharp s) lowercases to `ß` (U+00DF) — a single
        // char, NOT `ss` (that's uppercase/full-case-fold behavior, not
        // `char::to_lowercase`). Covers non-ASCII case-insensitive matching, and
        // that a later mention still resolves after it.
        let members = vec![m("ẞ", &pk('d')), m("Max", &pk('b'))];
        assert_eq!(
            resolve_mention_pubkeys("@ẞ and @Max", &members),
            vec![pk('d'), pk('b')]
        );
    }

    // Adversarial rows from Quinn's re-review (the two `ẞ→ss`-premised ones were
    // dropped as vacuous — `ẞ` lowercases to `ß`, one char, so it never inverts
    // original-vs-folded length; only `İ` does).

    #[test]
    fn producer_gate_blocks_legacy_anyone_mixed_version_rollout() {
        let error = ensure_workflow_agent_delivery_enabled(false)
            .expect_err("disabled producer must not emit a visible kind:9");
        assert!(matches!(error, ActionSinkError::InvalidInput(_)));
        assert!(
            ensure_workflow_agent_delivery_enabled(true).is_ok(),
            "operator may enable delivery after ACP harnesses are upgraded"
        );
    }

    #[test]
    fn combining_mark_in_name_matches() {
        // A name carrying a combining mark (`é` as `e` + U+0301) matches the same
        // sequence in text (1:1 folding) and terminates cleanly.
        let members = vec![m("Jos\u{0065}\u{0301}", &pk('4'))]; // "José" decomposed
        assert_eq!(
            resolve_mention_pubkeys("hi @Jos\u{0065}\u{0301}!", &members),
            vec![pk('4')]
        );
    }

    #[test]
    fn expanding_name_at_trailing_boundary() {
        // Expansion at the very end: `@İ` with nothing after must match, and
        // `@İx` (x extends the name, no `İx` member) must NOT match `İ`.
        let members = vec![m("İ", &pk('5'))];
        assert_eq!(resolve_mention_pubkeys("@İ", &members), vec![pk('5')]);
        assert!(resolve_mention_pubkeys("@İx", &members).is_empty());
    }

    #[test]
    fn back_to_back_at_is_one_mention() {
        // `@İ@Robby`: the second `@` is preceded by a name char (`İ`), so it is
        // NOT at a left boundary — same rule as `alice@Robby`. Back-to-back
        // `@a@b` is intentionally one mention; a separator is required to wake
        // both. The expanding first name (`İ` → 2 folded chars) also proves the
        // span accounting stays in original coordinates.
        let members = vec![m("İ", &pk('5')), m("Robby", &pk('a'))];
        assert_eq!(resolve_mention_pubkeys("@İ@Robby", &members), vec![pk('5')]);
        // ASCII control: same shape, same outcome — it's the boundary rule, not
        // a Unicode span-accounting bug.
        let ascii = vec![m("Sam", &pk('6')), m("Robby", &pk('a'))];
        assert_eq!(resolve_mention_pubkeys("@Sam@Robby", &ascii), vec![pk('6')]);
        // With a separator, both wake.
        assert_eq!(
            resolve_mention_pubkeys("@İ @Robby", &members),
            vec![pk('5'), pk('a')]
        );
    }

    #[test]
    fn ambiguous_name_wakes_no_one() {
        // Six "Fizz" agents (real team case) with distinct pubkeys → tag none.
        let members = vec![
            m("Fizz", &pk('6')),
            m("Fizz", &pk('7')),
            m("Fizz", &pk('8')),
        ];
        assert!(resolve_mention_pubkeys("@Fizz status?", &members).is_empty());
    }

    #[test]
    fn duplicate_name_same_pubkey_is_not_ambiguous() {
        // Same identity listed twice (e.g. two channels) is not a conflict.
        let members = vec![m("Fizz", &pk('6')), m("Fizz", &pk('6'))];
        assert_eq!(resolve_mention_pubkeys("@Fizz go", &members), vec![pk('6')]);
    }

    #[test]
    fn dedupes_repeated_mentions_in_first_appearance_order() {
        let members = vec![m("Robby", &pk('a')), m("Max", &pk('b'))];
        assert_eq!(
            resolve_mention_pubkeys("@Max then @Robby then @Max again", &members),
            vec![pk('b'), pk('a')]
        );
    }
}

#[cfg(test)]
#[path = "workflow_sink/integration_tests.rs"]
mod integration_tests;
