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

/// Append legacy routing tags from rendered output and authority-bearing tags
/// only for targets also named in the workflow owner's stored step template.
fn append_workflow_mention_tags(
    tags: &mut Vec<Tag>,
    rendered_text: &str,
    authored_text: &str,
    members: &[(String, String)],
) -> Result<(), ActionSinkError> {
    let rendered_mentions = resolve_mention_pubkeys(rendered_text, members);
    let authored_mentions: std::collections::HashSet<String> =
        resolve_mention_pubkeys(authored_text, members)
            .into_iter()
            .collect();

    for mentioned in rendered_mentions {
        tags.push(
            Tag::parse(["p", &mentioned])
                .map_err(|e| ActionSinkError::EventBuild(format!("mention p tag: {e}")))?,
        );
        if authored_mentions.contains(&mentioned) {
            tags.push(
                Tag::parse(["buzz:workflow-mention", &mentioned]).map_err(|e| {
                    ActionSinkError::EventBuild(format!("workflow mention tag: {e}"))
                })?,
            );
        }
    }
    Ok(())
}

/// `users.display_name` is `VARCHAR(255)`, so no stored name is longer than
/// 255 characters, and an ASCII name folds to at most 255 characters.
const MAX_DISPLAY_NAME_CHARS: usize = 255;

/// Most distinct `@` tails one message sends to the candidate query. Past
/// this the sink reads every named member, as it always did before.
const MAX_MENTION_TAILS: usize = 32;

/// What [`load_mention_candidates`] reads for one message.
#[derive(Debug, PartialEq, Eq)]
enum MentionCandidateQuery {
    /// The rendered text has no mention anchor, so it can resolve no mention
    /// and no tag is emitted whatever the members are: no read at all.
    NoAnchor,
    /// Read only the members this filter admits.
    Names(buzz_db::channel_members::MentionNameFilter),
    /// More tails than [`MAX_MENTION_TAILS`]: read every named member.
    AllMembers,
}

/// Build the candidate query for the rendered and authored texts.
///
/// [`resolve_mention_pubkeys`] matches a name only right after an `@` at a
/// left boundary (start, whitespace or `(`): the folded name must equal the
/// folded text that follows. For every such anchor in either text this
/// collects the *tail*, the text after the `@` folded per char with
/// `char::to_lowercase` exactly as the resolver folds it, cut to
/// [`MAX_DISPLAY_NAME_CHARS`]. The filter then admits a superset of every
/// name the resolver can match at any anchor:
///
/// - An ASCII name folds to its ASCII lowercase, so it can only match if that
///   is a prefix of some tail. The database lowercases with `translate` over
///   `A`–`Z`, which is the same fold with no locale involved.
/// - Any other name can only match if the lowercase of its first char starts
///   with the first char of some tail (`str::to_lowercase` folds a first char
///   like `char::to_lowercase` does: the final-sigma rule needs a cased letter
///   before it). So the filter sends every char whose lowercase starts with
///   that char ([`lowercase_first_preimage`]) and the database compares only
///   the name's first code point. Postgres `lower()` can disagree with Rust
///   (`İ`, `ẞ`, final `Σ`, locale rules), so it is never used.
///
/// Names the resolver can't match at any anchor neither bind nor consume a
/// span, so dropping them changes nothing; every member sharing a matched
/// folded name matches at the same anchor, so the ambiguity rule still sees
/// all of them. Authored tails are included because authority is resolved
/// against the authored text with the same candidates.
fn mention_candidate_query(rendered_text: &str, authored_text: &str) -> MentionCandidateQuery {
    let mut tails: Vec<String> = Vec::new();
    if !push_mention_tails(rendered_text, &mut tails) {
        return MentionCandidateQuery::AllMembers;
    }
    if tails.is_empty() {
        return MentionCandidateQuery::NoAnchor;
    }
    if !push_mention_tails(authored_text, &mut tails) {
        return MentionCandidateQuery::AllMembers;
    }

    let mut first_chars: Vec<String> = Vec::new();
    for first in tails.iter().filter_map(|tail| tail.chars().next()) {
        for c in lowercase_first_preimage(first) {
            let c = c.to_string();
            if !first_chars.contains(&c) {
                first_chars.push(c);
            }
        }
    }
    MentionCandidateQuery::Names(buzz_db::channel_members::MentionNameFilter {
        lowercase_tails: tails,
        first_chars,
    })
}

/// Add the folded tail after every left-boundary `@` in `text` to `tails`.
/// A tail that is a prefix of another admits no name the longer one doesn't
/// (same first char, and a name prefixing it prefixes the longer one), so
/// only the longest of such tails is kept; repeated mentions of one name
/// then cost one tail. Returns `false`, and stops, once there are more than
/// [`MAX_MENTION_TAILS`].
fn push_mention_tails(text: &str, tails: &mut Vec<String>) -> bool {
    let mut previous: Option<char> = None;
    for (index, c) in text.char_indices() {
        let at_boundary = previous.is_none_or(|p| p.is_whitespace() || p == '(');
        previous = Some(c);
        if c != '@' || !at_boundary {
            continue;
        }
        let tail: String = text[index + '@'.len_utf8()..]
            .chars()
            .flat_map(char::to_lowercase)
            .take(MAX_DISPLAY_NAME_CHARS)
            .collect();
        // A bare trailing `@` names no one: empty names never match.
        if tail.is_empty() || tails.iter().any(|kept| kept.starts_with(&tail)) {
            continue;
        }
        tails.retain(|kept| !tail.starts_with(kept.as_str()));
        if tails.len() == MAX_MENTION_TAILS {
            return false;
        }
        tails.push(tail);
    }
    true
}

