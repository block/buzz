//! Relay-side implementation of [`ActionSink`] for workflow actions.
//!
//! Builds Nostr events, persists them, and delegates post-persist side effects
//! (WebSocket fan-out, Redis pub/sub, search indexing, audit logging) to the
//! existing [`dispatch_persistent_event`] helper.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};

use buzz_core::kind::KIND_STREAM_MESSAGE;
use buzz_core::tenant::CommunityId;
use buzz_workflow::action_sink::{ActionSink, ActionSinkError};
use chrono::Utc;
use nostr::{EventBuilder, Kind, Tag};
use tracing::info;
use uuid::Uuid;

use crate::handlers::event::dispatch_persistent_event;
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

/// Build the tag set for an `assign_agent` kind:9 event.
///
/// Split out of the sink so the tag contract is testable without Postgres —
/// the end-to-end sink tests are `#[ignore]`-gated on a live database, which
/// is exactly how the missing `buzz:workflow-owner` tag went unnoticed.
///
/// The set is:
/// - `actor` (owner) — attribution, matching `send_message`. Attribution
///   deliberately does not ride on a `p` tag: ACP wakes on *any* `p` tag
///   matching an agent's pubkey, so p-tagging the owner woke them as a second
///   agent whenever an agent owned the workflow — which defeats the
///   single-assignee contract this action exists to provide. `actor` is the
///   relay-trusted attribution channel `effective_message_author` already
///   prefers, leaving `p` to mean "wake" and nothing else.
/// - `h` — the destination channel.
/// - `buzz:workflow` — marks the event as workflow-emitted (prevents recursive
///   triggering).
/// - `buzz:workflow-owner` — **load-bearing**. This event is signed by the
///   relay keypair, so buzz-acp gates it through `workflow_attributed_author`,
///   which requires exactly one `["buzz:workflow", "true"]` *and* exactly one
///   `["buzz:workflow-owner", <pubkey>]` and fails closed otherwise. Without
///   it the event is gated on the relay's own pubkey, which `author_allowed`
///   rejects under `owner-only` — the mode official desktop builds hard-clamp
///   — so the assignee never wakes and the action cannot do the one thing it
///   exists for. Mention `p` tags are explicitly *not* used for attribution.
/// - `p` (assignee) — **exactly one**, the sole wake target.
/// - `task` — optional correlation id, trimmed, omitted when empty.
fn assign_agent_tags(
    author_pubkey_hex: &str,
    channel_id_canonical: &str,
    agent_pk_hex: &str,
    task_id: Option<&str>,
) -> Result<Vec<Tag>, ActionSinkError> {
    let mut tags = vec![
        Tag::parse(["actor", author_pubkey_hex])
            .map_err(|e| ActionSinkError::EventBuild(format!("actor tag: {e}")))?,
        Tag::parse(["h", channel_id_canonical])
            .map_err(|e| ActionSinkError::EventBuild(format!("h tag: {e}")))?,
        Tag::parse(["buzz:workflow", "true"])
            .map_err(|e| ActionSinkError::EventBuild(format!("workflow tag: {e}")))?,
        Tag::parse(["buzz:workflow-owner", author_pubkey_hex])
            .map_err(|e| ActionSinkError::EventBuild(format!("workflow owner tag: {e}")))?,
        // The one and only wake target. No dedup against the owner is needed
        // now that the owner is attributed via `actor` rather than `p`: a
        // self-assignment is a genuine assignment and must still wake.
        // No reverse-parse of `@Name` mentions in `text` either — that is the
        // failure mode assign_agent exists to avoid.
        Tag::parse(["p", agent_pk_hex])
            .map_err(|e| ActionSinkError::EventBuild(format!("assignee p tag: {e}")))?,
    ];
    // Emit the trimmed id, not the caller's string: definition-time validation
    // checks `tid.trim()` but stores the original, so a padded `task_id` would
    // otherwise put whitespace on the wire and break correlation for every
    // reader that compares it verbatim.
    if let Some(tid) = task_id.map(str::trim) {
        if !tid.is_empty() {
            tags.push(
                Tag::parse(["task", tid])
                    .map_err(|e| ActionSinkError::EventBuild(format!("task tag: {e}")))?,
            );
        }
    }
    Ok(tags)
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
    fn send_message(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>> {
        let channel_id = channel_id.to_owned();
        let text = text.to_owned();
        let author_pubkey = author_pubkey.to_owned();
        let reply_to = reply_to.map(str::to_owned);

        Box::pin(async move {
            // 0. Upgrade weak reference — fails only during shutdown.
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| ActionSinkError::Database("relay is shutting down".into()))?;

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
            //    - `actor` tag attributes the message to the workflow owner.
            //      Attribution deliberately does NOT ride on a `p` tag: ACP
            //      wakes on *any* `p` tag matching an agent's pubkey, so
            //      p-tagging the owner woke them as a second agent whenever an
            //      agent owned the workflow. `actor` is the relay-trusted
            //      attribution channel `effective_message_author` already
            //      prefers, so `p` can mean "wake" and nothing else.
            //    - `h` tag scopes to the channel (NIP-29, canonical UUID)
            //    - `buzz:workflow` tag prevents recursive workflow triggering
            //    - one `p` tag per `@Name` that resolves to a channel member,
            //      so mentioned agents are woken (wake is `p`-tag gated)
            let mut tags = vec![
                Tag::parse(["actor", &author_pubkey_hex])
                    .map_err(|e| ActionSinkError::EventBuild(format!("actor tag: {e}")))?,
                Tag::parse(["h", &channel_id_canonical])
                    .map_err(|e| ActionSinkError::EventBuild(format!("h tag: {e}")))?,
                Tag::parse(["buzz:workflow", "true"])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow tag: {e}")))?,
            ];

            // Resolve thread ancestry when this is a threaded reply, so the
            // built event carries NIP-10 `root`/`reply` e-tags and persists real
            // thread metadata (matching the ingest path) instead of top-level.
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

            // NIP-10 e-tags for the thread. Marked `root`/`reply` so clients and
            // the ingest resolver read the ancestry the same way. A direct reply
            // (parent == root) emits a single `reply` tag; a nested reply emits
            // the `root` + `reply` pair — matching `buzz_sdk::builders::thread_tags`
            // so every writer produces one wire shape per reply kind.
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

            // Resolve `@Name` mentions to channel-member pubkeys and append a
            // `p` tag for each (skipping the author, already tagged above). A
            // resolution failure must not drop the message, so log and proceed
            // with the base tags.
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
            // The owner is attributed via `actor`, never p-tagged, so they are
            // not woken by their own workflow's output even if the text names
            // them. Skipping them here keeps that true.
            for mentioned in resolve_mention_pubkeys(&text, &named_members) {
                if mentioned == author_pubkey_hex {
                    continue;
                }
                tags.push(
                    Tag::parse(["p", &mentioned])
                        .map_err(|e| ActionSinkError::EventBuild(format!("mention p tag: {e}")))?,
                );
            }

            let kind = Kind::from(KIND_STREAM_MESSAGE as u16);
            let event = EventBuilder::new(kind, &text)
                .tags(tags)
                .sign_with_keys(&state.relay_keypair)
                .map_err(|e| ActionSinkError::EventBuild(format!("signing: {e}")))?;

            let event_id_hex = event.id.to_hex();
            let event_id_bytes = event.id.as_bytes().to_vec();
            let kind_u32 = KIND_STREAM_MESSAGE;

            let event_created_at = {
                let ts = event.created_at.as_secs() as i64;
                chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
            };

            info!(
                event_id = %event_id_hex,
                channel_id = %channel_id_canonical,
                author = %author_pubkey,
                "Workflow SendMessage: posting kind {kind_u32} event"
            );

            // 4. Persist event with thread metadata (matches REST handler path).
            //    Threaded replies persist the resolved parent/root/depth; a
            //    non-reply workflow message stays top-level (depth=0, no parent).
            let thread_meta_owned = reply_ancestry.map(|ancestry| {
                ancestry.into_thread_meta(event_id_bytes.clone(), event_created_at, channel_uuid)
            });
            let thread_meta = Some(match &thread_meta_owned {
                Some(owned) => owned.as_params(),
                None => buzz_db::event::ThreadMetadataParams {
                    event_id: &event_id_bytes,
                    event_created_at,
                    channel_id: channel_uuid,
                    parent_event_id: None,
                    parent_event_created_at: None,
                    root_event_id: None,
                    root_event_created_at: None,
                    depth: 0,
                    broadcast: false,
                },
            });

            let (stored_event, was_inserted) = state
                .db
                .insert_event_with_thread_metadata(
                    tenant.community(),
                    &event,
                    Some(channel_uuid),
                    thread_meta,
                )
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;

            // 5. Post-persist side effects (fan-out, search, audit)
            //    Only if actually inserted (idempotency guard).
            if was_inserted {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    &stored_event,
                    kind_u32,
                    &author_pubkey_hex,
                    None,
                )
                .await;

                // A threaded reply changed its thread's counters — push a fresh
                // relay-signed kind:39005 so subscribed clients update badge
                // counts without refetching the head window, exactly as the
                // ingest path does after a reply insert. Fan-out-only and
                // best-effort; skipped for top-level (non-reply) messages.
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

    fn assign_agent(
        &self,
        community_id: CommunityId,
        channel_id: &str,
        text: &str,
        author_pubkey: &str,
        agent_pubkey: &str,
        task_id: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>> {
        let channel_id = channel_id.to_owned();
        let text = text.to_owned();
        let author_pubkey = author_pubkey.to_owned();
        let agent_pubkey = agent_pubkey.to_owned();
        let task_id = task_id.map(str::to_owned);

        Box::pin(async move {
            // 0. Upgrade weak reference — fails only during shutdown.
            let state = self
                .state
                .upgrade()
                .ok_or_else(|| ActionSinkError::Database("relay is shutting down".into()))?;

            // 1. Resolve tenant for the run's community (see send_message for
            //    rationale). Fail closed if the community is no longer mapped.
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

            // 2. Validate text.
            if text.trim().is_empty() {
                return Err(ActionSinkError::EmptyContent);
            }

            // 3. Parse/canonicalize channel UUID and look up channel.
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

            // 4. Parse author (workflow owner) and verify their access.
            let author_pubkey = nostr::PublicKey::from_hex(&author_pubkey).map_err(|e| {
                ActionSinkError::InvalidInput(format!("invalid author pubkey: {e}"))
            })?;
            let author_pubkey_bytes = author_pubkey.to_bytes().to_vec();
            let author_pubkey_hex = author_pubkey.to_hex();
            let owner_is_member = state
                .is_member_cached(tenant.community(), channel_uuid, &author_pubkey_bytes)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            if !owner_is_member && channel.visibility != "open" {
                return Err(ActionSinkError::InvalidInput(
                    "workflow owner does not have access to destination channel".into(),
                ));
            }

            // 5. Parse assignee and enforce membership (fail-closed).
            //    The assignee must already be a channel member; silently
            //    adding them would let a workflow escalate authority beyond
            //    what the owner granted at save time.
            let agent_pk = nostr::PublicKey::from_hex(&agent_pubkey)
                .map_err(|e| ActionSinkError::InvalidInput(format!("invalid agent pubkey: {e}")))?;
            let agent_pk_bytes = agent_pk.to_bytes().to_vec();
            let agent_pk_hex = agent_pk.to_hex();
            let agent_is_member = state
                .is_member_cached(tenant.community(), channel_uuid, &agent_pk_bytes)
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;
            if !agent_is_member {
                return Err(ActionSinkError::AssigneeNotMember(agent_pk_hex));
            }

            // 6. Build the kind:9 event.
            let tags = assign_agent_tags(
                &author_pubkey_hex,
                &channel_id_canonical,
                &agent_pk_hex,
                task_id.as_deref(),
            )?;

            let kind = Kind::from(KIND_STREAM_MESSAGE as u16);
            let event = EventBuilder::new(kind, &text)
                .tags(tags)
                .sign_with_keys(&state.relay_keypair)
                .map_err(|e| ActionSinkError::EventBuild(format!("signing: {e}")))?;

            let event_id_hex = event.id.to_hex();
            let event_id_bytes = event.id.as_bytes().to_vec();
            let kind_u32 = KIND_STREAM_MESSAGE;

            let event_created_at = {
                let ts = event.created_at.as_secs() as i64;
                chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
            };

            info!(
                event_id = %event_id_hex,
                channel_id = %channel_id_canonical,
                author = %author_pubkey,
                agent = %agent_pk_hex,
                "Workflow AssignAgent: posting kind {kind_u32} event"
            );

            // 7. Persist with thread metadata (top-level, same as send_message).
            let thread_meta = Some(buzz_db::event::ThreadMetadataParams {
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

            let (stored_event, was_inserted) = state
                .db
                .insert_event_with_thread_metadata(
                    tenant.community(),
                    &event,
                    Some(channel_uuid),
                    thread_meta,
                )
                .await
                .map_err(|e| ActionSinkError::Database(e.to_string()))?;

            // 8. Post-persist fan-out (only on real insert).
            if was_inserted {
                let _ = dispatch_persistent_event(
                    &tenant,
                    &state,
                    &stored_event,
                    kind_u32,
                    &author_pubkey_hex,
                    None,
                )
                .await;
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
    /// Values of every tag with the given name, in order.
    fn tag_values<'a>(tags: &'a [Tag], name: &str) -> Vec<&'a str> {
        tags.iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some(name))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect()
    }

    /// The load-bearing one. An assign_agent event is signed by the relay
    /// keypair, so buzz-acp only accepts it via `workflow_attributed_author`,
    /// which needs exactly one `buzz:workflow-owner`. Without it the event is
    /// gated on the relay pubkey and dropped under `owner-only` — the mode
    /// shipped desktop builds hard-clamp — so the assignee never wakes.
    #[test]
    fn assign_agent_names_the_owner_for_the_harness_author_gate() {
        let owner = pk('a');
        let agent = pk('b');
        let tags = assign_agent_tags(&owner, "chan-1", &agent, None).expect("tags build");

        assert_eq!(
            tag_values(&tags, "buzz:workflow-owner"),
            vec![owner.as_str()],
            "exactly one buzz:workflow-owner naming the workflow owner"
        );
        assert_eq!(
            tag_values(&tags, "buzz:workflow"),
            vec!["true"],
            "exactly one buzz:workflow marker — the gate rejects duplicates"
        );
        // Exactly one `p` tag: the assignee. ACP wakes on any `p` matching an
        // agent's pubkey, so an owner `p` here would wake the owner as a
        // second agent whenever an agent owns the workflow.
        assert_eq!(
            tag_values(&tags, "p"),
            vec![agent.as_str()],
            "the assignee must be the only wake target"
        );
        assert_eq!(
            tag_values(&tags, "actor"),
            vec![owner.as_str()],
            "owner attribution rides on `actor`, which effective_message_author prefers"
        );
    }

    /// A workflow may assign its own owner. That is a genuine assignment and
    /// must still wake them — the owner is only excluded from `p` when they are
    /// *not* the assignee, and attribution is unaffected either way because it
    /// rides on `actor`.
    #[test]
    fn assign_agent_self_assignment_still_wakes_the_owner() {
        let owner = pk('c');
        let tags = assign_agent_tags(&owner, "chan-1", &owner, None).expect("tags build");

        assert_eq!(
            tag_values(&tags, "p"),
            vec![owner.as_str()],
            "self-assignment is an assignment: exactly one wake, no duplicate"
        );
        assert_eq!(tag_values(&tags, "actor"), vec![owner.as_str()]);
        assert_eq!(
            tag_values(&tags, "buzz:workflow-owner"),
            vec![owner.as_str()]
        );
    }

    /// Definition validation checks `task_id.trim()` but stores the original,
    /// so an untrimmed emit would put whitespace on the wire and break
    /// correlation for any reader comparing it verbatim.
    #[test]
    fn assign_agent_task_id_is_trimmed_and_dropped_when_blank() {
        let owner = pk('d');
        let agent = pk('e');

        let padded = assign_agent_tags(&owner, "c", &agent, Some("  task-7  ")).expect("build");
        assert_eq!(tag_values(&padded, "task"), vec!["task-7"]);

        let blank = assign_agent_tags(&owner, "c", &agent, Some("   ")).expect("build");
        assert!(
            tag_values(&blank, "task").is_empty(),
            "a whitespace-only correlation id must not reach the wire"
        );

        let absent = assign_agent_tags(&owner, "c", &agent, None).expect("build");
        assert!(tag_values(&absent, "task").is_empty());
    }
}

#[cfg(test)]
mod integration_tests {
    //! Regression test for `e3661764` / `7899c1a8`: a workflow `send_message`
    //! that mentions a channel member by name (`@Name`) must emit a `p` tag for
    //! that member so ACP agent wake (`event_mentions_agent`, p-tag gated) fires.
    //!
    //! Postgres-gated like the other DB-backed relay tests. Run with:
    //!   `cargo test -p buzz-relay --lib workflow_sink -- --ignored`
    use super::*;
    use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
    use buzz_db::CreateCommunityWithOwnerResult;
    use std::sync::Arc;

    /// Real-PG state mirroring `handlers::event::tests::test_state_with_redis_url`.
    async fn test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_send_message_p_tags_mentioned_member() {
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();
        let agent = nostr::Keys::generate();
        let agent_hex = agent.public_key().to_hex();
        let agent_bytes = agent.public_key().to_bytes().to_vec();

        let host = format!("wf-ptag-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        // Open channel; the creator (author) is bootstrapped as an owner-member.
        let channel = state
            .db
            .create_channel(
                community,
                "wf-ptag",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        // The mentioned agent is a real member with a resolvable display name.
        state
            .db
            .ensure_user(community, &agent_bytes)
            .await
            .expect("ensure agent user row");
        state
            .db
            .update_user_profile(community, &agent_bytes, Some("Robby"), None, None, None)
            .await
            .expect("set agent display name");
        state
            .db
            .add_member(
                community,
                channel.id,
                &agent_bytes,
                MemberRole::Bot,
                Some(&author.public_key().to_bytes()),
            )
            .await
            .expect("add agent member");

        let sink = RelayActionSink::new(&state);
        let event_id_hex = sink
            .send_message(
                community,
                &channel.id.to_string(),
                "heads up @Robby — please take a look",
                &author_hex,
                None,
            )
            .await
            .expect("send_message");

        let id_bytes = nostr::EventId::from_hex(&event_id_hex)
            .expect("event id")
            .as_bytes()
            .to_vec();
        let stored = state
            .db
            .get_event_by_id(community, &id_bytes)
            .await
            .expect("query event")
            .expect("event persisted");

        let p_tag_targets: Vec<&str> = stored
            .event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect();

        // Attribution moved off `p` and onto `actor`: ACP wakes on any `p`
        // matching an agent's pubkey, so p-tagging the owner woke them as an
        // extra agent whenever an agent owned the workflow.
        let actor_targets: Vec<&str> = stored
            .event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("actor"))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect();
        assert_eq!(
            actor_targets,
            vec![author_hex.as_str()],
            "author must be attributed via the actor tag; got {actor_targets:?}"
        );
        assert!(
            !p_tag_targets.contains(&author_hex.as_str()),
            "author must NOT be p-tagged — that wakes them as a second agent; got {p_tag_targets:?}"
        );
        assert!(
            p_tag_targets.contains(&agent_hex.as_str()),
            "mentioned member {agent_hex} must be p-tagged so it wakes; got {p_tag_targets:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_reply_in_thread_threads_onto_parent() {
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();

        let host = format!("wf-thread-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        let channel = state
            .db
            .create_channel(
                community,
                "wf-thread",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        let sink = RelayActionSink::new(&state);

        // 1. A top-level workflow message becomes the thread root.
        let root_hex = sink
            .send_message(
                community,
                &channel.id.to_string(),
                "root message",
                &author_hex,
                None,
            )
            .await
            .expect("send root");

        // 2. A reply_in_thread message threads onto it.
        let reply_hex = sink
            .send_message(
                community,
                &channel.id.to_string(),
                "threaded reply",
                &author_hex,
                Some(&root_hex),
            )
            .await
            .expect("send reply");

        // A direct reply carries a single NIP-10 reply e-tag at the root (no
        // root marker), matching SDK `thread_tags`.
        let reply_id_bytes = nostr::EventId::from_hex(&reply_hex)
            .expect("reply id")
            .as_bytes()
            .to_vec();
        let stored = state
            .db
            .get_event_by_id(community, &reply_id_bytes)
            .await
            .expect("query reply")
            .expect("reply persisted");
        let marker = |m: &str| -> Option<String> {
            stored.event.tags.iter().find_map(|t| {
                let p = t.as_slice();
                if p.len() >= 4 && p[0] == "e" && p[3] == m {
                    Some(p[1].clone())
                } else {
                    None
                }
            })
        };
        assert_eq!(
            marker("reply").as_deref(),
            Some(root_hex.as_str()),
            "direct reply emits a single reply marker at the root"
        );
        assert_eq!(
            marker("root"),
            None,
            "direct reply omits the root marker (matches SDK thread_tags)"
        );

        // Thread metadata reflects a depth-1 reply parented on the root.
        let meta = state
            .db
            .get_thread_metadata_by_event(community, &reply_id_bytes)
            .await
            .expect("query meta")
            .expect("reply has thread metadata");
        assert_eq!(
            meta.depth, 1,
            "direct reply to a top-level message is depth 1"
        );
        let root_bytes = nostr::EventId::from_hex(&root_hex)
            .expect("root id")
            .as_bytes()
            .to_vec();
        assert_eq!(meta.parent_event_id.as_deref(), Some(root_bytes.as_slice()));
        assert_eq!(meta.root_event_id.as_deref(), Some(root_bytes.as_slice()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_replies_recover_metadata_less_parent_ancestry() {
        // A parent that carries NIP-10 root/reply markers but has NO
        // thread_metadata row (legacy or not-yet-indexed) must be recognized as
        // nested: the workflow reply threads at depth 2 onto the parent's own
        // root, not a false top-level depth 1.
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();

        let host = format!("wf-legacy-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        let channel = state
            .db
            .create_channel(
                community,
                "wf-legacy",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        let channel_hex = channel.id.to_string();

        // A top-level root message, inserted WITHOUT any thread metadata row.
        let root_event = EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), "root")
            .tags([Tag::parse(["h", &channel_hex]).expect("h tag")])
            .sign_with_keys(&author)
            .expect("sign root");
        let root_hex = root_event.id.to_hex();
        state
            .db
            .insert_event(community, &root_event, Some(channel.id))
            .await
            .expect("insert root");

        // A nested parent that marks its root/reply — but, crucially, is stored
        // with NO thread_metadata row (the legacy/unindexed case F1 addresses).
        let parent_event =
            EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), "nested parent")
                .tags([
                    Tag::parse(["h", &channel_hex]).expect("h tag"),
                    Tag::parse(["e", &root_hex, "", "root"]).expect("root tag"),
                    Tag::parse(["e", &root_hex, "", "reply"]).expect("reply tag"),
                ])
                .sign_with_keys(&author)
                .expect("sign parent");
        let parent_hex = parent_event.id.to_hex();
        state
            .db
            .insert_event(community, &parent_event, Some(channel.id))
            .await
            .expect("insert parent");
        assert!(
            state
                .db
                .get_thread_metadata_by_event(community, parent_event.id.as_bytes())
                .await
                .expect("query parent meta")
                .is_none(),
            "test premise: the nested parent must have no thread_metadata row"
        );

        // A workflow reply onto the metadata-less nested parent.
        let reply_hex = RelayActionSink::new(&state)
            .send_message(
                community,
                &channel_hex,
                "workflow reply",
                &author_hex,
                Some(&parent_hex),
            )
            .await
            .expect("send reply");

        let reply_id_bytes = nostr::EventId::from_hex(&reply_hex)
            .expect("reply id")
            .as_bytes()
            .to_vec();
        let meta = state
            .db
            .get_thread_metadata_by_event(community, &reply_id_bytes)
            .await
            .expect("query meta")
            .expect("reply has thread metadata");

        assert_eq!(
            meta.depth, 2,
            "reply to a marked-but-unindexed nested parent is depth 2, not top-level"
        );
        let root_bytes = nostr::EventId::from_hex(&root_hex)
            .expect("root id")
            .as_bytes()
            .to_vec();
        let parent_bytes = parent_event.id.as_bytes().to_vec();
        assert_eq!(
            meta.root_event_id.as_deref(),
            Some(root_bytes.as_slice()),
            "root recovered from the parent's own NIP-10 markers"
        );
        assert_eq!(
            meta.parent_event_id.as_deref(),
            Some(parent_bytes.as_slice())
        );

        // The reply's own NIP-10 e-tags point root→the recovered root,
        // reply→the immediate parent (matching the ingest resolver).
        let stored = state
            .db
            .get_event_by_id(community, &reply_id_bytes)
            .await
            .expect("query reply")
            .expect("reply persisted");
        let marker = |m: &str| -> Option<String> {
            stored.event.tags.iter().find_map(|t| {
                let p = t.as_slice();
                if p.len() >= 4 && p[0] == "e" && p[3] == m {
                    Some(p[1].clone())
                } else {
                    None
                }
            })
        };
        assert_eq!(marker("root").as_deref(), Some(root_hex.as_str()));
        assert_eq!(marker("reply").as_deref(), Some(parent_hex.as_str()));

        // A root-only parent is top-level under the shared collapse rule, even
        // without metadata. A workflow reply therefore starts a thread at P,
        // rather than incorrectly inheriting the marker's unrelated root R.
        let root_only_parent =
            EventBuilder::new(Kind::from(KIND_STREAM_MESSAGE as u16), "root-only parent")
                .tags([
                    Tag::parse(["h", &channel_hex]).expect("h tag"),
                    Tag::parse(["e", &root_hex, "", "root"]).expect("root tag"),
                ])
                .sign_with_keys(&author)
                .expect("sign root-only parent");
        let root_only_parent_hex = root_only_parent.id.to_hex();
        let root_only_parent_bytes = root_only_parent.id.as_bytes().to_vec();
        state
            .db
            .insert_event(community, &root_only_parent, Some(channel.id))
            .await
            .expect("insert root-only parent");

        let root_only_reply_hex = RelayActionSink::new(&state)
            .send_message(
                community,
                &channel_hex,
                "workflow reply to root-only parent",
                &author_hex,
                Some(&root_only_parent_hex),
            )
            .await
            .expect("send root-only reply");
        let root_only_reply_bytes = nostr::EventId::from_hex(&root_only_reply_hex)
            .expect("reply id")
            .as_bytes()
            .to_vec();
        let root_only_meta = state
            .db
            .get_thread_metadata_by_event(community, &root_only_reply_bytes)
            .await
            .expect("query root-only reply meta")
            .expect("root-only reply has thread metadata");
        assert_eq!(root_only_meta.depth, 1);
        assert_eq!(
            root_only_meta.parent_event_id.as_deref(),
            Some(root_only_parent_bytes.as_slice())
        );
        assert_eq!(
            root_only_meta.root_event_id.as_deref(),
            Some(root_only_parent_bytes.as_slice())
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_reply_to_missing_parent_errors() {
        let state = test_state().await;
        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();
        let host = format!("wf-missing-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };
        let channel = state
            .db
            .create_channel(
                community,
                "wf-missing",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        let unknown = nostr::Keys::generate().public_key().to_hex();
        let err = RelayActionSink::new(&state)
            .send_message(
                community,
                &channel.id.to_string(),
                "orphan reply",
                &author_hex,
                Some(&unknown),
            )
            .await
            .expect_err("reply to a non-existent parent must fail");
        assert!(
            matches!(err, ActionSinkError::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    /// The identity-safety contract for `assign_agent`, exercised end to end
    /// against real Postgres. The three assertions map directly to Airy's
    /// review corrections (singular assignee, no prose parsing, membership
    /// fail-closed):
    ///
    /// 1. Two channel members share the display name "Winnie" (the exact
    ///    duplicate-name repro from #4108b496). Dispatching by pubkey wakes
    ///    exactly the selected pubkey — never the other, never both.
    /// 2. The message text contains `@Winnie`, which under `send_message`
    ///    would be dropped as ambiguous. `assign_agent` must NOT reverse-parse
    ///    that name; the `p`-tag set must be exactly `{owner, selected agent}`.
    /// 3. A non-member assignee is rejected with `AssigneeNotMember` — never
    ///    silently added, never posted-but-unwaked.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_assign_agent_wakes_exact_pubkey_and_ignores_prose() {
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();

        // Two agents sharing the display name "Winnie" (the duplicate-name
        // hazard driving Slice 1). One is chosen; the other must not wake.
        let winnie_a = nostr::Keys::generate();
        let winnie_a_bytes = winnie_a.public_key().to_bytes().to_vec();
        let winnie_a_hex = winnie_a.public_key().to_hex();
        let winnie_b = nostr::Keys::generate();
        let winnie_b_bytes = winnie_b.public_key().to_bytes().to_vec();
        let winnie_b_hex = winnie_b.public_key().to_hex();

        let host = format!("wf-assign-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        let channel = state
            .db
            .create_channel(
                community,
                "wf-assign",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        for bytes in [&winnie_a_bytes, &winnie_b_bytes] {
            state
                .db
                .ensure_user(community, bytes)
                .await
                .expect("ensure user row");
            state
                .db
                .update_user_profile(community, bytes, Some("Winnie"), None, None, None)
                .await
                .expect("set display name");
            state
                .db
                .add_member(
                    community,
                    channel.id,
                    bytes,
                    MemberRole::Bot,
                    Some(&author.public_key().to_bytes()),
                )
                .await
                .expect("add member");
        }

        let sink = RelayActionSink::new(&state);
        let event_id_hex = sink
            .assign_agent(
                community,
                &channel.id.to_string(),
                // Prose contains `@Winnie` — under send_message this is
                // ambiguous and drops. assign_agent must NOT reverse-parse it.
                "@Winnie please pick this up",
                &author_hex,
                &winnie_a_hex,
                Some("11111111-2222-3333-4444-555555555555"),
            )
            .await
            .expect("assign_agent");

        let id_bytes = nostr::EventId::from_hex(&event_id_hex)
            .expect("event id")
            .as_bytes()
            .to_vec();
        let stored = state
            .db
            .get_event_by_id(community, &id_bytes)
            .await
            .expect("query event")
            .expect("event persisted");

        let p_tag_targets: Vec<&str> = stored
            .event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect();

        // Exactly ONE `p` tag: the selected agent. The owner is attributed via
        // `actor`, not `p` — ACP wakes on any `p` matching an agent's pubkey,
        // so an owner `p` woke them as a second agent and broke the
        // single-assignee contract this action exists to provide.
        assert_eq!(
            p_tag_targets,
            vec![winnie_a_hex.as_str()],
            "the assignee must be the sole wake target; got {p_tag_targets:?}"
        );
        let actor_targets: Vec<&str> = stored
            .event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("actor"))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect();
        assert_eq!(
            actor_targets,
            vec![author_hex.as_str()],
            "owner must be attributed via actor; got {actor_targets:?}"
        );
        // The other same-name member must NOT wake.
        assert!(
            !p_tag_targets.contains(&winnie_b_hex.as_str()),
            "second same-name member {winnie_b_hex} must NOT be p-tagged"
        );

        // The `task` correlation id is present.
        let task_tag = stored
            .event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|s| s.as_str()) == Some("task"))
            .expect("task tag present");
        assert_eq!(
            task_tag.as_slice().get(1).map(|s| s.as_str()),
            Some("11111111-2222-3333-4444-555555555555")
        );

        // The owner must be named in `buzz:workflow-owner`, not merely p-tagged.
        // This event is signed by the relay keypair, so buzz-acp's
        // `workflow_attributed_author` is the only reason the harness accepts
        // it at all — and that gate fails closed without exactly one of these
        // tags, leaving the event gated on the relay's own pubkey, which
        // `author_allowed` rejects under `owner-only`. Official desktop builds
        // hard-clamp that mode, so without this tag the assignee never wakes
        // and the action cannot do the one thing it exists for. The p tags
        // above do not substitute: the gate explicitly ignores them for
        // attribution.
        let owner_tags: Vec<&str> = stored
            .event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("buzz:workflow-owner"))
            .filter_map(|t| t.as_slice().get(1).map(|s| s.as_str()))
            .collect();
        assert_eq!(
            owner_tags,
            vec![author_hex.as_str()],
            "assign_agent must emit exactly one buzz:workflow-owner naming the \
             workflow owner, or the harness drops the event; got {owner_tags:?}"
        );
    }

    /// A non-member assignee must be rejected fail-closed — the workflow
    /// owner's authority cannot be silently extended by mid-run membership
    /// changes.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_assign_agent_rejects_non_member() {
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();
        let stranger = nostr::Keys::generate();
        let stranger_hex = stranger.public_key().to_hex();

        let host = format!("wf-nonmember-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex)
            .await
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        let channel = state
            .db
            .create_channel(
                community,
                "wf-nonmember",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &author.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        // stranger is deliberately NOT added as a channel member.
        let sink = RelayActionSink::new(&state);
        let err = sink
            .assign_agent(
                community,
                &channel.id.to_string(),
                "please pick this up",
                &author_hex,
                &stranger_hex,
                None,
            )
            .await
            .expect_err("assign_agent must fail when assignee is not a member");

        match err {
            ActionSinkError::AssigneeNotMember(pk) => {
                assert_eq!(pk, stranger_hex);
            }
            other => panic!("expected AssigneeNotMember, got: {other}"),
        }
    }
}