/// Every char whose `char::to_lowercase` starts with `first`.
///
/// The table of chars that lowercase to something else is built once by
/// scanning all of Unicode, so it is exact for this Rust version's case
/// tables (for example `i` yields `i`, `I` and `İ`; `k` yields `k`, `K` and
/// the Kelvin sign).
fn lowercase_first_preimage(first: char) -> impl Iterator<Item = char> {
    static CHANGED_BY_LOWERCASE: std::sync::LazyLock<std::collections::HashMap<char, Vec<char>>> =
        std::sync::LazyLock::new(|| {
            let mut preimage: std::collections::HashMap<char, Vec<char>> =
                std::collections::HashMap::new();
            for c in '\0'..=char::MAX {
                if let Some(lower) = c.to_lowercase().next().filter(|&lower| lower != c) {
                    preimage.entry(lower).or_default().push(c);
                }
            }
            preimage
        });
    let itself = (first.to_lowercase().next() == Some(first)).then_some(first);
    itself.into_iter().chain(
        CHANGED_BY_LOWERCASE
            .get(&first)
            .into_iter()
            .flatten()
            .copied(),
    )
}

/// Load `(display_name, pubkey_hex)` for the channel members that `@`
/// mentions in `rendered_text` (and, for authority, `authored_text`) could
/// resolve to. See [`mention_candidate_query`] for why the result resolves
/// exactly as the full roster would.
async fn load_mention_candidates(
    state: &AppState,
    community: CommunityId,
    channel_uuid: Uuid,
    rendered_text: &str,
    authored_text: &str,
) -> Result<Vec<(String, String)>, ActionSinkError> {
    let filter = match mention_candidate_query(rendered_text, authored_text) {
        MentionCandidateQuery::NoAnchor => return Ok(Vec::new()),
        MentionCandidateQuery::Names(filter) => Some(filter),
        MentionCandidateQuery::AllMembers => None,
    };
    let members = state
        .db
        .get_named_members_for_event_write(community, channel_uuid, filter.as_ref())
        .await
        .map_err(|e| ActionSinkError::Database(e.to_string()))?;
    Ok(members
        .into_iter()
        .filter_map(|(pubkey, name)| {
            Some((name, nostr::PublicKey::from_slice(&pubkey).ok()?.to_hex()))
        })
        .collect())
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
        authored_text: &str,
        author_pubkey: &str,
        reply_to: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ActionSinkError>> + Send + '_>> {
        let channel_id = channel_id.to_owned();
        let text = text.to_owned();
        let authored_text = authored_text.to_owned();
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
                .get_channel_for_event_write(tenant.community(), channel_uuid)
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
            if !crate::handlers::ingest::may_post(
                &state,
                tenant.community(),
                channel_uuid,
                &author_pubkey_bytes,
                &channel,
                is_member,
            )
            .await
            .map_err(|e| ActionSinkError::Database(e.to_string()))?
            {
                return Err(ActionSinkError::InvalidInput(
                    "workflow owner cannot post in destination channel".into(),
                ));
            }
            crate::handlers::ingest::check_agent_dm_write(
                tenant.community(),
                &state,
                channel_uuid,
                &author_pubkey_bytes,
                &channel,
            )
            .await
            .map_err(|e| {
                if e == crate::handlers::ingest::AGENT_DM_REFUSED {
                    ActionSinkError::InvalidInput(e)
                } else {
                    ActionSinkError::Database(e)
                }
            })?;

            // 3. Build kind:9 Nostr event
            //    - Signed by relay keypair (event.pubkey = relay pubkey)
            //    - first `p` tag attributes the message to the workflow owner;
            //      its `owner` marker keeps it from counting as a mention
            //    - `h` tag scopes to the channel (NIP-29, canonical UUID)
            //    - `buzz:workflow` tag prevents recursive workflow triggering
            //    - `buzz:workflow-owner` lets harnesses apply the owner's
            //      inbound-author policy after verifying the relay signature
            //    - one plain `p` tag for every resolved mention in the rendered
            //      output, the owner included, preserving legacy wake/feed behavior
            //    - one `buzz:workflow-mention` tag only when the same target was
            //      named in the workflow owner's stored step template. This is the
            //      authority-bearing provenance used by ACP; trigger-controlled
            //      template substitutions cannot create it.
            let mut tags = vec![
                Tag::parse(["p", &author_pubkey_hex, "", "owner"])
                    .map_err(|e| ActionSinkError::EventBuild(format!("p tag: {e}")))?,
                Tag::parse(["h", &channel_id_canonical])
                    .map_err(|e| ActionSinkError::EventBuild(format!("h tag: {e}")))?,
                Tag::parse(["buzz:workflow", "true"])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow tag: {e}")))?,
                Tag::parse(["buzz:workflow-owner", &author_pubkey_hex])
                    .map_err(|e| ActionSinkError::EventBuild(format!("workflow owner tag: {e}")))?,
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
                        &author_pubkey_bytes,
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

            // Resolve `@Name` mentions to channel-member pubkeys. The rendered
            // text supplies the legacy `p` tags used by subscriptions and feeds.
            // The stored author-written template independently supplies the
            // authority-bearing workflow-mention tags. A trigger may therefore
            // render an `@Name` into visible output, but it cannot borrow the
            // workflow owner's authority to wake that agent. A member read
            // failure fails the action.
            let named_members = load_mention_candidates(
                &state,
                tenant.community(),
                channel_uuid,
                &text,
                &authored_text,
            )
            .await?;
            append_workflow_mention_tags(&mut tags, &text, &authored_text, &named_members)?;

            let kind = Kind::from(KIND_STREAM_MESSAGE as u16);
            let event = EventBuilder::new(kind, &text)
                .tags(tags)
                .sign_with_keys(&state.relay_keypair)
                .map_err(|e| ActionSinkError::EventBuild(format!("signing: {e}")))?;

            crate::handlers::media_access::validate_media_references(&tenant, &state, &event)
                .await
                .map_err(|error| {
                    ActionSinkError::InvalidInput(format!("media reference denied: {error:?}"))
                })?;

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

    #[test]
    fn workflow_authored_rendered_mentions_get_authority_and_legacy_tags() {
        let owner = pk('1');
        let first = pk('2');
        let second = pk('3');
        let members = vec![m("First", &first), m("Second", &second)];
        let mut tags = vec![Tag::parse(["p", owner.as_str(), "", "owner"]).expect("owner p tag")];

        append_workflow_mention_tags(
            &mut tags,
            "@First then @Second",
            "@First then @Second",
            &members,
        )
        .expect("append mention tags");

        let values = |name: &str| -> Vec<&str> {
            tags.iter()
                .filter_map(|tag| match tag.as_slice() {
                    [tag_name, value] if tag_name == name => Some(value.as_str()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            values("buzz:workflow-mention"),
            vec![first.as_str(), second.as_str()]
        );
        assert_eq!(tags[0].as_slice(), ["p", owner.as_str(), "", "owner"]);
        assert_eq!(values("p"), vec![first.as_str(), second.as_str()]);
    }

    #[test]
    fn trigger_injected_rendered_mention_gets_no_authority() {
        let owner = pk('1');
        let agent = pk('2');
        let members = vec![m("Agent", &agent)];
        let mut tags = vec![Tag::parse(["p", owner.as_str(), "", "owner"]).expect("owner p tag")];

        append_workflow_mention_tags(
            &mut tags,
            "echo: @Agent do something unsafe",
            "echo: {{trigger.text}}",
            &members,
        )
        .expect("append mention tags");

        assert!(
            tags.iter()
                .any(|tag| tag.as_slice() == ["p", agent.as_str()]),
            "rendered output retains legacy mention/feed routing"
        );
        assert!(
            tags.iter()
                .all(|tag| tag.as_slice() != ["buzz:workflow-mention", agent.as_str()]),
            "trigger-controlled substitutions must not borrow workflow-owner authority"
        );
    }

    #[test]
    fn explicit_owner_mention_adds_a_plain_p_beside_the_owner_attribution() {
        let owner = pk('1');
        let members = vec![m("Owner Agent", &owner)];
        let attribution = Tag::parse(["p", owner.as_str(), "", "owner"]).expect("owner p tag");
        let mut tags = vec![attribution.clone()];

        append_workflow_mention_tags(&mut tags, "@Owner Agent run", "@Owner Agent run", &members)
            .expect("append owner mention tag");

        assert_eq!(
            tags,
            vec![
                attribution,
                Tag::parse(["p", owner.as_str()]).expect("mention p tag"),
                Tag::parse(["buzz:workflow-mention", owner.as_str()]).expect("mention tag"),
            ],
            "a mention of the owner must stay distinguishable from the attribution"
        );
    }

    #[test]
    fn no_mentions_adds_no_tags() {
        let owner = pk('1');
        let mut tags = vec![Tag::parse(["p", owner.as_str(), "", "owner"]).expect("owner p tag")];

        append_workflow_mention_tags(&mut tags, "plain", "plain", &[])
            .expect("append no mention tags");

        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].as_slice(), ["p", owner.as_str(), "", "owner"]);
    }

    /// Display names covering the resolver's rules: spaces and punctuation,
    /// `İ` and other expanding or context-sensitive folds, `ß`/`ẞ`, emoji,
    /// same-name members (ambiguous), names that prefix other names, and
    /// unrelated members. Each entry is a distinct person.
    pub(super) const MENTION_ROSTER: &[&str] = &[
        "Will",
        "Will Pfleger",
        "Lep (Subagent)",
        "İ",
        "İlker",
        "i\u{307}lker",
        "ilker",
        "ß",
        "ẞ Team",
        "Straße",
        "STRASSE",
        "🤖 Bot",
        "🤖",
        "Fizz",
        "Fizz",
        "fizz",
        "Robby",
        "Rob",
        "Robby Jr.",
        "Σοφία",
        "ΟΔΥΣΣΕΥΣ",
        "οδυσσευς",
        "Zoë",
        "Jos\u{65}\u{301}",
        "\u{212A}ate",
        "Kate",
        "Max",
        "Sam",
        " Spacey",
        "Alice",
        "Bob",
        "Ómar",
        "Øystein",
        "Member 1",
        "Member 7",
        "Member 12",
        "member 70",
    ];

    /// `(rendered, authored)` texts run against [`MENTION_ROSTER`].
    pub(super) const MENTION_CASES: &[(&str, &str)] = &[
        ("cc @Will Pfleger on this", "cc @Will Pfleger on this"),
        ("cc @Will on this", "cc @Will on this"),
        ("cc @Will", "cc @Will Pfleger"),
        ("@Will{{x}}", "@Will Pfleger"),
        ("@Lep (Subagent) take it", "@Lep (Subagent) take it"),
        ("@İ @Robby", "@İ @Robby"),
        ("@İlker hi", "@İlker hi"),
        ("@i\u{307}lker hi", "@ilker hi"),
        ("@ILKER and @ilker", "@ILKER"),
        ("@ß and @Max", "@ß and @Max"),
        ("@ẞ Team ship it", "@ß team ship it"),
        ("@STRASSE vs @straße", "@STRASSE vs @straße"),
        ("@🤖 Bot go", "@🤖 Bot go"),
        ("@🤖 go", "@🤖 go"),
        ("@Fizz status?", "@Fizz status?"),
        ("@fizz", "@fizz"),
        (
            "@Rob then @Robby then @Robby Jr. done",
            "@Rob then @Robby Jr. done",
        ),
        ("@σοφία", "@ΣΟΦΊΑ"),
        ("@ΟΔΥΣΣΕΥΣ", "@οδυσσευς"),
        ("@οδυσσευς", "@οδυσσευς"),
        ("welcome @Zoë!", "welcome @ZOË!"),
        ("hi @Jos\u{65}\u{301}!", "hi @Jos\u{65}\u{301}!"),
        ("@kate and @\u{212A}ATE", "@KATE"),
        ("(@Sam) and @ Spacey", "(@Sam) and @ Spacey"),
        ("@Member 7 and @Member 12", "@member 7"),
        ("@Max @Sam @Bob", "@Max"),
        ("alice@Robby", "@Robby"),
        ("@@Robby", "@Robby"),
        ("@", "@Robby"),
        ("no mention here", "@Robby"),
        ("＠Robby", "＠Robby"),
    ];

    pub(super) fn mention_roster(pubkey_for: impl Fn(usize) -> String) -> Vec<(String, String)> {
        MENTION_ROSTER
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), pubkey_for(i)))
            .collect()
    }

    pub(super) fn mention_tags(
        rendered: &str,
        authored: &str,
        members: &[(String, String)],
    ) -> Vec<Vec<String>> {
        let mut tags = Vec::new();
        append_workflow_mention_tags(&mut tags, rendered, authored, members)
            .expect("append mention tags");
        tags.into_iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect()
    }

    /// The SQL filter in `buzz_db::channel_members::get_named_members_for_event_write`,
    /// restated: ASCII names by ASCII-lowercased prefix, others by first char.
    fn admitted(filter: &buzz_db::channel_members::MentionNameFilter, name: &str) -> bool {
        if name.is_ascii() {
            let folded = name.to_ascii_lowercase();
            filter
                .lowercase_tails
                .iter()
                .any(|tail| tail.starts_with(&folded))
        } else {
            name.chars()
                .next()
                .is_some_and(|first| filter.first_chars.contains(&first.to_string()))
        }
    }

    /// Resolving against only the members the filter admits gives the same
    /// `p` and `buzz:workflow-mention` tags as resolving against the whole
    /// roster, for every case, while reading fewer members.
    #[test]
    fn candidate_filter_resolves_like_the_full_roster() {
        let mut roster = mention_roster(|i| format!("{:064x}", i + 1));
        // The same person listed twice is not ambiguous.
        let max = roster
            .iter()
            .find(|(name, _)| name == "Max")
            .cloned()
            .expect("Max in roster");
        roster.push(max);
        let mut resolved_any = 0;
        for &(rendered, authored) in MENTION_CASES {
            let expected = mention_tags(rendered, authored, &roster);
            resolved_any += usize::from(!expected.is_empty());
            let candidates: Vec<(String, String)> =
                match mention_candidate_query(rendered, authored) {
                    MentionCandidateQuery::NoAnchor => {
                        assert!(
                            expected.is_empty(),
                            "{rendered:?} has no anchor but resolves {expected:?}"
                        );
                        Vec::new()
                    }
                    MentionCandidateQuery::Names(filter) => {
                        let candidates: Vec<_> = roster
                            .iter()
                            .filter(|(name, _)| admitted(&filter, name))
                            .cloned()
                            .collect();
                        assert!(
                            candidates.len() < roster.len() / 2,
                            "{rendered:?} admits {} of {} members: {filter:?}",
                            candidates.len(),
                            roster.len()
                        );
                        candidates
                    }
                    MentionCandidateQuery::AllMembers => {
                        panic!("{rendered:?} is under the tail cap")
                    }
                };
            assert_eq!(
                mention_tags(rendered, authored, &candidates),
                expected,
                "{rendered:?} / {authored:?} resolves differently from candidates {candidates:?}"
            );
        }
        assert!(
            resolved_any >= 15,
            "the table must exercise real matches: {resolved_any}"
        );
    }

    /// `mention_candidate_query` finds an anchor exactly where the resolver
    /// may start a match, for every char that can precede the `@`.
    #[test]
    fn anchor_rule_matches_the_resolver() {
        let members = vec![m("Robby", &pk('a'))];
        for before in ('\0'..='\u{FFFF}').chain(['\u{1F916}', '\u{E0020}']) {
            let text = format!("{before}@Robby");
            let resolves = !resolve_mention_pubkeys(&text, &members).is_empty();
            let anchored = match mention_candidate_query(&text, "") {
                MentionCandidateQuery::Names(filter) => {
                    filter.lowercase_tails.iter().any(|tail| tail == "robby")
                }
                MentionCandidateQuery::NoAnchor => false,
                MentionCandidateQuery::AllMembers => unreachable!("one or two anchors"),
            };
            assert_eq!(resolves, anchored, "{before:?} before '@'");
        }
    }

    #[test]
    fn lowercase_first_preimage_covers_expanding_and_special_folds() {
        let preimage = |c: char| {
            let mut chars: Vec<char> = lowercase_first_preimage(c).collect();
            chars.sort_unstable();
            chars
        };
        assert_eq!(preimage('i'), vec!['I', 'i', 'İ']);
        assert_eq!(preimage('k'), vec!['K', 'k', '\u{212A}']);
        assert_eq!(preimage('ß'), vec!['ß', 'ẞ']);
        assert_eq!(preimage('σ'), vec!['Σ', 'σ']);
        assert_eq!(preimage('ς'), vec!['ς']);
        assert_eq!(preimage('🤖'), vec!['🤖']);
        assert!(preimage('\u{307}').contains(&'\u{307}'));
    }

    /// Tails are deduplicated; past [`MAX_MENTION_TAILS`] distinct ones the
    /// query falls back to the whole roster rather than drop an anchor.
    #[test]
    fn too_many_distinct_tails_read_every_member() {
        let names = |n: usize| (0..n).map(|i| format!("@name{i:02} ")).collect::<String>();
        let MentionCandidateQuery::Names(filter) =
            mention_candidate_query(&names(MAX_MENTION_TAILS), "")
        else {
            panic!("{MAX_MENTION_TAILS} tails fit");
        };
        assert_eq!(filter.lowercase_tails.len(), MAX_MENTION_TAILS);
        assert_eq!(
            mention_candidate_query(&names(MAX_MENTION_TAILS + 1), ""),
            MentionCandidateQuery::AllMembers
        );
        assert_eq!(
            mention_candidate_query(&names(MAX_MENTION_TAILS), &names(MAX_MENTION_TAILS + 1)),
            MentionCandidateQuery::AllMembers,
            "authored tails count toward the cap"
        );
        let repeated = "@Robby ".repeat(10 * MAX_MENTION_TAILS);
        let MentionCandidateQuery::Names(filter) = mention_candidate_query(&repeated, "") else {
            panic!("repeated mentions are one tail each");
        };
        assert!(filter.lowercase_tails.len() <= 2, "{filter:?}");
    }

    /// A tail is cut at the longest display name a `VARCHAR(255)` column can
    /// hold, counted in folded chars.
    #[test]
    fn tails_are_cut_at_the_display_name_limit() {
        let text = format!("@{}", "İ".repeat(300));
        let MentionCandidateQuery::Names(filter) = mention_candidate_query(&text, "") else {
            panic!("anchored");
        };
        assert_eq!(filter.lowercase_tails.len(), 1);
        assert_eq!(
            filter.lowercase_tails[0].chars().count(),
            MAX_DISPLAY_NAME_CHARS
        );
    }

    /// Rendered text with no left-boundary `@` resolves to no candidates
    /// without reading the database (unreachable here); an anchored mention
    /// still reads and fails.
    #[tokio::test]
    async fn mention_candidates_skip_the_database_without_an_anchor() {
        let state = crate::state::tests::test_state_with_unreachable_database_url(
            "postgres://buzz@127.0.0.1:1/buzz",
        )
        .await;
        let community = CommunityId::from_uuid(Uuid::nil());
        let channel = Uuid::new_v4();
        for rendered in ["build passed", "mail alice@Robby", "trailing @"] {
            let none = load_mention_candidates(&state, community, channel, rendered, "@Robby")
                .await
                .expect("no anchor must not touch the database");
            assert!(none.is_empty(), "{rendered:?}");
        }
        assert!(matches!(
            load_mention_candidates(&state, community, channel, "ping @Robby", "").await,
            Err(ActionSinkError::Database(_))
        ));
    }

    /// `load_mention_candidates` skips the member read when the rendered text
    /// has no `@`. That is only sound while no mention syntax resolves
    /// without one: if this fails because a new form (say `nostr:npub…`)
    /// now produces tags, update `mention_candidate_query` in the same change.
    #[test]
    fn text_without_at_never_resolves_a_mention() {
        use nostr::nips::nip19::ToBech32;

        let robby = pk('2');
        let npub = nostr::PublicKey::from_hex(&robby)
            .expect("member pubkey")
            .to_bech32()
            .expect("npub");
        let members = vec![("Robby".to_string(), robby.clone())];
        for text in [
            "Robby".to_string(),
            "hi Robby, build passed".to_string(),
            "＠Robby".to_string(),
            "&#64;Robby".to_string(),
            format!("nostr:{npub}"),
            robby.clone(),
        ] {
            assert!(!text.contains('@'), "case must not contain '@': {text}");
            assert_eq!(
                mention_candidate_query(&text, &text),
                MentionCandidateQuery::NoAnchor
            );
            let mut tags = Vec::new();
            append_workflow_mention_tags(&mut tags, &text, &text, &members)
                .expect("append mention tags");
            assert!(tags.is_empty(), "{text:?} produced {tags:?}");
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    //! Regression test for `e3661764` / `7899c1a8`: a workflow `send_message`
    //! that mentions a channel member by name (`@Name`) in its author-written
    //! step template must emit both the legacy `p` tag and authenticated
    //! workflow-mention provenance for that member. Rendered trigger data may
    //! still create a legacy `p` tag, but never authority-bearing provenance.
    //!
    //! Postgres-gated like the other DB-backed relay tests. Run with:
    //!   `cargo test -p buzz-relay --lib workflow_sink -- --ignored`
    use super::*;
    use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
    use buzz_db::CreateCommunityWithOwnerResult;
    use std::sync::Arc;

    /// Real-PG state mirroring `handlers::event::tests::test_state_with_redis_url`.
    async fn test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).expect("redis pool");
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

    async fn execute_send_message_workflow(
        state: &Arc<AppState>,
        community: CommunityId,
        channel_id: Uuid,
        owner_pubkey: &[u8],
        name: &str,
        authored_text: &str,
        trigger_text: &str,
    ) -> String {
        let definition = serde_json::json!({
            "name": name,
            "trigger": {"on": "message_posted"},
            "steps": [{
                "id": "send",
                "action": "send_message",
                "text": authored_text,
            }],
            "enabled": true,
        });
        let definition_hash_byte = name.as_bytes().first().copied().unwrap_or_default();
        let workflow_id = state
            .db
            .create_workflow(
                community,
                Some(channel_id),
                owner_pubkey,
                name,
                &definition.to_string(),
                &[definition_hash_byte; 32],
            )
            .await
            .expect("create workflow");
        let trigger_ctx = buzz_workflow::executor::TriggerContext {
            text: trigger_text.to_owned(),
            channel_id: channel_id.to_string(),
            ..Default::default()
        };
        let trigger_ctx_json = serde_json::to_value(&trigger_ctx).expect("serialize trigger");
        let run_id = state
            .db
            .create_workflow_run(community, workflow_id, None, Some(&trigger_ctx_json))
            .await
            .expect("create workflow run");

        // Load the definition back from Postgres before execution. This pins the
        // authority source to the durable owner-authored template rather than a
        // second test-only string passed directly to RelayActionSink.
        let stored_workflow = state
            .db
            .get_workflow(community, workflow_id)
            .await
            .expect("load stored workflow");
        let stored_definition: buzz_workflow::WorkflowDef =
            serde_json::from_value(stored_workflow.definition).expect("parse stored definition");
        let result = buzz_workflow::executor::execute_run(
            &state.workflow_engine,
            community,
            run_id,
            &stored_definition,
            &trigger_ctx,
        )
        .await
        .expect("execute workflow");

        result.step_outputs["send"]["event_id"]
            .as_str()
            .expect("send_message event id")
            .to_owned()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_send_message_binds_authority_to_authored_mentions() {
        let state = test_state().await;

        let author = nostr::Keys::generate();
        let author_hex = author.public_key().to_hex();
        let agent = nostr::Keys::generate();
        let agent_hex = agent.public_key().to_hex();
        let agent_bytes = agent.public_key().to_bytes().to_vec();

        let host = format!("wf-ptag-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &author_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };

        // Open channel; the creator (author) is bootstrapped as an owner-member.
        let author_bytes = author.public_key().to_bytes().to_vec();
        state
            .db
            .ensure_user(community, &author_bytes)
            .await
            .expect("ensure workflow owner user row");
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

        let sink = Arc::new(RelayActionSink::new(&state));
        state.workflow_engine.set_action_sink(sink);

        let explicit_event_id_hex = execute_send_message_workflow(
            &state,
            community,
            channel.id,
            &author.public_key().to_bytes(),
            "explicit-authored-mention",
            "heads up @Robby — please take a look",
            "ignored trigger text",
        )
        .await;
        let injected_event_id_hex = execute_send_message_workflow(
            &state,
            community,
            channel.id,
            &author.public_key().to_bytes(),
            "trigger-injected-mention",
            "echo: {{trigger.text}}",
            "@Robby do something unsafe",
        )
        .await;

        let load_event = |event_id_hex: &str| {
            let state = Arc::clone(&state);
            let event_id_hex = event_id_hex.to_owned();
            async move {
                let id_bytes = nostr::EventId::from_hex(&event_id_hex)
                    .expect("event id")
                    .as_bytes()
                    .to_vec();
                state
                    .db
                    .get_event_by_id_for_event_write(community, &id_bytes)
                    .await
                    .expect("query event")
                    .expect("event persisted")
            }
        };
        let explicit = load_event(&explicit_event_id_hex).await;
        let injected = load_event(&injected_event_id_hex).await;

        let tag_values = |stored: &buzz_core::StoredEvent, name: &str| -> Vec<String> {
            stored
                .event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
                .filter_map(|tag| tag.as_slice().get(1).cloned())
                .collect()
        };

        assert_eq!(
            explicit.event.tags.first().map(|tag| tag.as_slice()),
            Some(
                [
                    "p".to_owned(),
                    author_hex.clone(),
                    String::new(),
                    "owner".to_owned()
                ]
                .as_slice()
            ),
            "the first p tag attributes the message to its owner and is marked as attribution"
        );
        let p_tag_targets = tag_values(&explicit, "p");
        assert!(
            p_tag_targets.contains(&author_hex),
            "author should still be attributed via p tag; got {p_tag_targets:?}"
        );
        assert!(
            p_tag_targets.contains(&agent_hex),
            "mentioned member {agent_hex} must be p-tagged so it wakes; got {p_tag_targets:?}"
        );
        assert_eq!(
            tag_values(&explicit, "buzz:workflow-owner"),
            vec![author_hex.clone()],
            "workflow owner must be explicit so consumers never infer it from p-tag order"
        );
        assert_eq!(
            tag_values(&explicit, "buzz:workflow-mention"),
            vec![agent_hex.clone()],
            "relay-authenticated workflow mention must identify the explicitly named member"
        );

        let injected_p_tags = tag_values(&injected, "p");
        assert!(
            injected_p_tags.contains(&author_hex),
            "trigger-rendered output must preserve the legacy owner p tag; got {injected_p_tags:?}"
        );
        assert!(
            injected_p_tags.contains(&agent_hex),
            "trigger-rendered mention must preserve legacy mention/feed routing; got {injected_p_tags:?}"
        );
        assert!(
            tag_values(&injected, "buzz:workflow-mention").is_empty(),
            "a mention introduced solely by trigger data must not receive owner-delegated authority"
        );
    }

    /// In a large channel the sink reads only the members a mention could
    /// name, and every mention case resolves to the same tags as against the
    /// full roster. Before this change the read returned every named member.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mention_candidates_read_only_names_a_mention_can_match() {
        use super::tests::{mention_roster, mention_tags, MENTION_CASES};

        let state = test_state().await;
        let owner = nostr::Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let host = format!("wf-candidates-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &owner_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };
        let channel = state
            .db
            .create_channel(
                community,
                "wf-candidates",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &owner.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel");

        let noise = 500;
        let mut full_roster: Vec<(String, String)> = (1..=noise)
            .map(|n| {
                (
                    format!("Member {n}"),
                    nostr::Keys::generate().public_key().to_hex(),
                )
            })
            .collect();
        full_roster.extend(mention_roster(|_| {
            nostr::Keys::generate().public_key().to_hex()
        }));
        let pubkeys: Vec<Vec<u8>> = full_roster
            .iter()
            .map(|(_, hex)| hex::decode(hex).expect("pubkey hex"))
            .collect();
        let names: Vec<&str> = full_roster.iter().map(|(name, _)| name.as_str()).collect();
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, display_name) \
             SELECT $1, * FROM unnest($2::bytea[], $3::text[])",
        )
        .bind(community.as_uuid())
        .bind(&pubkeys)
        .bind(&names)
        .execute(state.db.pool())
        .await
        .expect("insert users");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey) \
             SELECT $1, $2, * FROM unnest($3::bytea[])",
        )
        .bind(community.as_uuid())
        .bind(channel.id)
        .bind(&pubkeys)
        .execute(state.db.pool())
        .await
        .expect("insert members");

        let robby = load_mention_candidates(&state, community, channel.id, "ping @Robby", "")
            .await
            .expect("load candidates");
        let mut robby_names: Vec<&str> = robby.iter().map(|(name, _)| name.as_str()).collect();
        robby_names.sort_unstable();
        assert_eq!(
            robby_names,
            vec!["Rob", "Robby"],
            "only names that prefix the mention, out of {} members",
            full_roster.len()
        );

        for &(rendered, authored) in MENTION_CASES {
            let candidates =
                load_mention_candidates(&state, community, channel.id, rendered, authored)
                    .await
                    .expect("load candidates");
            assert!(
                candidates.len() <= 20,
                "{rendered:?} read {} members",
                candidates.len()
            );
            assert_eq!(
                mention_tags(rendered, authored, &candidates),
                mention_tags(rendered, authored, &full_roster),
                "{rendered:?} / {authored:?} resolves differently from candidates {candidates:?}"
            );
        }
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
            .create_community_with_owner(&host, &author_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
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
            .get_event_by_id_for_event_write(community, &reply_id_bytes)
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
            .create_community_with_owner(&host, &author_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
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
            .get_event_by_id_for_event_write(community, &reply_id_bytes)
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
    async fn workflow_send_message_keeps_agents_out_of_dms() {
        let state = test_state().await;
        let (owner, agent, stranger) = (
            nostr::Keys::generate(),
            nostr::Keys::generate(),
            nostr::Keys::generate(),
        );
        let host = format!("wf-agent-dm-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(
                &host,
                &owner.public_key().to_hex(),
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };
        for keys in [&owner, &agent, &stranger] {
            state
                .db
                .ensure_user(community, &keys.public_key().to_bytes())
                .await
                .expect("ensure user");
        }
        state
            .db
            .set_agent_owner_for_authorization(
                community,
                &agent.public_key().to_bytes(),
                &owner.public_key().to_bytes(),
            )
            .await
            .expect("link agent owner");
        let open_dm = |members: [&nostr::Keys; 2]| {
            let state = state.clone();
            let keys: Vec<[u8; 32]> = members.iter().map(|k| k.public_key().to_bytes()).collect();
            async move {
                let refs: Vec<&[u8]> = keys.iter().map(|k| k.as_slice()).collect();
                state
                    .db
                    .open_dm(community, &refs, refs[0])
                    .await
                    .expect("open dm")
                    .0
                    .id
            }
        };
        let stranger_dm = open_dm([&agent, &stranger]).await;
        let owner_dm = open_dm([&agent, &owner]).await;
        let sink = RelayActionSink::new(&state);
        let agent_hex = agent.public_key().to_hex();

        let err = sink
            .send_message(
                community,
                &stranger_dm.to_string(),
                "hi",
                "hi",
                &agent_hex,
                None,
            )
            .await
            .expect_err("an agent's workflow must not post in a DM with a stranger");
        assert!(
            matches!(&err, ActionSinkError::InvalidInput(m) if m == crate::handlers::ingest::AGENT_DM_REFUSED),
            "{err:?}"
        );
        sink.send_message(
            community,
            &owner_dm.to_string(),
            "hi",
            "hi",
            &agent_hex,
            None,
        )
        .await
        .expect("an agent's workflow may post in its owner-only DM");
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
            .create_community_with_owner(&host, &author_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
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

    /// A workflow reply to a parent the owner cannot use, in another channel
    /// or unreadable to them, fails exactly like a reply to a missing parent.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn workflow_reply_to_unusable_parent_matches_missing() {
        let state = test_state().await;
        let owner = nostr::Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let stranger = nostr::Keys::generate();
        let host = format!("wf-unusable-{}.example", uuid::Uuid::new_v4().simple());
        let community = match state
            .db
            .create_community_with_owner(&host, &owner_hex, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("create community")
        {
            CreateCommunityWithOwnerResult::Created(rec) => rec.id,
            other => panic!("expected fresh community, got {other:?}"),
        };
        let open_channel = |name: &'static str| {
            let state = Arc::clone(&state);
            let owner = owner.public_key().to_bytes();
            async move {
                state
                    .db
                    .create_channel(
                        community,
                        name,
                        ChannelType::Stream,
                        ChannelVisibility::Open,
                        None,
                        &owner,
                        None,
                    )
                    .await
                    .expect("create channel")
            }
        };
        let channel = open_channel("wf-unusable").await;
        let elsewhere = open_channel("wf-elsewhere").await;
        let insert = |keys: &nostr::Keys, kind: u32, channel_id: Uuid, extra: Vec<Tag>| {
            let state = Arc::clone(&state);
            let mut tags = vec![Tag::parse(["h", &channel_id.to_string()]).expect("h tag")];
            tags.extend(extra);
            let event = EventBuilder::new(Kind::from(kind as u16), "parent")
                .tags(tags)
                .sign_with_keys(keys)
                .expect("sign parent");
            async move {
                state
                    .db
                    .insert_event(community, &event, Some(channel_id))
                    .await
                    .expect("insert parent");
                event.id.to_hex()
            }
        };
        // A kind 5 whose `k` tag names an author-only kind is readable only by
        // its author. Production ingest never stores one channel-scoped (its
        // target is global), so this is the closest state that isolates the
        // resolver's visibility check and the reader identity it is given.
        let author_only_deletion = |keys: &nostr::Keys| {
            let target = nostr::Keys::generate().public_key().to_hex();
            insert(
                keys,
                buzz_core::kind::KIND_DELETION,
                channel.id,
                vec![
                    Tag::parse(["e", &target]).expect("e tag"),
                    Tag::parse(["k", "30300"]).expect("k tag"),
                ],
            )
        };
        let other_channel = insert(&owner, KIND_STREAM_MESSAGE, elsewhere.id, vec![]).await;
        let unreadable = author_only_deletion(&stranger).await;
        let owner_only = author_only_deletion(&owner).await;

        let sink = RelayActionSink::new(&state);
        let reply = |author_hex: String, parent: String| {
            let sink = &sink;
            let channel = channel.id.to_string();
            async move {
                sink.send_message(
                    community,
                    &channel,
                    "reply",
                    "reply",
                    &author_hex,
                    Some(&parent),
                )
                .await
            }
        };
        let missing = nostr::Keys::generate().public_key().to_hex();
        let other_owner_hex = nostr::Keys::generate().public_key().to_hex();
        for (case, author, parent) in [
            ("missing", &owner_hex, missing),
            ("other channel", &owner_hex, other_channel),
            ("unreadable", &owner_hex, unreadable),
            (
                "readable only by another owner",
                &other_owner_hex,
                owner_only.clone(),
            ),
        ] {
            match reply(author.clone(), parent).await {
                Err(ActionSinkError::InvalidInput(msg)) => {
                    assert_eq!(msg, "reply parent not found", "{case}")
                }
                other => panic!("{case}: expected the missing-parent error, got {other:?}"),
            }
        }

        // Positive control: the resolver reads as the workflow owner, who
        // alone can see their author-only parent.
        reply(owner_hex.clone(), owner_only)
            .await
            .expect("owner replies to a parent only they can read");
    }
}
