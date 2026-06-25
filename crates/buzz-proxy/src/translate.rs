//! Event translation between Buzz internal format and NIP-28 standard format.
//!
//! # Overview
//!
//! The proxy sits between standard Nostr clients (NIP-28) and the Buzz relay.
//! Buzz stores messages as kind:9 events with `#h <uuid>` channel tags.
//! NIP-28 clients expect kind:42 events with `#e <kind40_event_id>` channel tags.
//!
//! This module handles bidirectional translation:
//!
//! - **Outbound** (Buzz → client): kind:9 + `#h(uuid)` → kind:42 + `#e(event_id)`
//! - **Inbound** (client → Buzz): kind:42 + `#e(event_id)` → kind:9 + `#h(uuid)`
//!
//! All translated events are re-signed with deterministic shadow keys so that
//! each external user maps to a consistent shadow pubkey across sessions.

use std::sync::Arc;
use std::time::Duration;

use buzz_core::kind::{
    event_kind_u32, KIND_DELETION, KIND_REACTION, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_EDIT,
    KIND_STREAM_MESSAGE_V2,
};
use moka::sync::Cache;
use nostr::prelude::*;
use uuid::Uuid;

use crate::channel_map::{ChannelInfo, ChannelMap};
use crate::kind_translator::KindTranslator;
use crate::shadow_keys::ShadowKeyManager;
use crate::ProxyError;

/// Translates events and filters between Buzz internal format and NIP-28
/// standard format.
///
/// All translated events are re-signed with shadow keys to preserve per-user
/// identity while hiding internal key material from external clients.
pub struct Translator {
    kind_translator: KindTranslator,
    shadow_keys: Arc<ShadowKeyManager>,
    channel_map: Arc<ChannelMap>,
    http_client: reqwest::Client,
    api_base: String,
    api_token: String,
    /// Hex-encoded public key of the relay. Only events signed by this key
    /// may carry trusted `actor`/`p` attribution tags.
    relay_pubkey: String,
    internal_to_external_event_ids: Cache<String, String>,
    external_to_internal_event_ids: Cache<String, String>,
    internal_event_channels: Cache<String, Uuid>,
}

impl Translator {
    /// Create a new [`Translator`].
    pub fn new(
        shadow_keys: Arc<ShadowKeyManager>,
        channel_map: Arc<ChannelMap>,
        api_base: impl Into<String>,
        api_token: impl Into<String>,
        relay_pubkey: impl Into<String>,
    ) -> Self {
        let cache_ttl = Duration::from_secs(3600);
        Self {
            kind_translator: KindTranslator::new(),
            shadow_keys,
            channel_map,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("HTTP client build must succeed"),
            api_base: api_base.into(),
            api_token: api_token.into(),
            relay_pubkey: relay_pubkey.into(),
            internal_to_external_event_ids: Cache::builder()
                .max_capacity(100_000)
                .time_to_idle(cache_ttl)
                .build(),
            external_to_internal_event_ids: Cache::builder()
                .max_capacity(100_000)
                .time_to_idle(cache_ttl)
                .build(),
            internal_event_channels: Cache::builder()
                .max_capacity(100_000)
                .time_to_idle(cache_ttl)
                .build(),
        }
    }
}

impl Translator {
    fn cache_event_mapping(&self, internal_event_id: &str, external_event_id: &str) {
        self.internal_to_external_event_ids
            .insert(internal_event_id.to_string(), external_event_id.to_string());
        self.external_to_internal_event_ids
            .insert(external_event_id.to_string(), internal_event_id.to_string());
    }

    fn lookup_external_event_id(&self, internal_event_id: &str) -> Option<String> {
        self.internal_to_external_event_ids.get(internal_event_id)
    }

    fn lookup_internal_event_id(&self, external_event_id: &str) -> Option<String> {
        self.external_to_internal_event_ids.get(external_event_id)
    }

    fn cache_internal_channel(&self, internal_event_id: &str, channel_uuid: Uuid) {
        self.internal_event_channels
            .insert(internal_event_id.to_string(), channel_uuid);
    }

    fn lookup_internal_channel(&self, internal_event_id: &str) -> Option<Uuid> {
        self.internal_event_channels.get(internal_event_id)
    }

    fn resolve_shadow_author_hex(&self, event: &Event) -> String {
        // Only trust actor/p tags on relay-signed events (API-originated).
        // User-signed events could carry spoofed actor/p tags — if the event
        // pubkey doesn't match the relay's keypair, fall back to the event
        // pubkey directly.
        if event.pubkey.to_hex() == self.relay_pubkey {
            for tag_name in &["actor", "p"] {
                for tag in event.tags.iter() {
                    let slice = tag.as_slice();
                    if slice.len() >= 2 && slice[0] == *tag_name {
                        let hex = &slice[1];
                        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                            // Normalize to lowercase for consistent shadow-key derivation.
                            return hex.to_lowercase();
                        }
                    }
                }
            }
        }
        event.pubkey.to_hex()
    }

    fn resolve_channel_info(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
    ) -> Result<Option<(String, ChannelInfo)>, ProxyError> {
        let Some((uuid_str, channel_info)) = event
            .tags
            .iter()
            .filter(|tag| {
                let slice = tag.as_slice();
                slice.len() >= 2 && slice[0] == "h"
            })
            .find_map(|tag| {
                let uuid_str = tag.as_slice().get(1)?.clone();
                let uuid: Uuid = uuid_str.parse().ok()?;
                let info = self.channel_map.lookup_by_uuid(&uuid)?;
                Some((uuid_str, info))
            })
        else {
            return Ok(None);
        };

        if !allowed_channels.contains(&channel_info.uuid) {
            return Err(ProxyError::PermissionDenied(format!(
                "channel {} not in invite scope",
                channel_info.uuid
            )));
        }

        Ok(Some((uuid_str, channel_info)))
    }

    async fn fetch_internal_event(&self, event_id: &str) -> Result<Option<Event>, ProxyError> {
        let response = self
            .http_client
            .get(format!("{}/api/events/{}", self.api_base, event_id))
            .header("Authorization", format!("Bearer {}", self.api_token))
            .send()
            .await
            .map_err(|e| ProxyError::Upstream(format!("event fetch failed: {e}")))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = response
            .error_for_status()
            .map_err(|e| ProxyError::Upstream(format!("event fetch failed: {e}")))?;
        let body = response
            .text()
            .await
            .map_err(|e| ProxyError::Upstream(format!("event fetch body failed: {e}")))?;

        serde_json::from_str::<Event>(&body)
            .map(Some)
            .map_err(|e| ProxyError::Upstream(format!("event decode failed: {e}")))
    }

    async fn ensure_external_event_id(
        &self,
        internal_event_id: &str,
        allowed_channels: &[Uuid],
    ) -> Result<Option<String>, ProxyError> {
        if let Some(mapped) = self.lookup_external_event_id(internal_event_id) {
            return Ok(Some(mapped));
        }

        let Some(event) = self.fetch_internal_event(internal_event_id).await? else {
            return Ok(None);
        };

        let translated = self.translate_outbound_message_like(&event, allowed_channels)?;
        Ok(translated.map(|translated| translated.id.to_hex()))
    }

    fn translate_outbound_message_like(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
    ) -> Result<Option<Event>, ProxyError> {
        let kind_u32 = event_kind_u32(event);
        let is_stream_message =
            kind_u32 == KIND_STREAM_MESSAGE || kind_u32 == KIND_STREAM_MESSAGE_V2;
        let is_edit = kind_u32 == KIND_STREAM_MESSAGE_EDIT;

        if !is_stream_message && !is_edit {
            return Ok(None);
        }

        let Some((uuid_str, channel_info)) = self.resolve_channel_info(event, allowed_channels)?
        else {
            return Ok(None);
        };

        let mut new_tags: Vec<Tag> = Vec::new();
        new_tags
            .push(Tag::parse(["e", &channel_info.kind40_event_id]).expect("e tag is always valid"));
        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            if slice.first().map(|value| value.as_str()) == Some("h")
                && slice.get(1).map(|value| value.as_str()) == Some(uuid_str.as_str())
            {
                continue;
            }
            if slice.first().map(|value| value.as_str()) == Some("actor") {
                continue;
            }
            new_tags.push(tag.clone());
        }

        let standard_kind = self.kind_translator.to_standard(kind_u32);
        let content = if kind_u32 == KIND_STREAM_MESSAGE_V2 {
            extract_plain_text(&event.content)
        } else {
            event.content.clone()
        };

        let shadow_keys = self
            .shadow_keys
            .get_or_create(&self.resolve_shadow_author_hex(event))?;

        let translated = EventBuilder::new(
            Kind::Custom(u16::try_from(standard_kind).expect("standard kind must fit in u16")),
            content,
        )
        .tags(new_tags)
        .custom_created_at(event.created_at)
        .sign_with_keys(&shadow_keys)
        .map_err(|e| ProxyError::Upstream(format!("outbound signing failed: {e}")))?;

        self.cache_event_mapping(&event.id.to_hex(), &translated.id.to_hex());
        self.cache_internal_channel(&event.id.to_hex(), channel_info.uuid);
        Ok(Some(translated))
    }

    async fn translate_outbound_reaction(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
    ) -> Result<Option<Event>, ProxyError> {
        self.translate_outbound_tag_targeted(event, allowed_channels, Kind::Reaction)
            .await
    }

    async fn translate_outbound_deletion(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
    ) -> Result<Option<Event>, ProxyError> {
        self.translate_outbound_tag_targeted(event, allowed_channels, Kind::EventDeletion)
            .await
    }

    /// Shared outbound translation for tag-targeted events (reactions, deletions).
    ///
    /// Resolves the channel via `#h`, translates all internal `#e` event IDs to
    /// their external counterparts, strips `#h` and `actor` tags, and re-signs
    /// with the shadow key for the original author.
    async fn translate_outbound_tag_targeted(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
        kind: Kind,
    ) -> Result<Option<Event>, ProxyError> {
        let Some((_uuid_str, channel_info)) = self.resolve_channel_info(event, allowed_channels)?
        else {
            return Ok(None);
        };

        let mut new_tags: Vec<Tag> = Vec::new();
        let mut saw_target = false;

        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            if slice.first().map(|value| value.as_str()) == Some("h") {
                continue;
            }
            if slice.first().map(|value| value.as_str()) == Some("actor") {
                continue;
            }
            if slice.len() >= 2
                && slice[0] == "e"
                && slice[1].len() == 64
                && slice[1].chars().all(|ch| ch.is_ascii_hexdigit())
            {
                let Some(external_target_id) = self
                    .ensure_external_event_id(&slice[1], allowed_channels)
                    .await?
                else {
                    return Ok(None);
                };

                let mut parts = vec!["e".to_string(), external_target_id];
                parts.extend(slice.iter().skip(2).cloned());
                new_tags.push(Tag::parse(parts).map_err(|e| {
                    ProxyError::Upstream(format!("outbound tag build failed: {e}"))
                })?);
                saw_target = true;
                continue;
            }
            new_tags.push(tag.clone());
        }

        if !saw_target {
            return Ok(None);
        }

        let shadow_keys = self
            .shadow_keys
            .get_or_create(&self.resolve_shadow_author_hex(event))?;

        let translated = EventBuilder::new(kind, event.content.clone())
            .tags(new_tags)
            .custom_created_at(event.created_at)
            .sign_with_keys(&shadow_keys)
            .map_err(|e| ProxyError::Upstream(format!("outbound signing failed: {e}")))?;

        self.cache_event_mapping(&event.id.to_hex(), &translated.id.to_hex());
        self.cache_internal_channel(&event.id.to_hex(), channel_info.uuid);
        Ok(Some(translated))
    }

    /// Translate a Buzz event to NIP-28 format for delivery to external clients.
    ///
    /// Returns `Ok(Some(event))` on success, `Ok(None)` if the event should be
    /// silently dropped (unknown kind, no channel tag, etc.), or an error if the
    /// event references a channel the client is not allowed to see.
    ///
    /// # Translation rules
    ///
    /// - kind:9 / kind:40002 → kind:42
    /// - `#h <uuid>` tag → `#e <kind40_event_id>` tag
    /// - All other tags are preserved unchanged
    /// - Re-signed with the shadow key for the original author's pubkey
    /// - V2 rich content (JSON with `"text"` field) is unwrapped to plain text
    pub async fn translate_outbound(
        &self,
        event: &Event,
        allowed_channels: &[Uuid],
    ) -> Result<Option<Event>, ProxyError> {
        let kind_u32 = event_kind_u32(event);
        if kind_u32 == KIND_REACTION {
            return self
                .translate_outbound_reaction(event, allowed_channels)
                .await;
        }
        if kind_u32 == KIND_DELETION {
            return self
                .translate_outbound_deletion(event, allowed_channels)
                .await;
        }

        self.translate_outbound_message_like(event, allowed_channels)
    }
}

impl Translator {
    /// Translate a NIP-28 event from an external client into Buzz format.
    ///
    /// # Translation rules
    ///
    /// - kind:42 (or kind:1) → kind:9
    /// - `#e <kind40_event_id>` tag → `#h <uuid>` tag
    /// - All other tags are preserved unchanged
    /// - Re-signed with the shadow key for `external_pubkey`
    ///
    /// Returns an error if the event kind is not accepted, the `#e` tag is
    /// missing or references an unknown channel, or the channel is not in the
    /// client's invite scope.
    pub fn translate_inbound(
        &self,
        event: &Event,
        external_pubkey: &str,
        allowed_channels: &[Uuid],
    ) -> Result<Event, ProxyError> {
        let kind_u32 = event.kind.as_u16() as u32;

        // Accept kind:42 (channel message), kind:41 (channel metadata edit),
        // kind:1 (text note), and kind:7 (reaction).
        // kind:5 (deletion) is intentionally NOT accepted inbound.
        // Inbound kind:5 blocked: relay deletion handler lacks author-match authorization.
        // External clients could delete any user's messages. Re-enable after adding
        // author validation to handle_standard_deletion_event.
        // Everything else is rejected.
        if kind_u32 != 42 && kind_u32 != 41 && kind_u32 != 1 && kind_u32 != 7 {
            return Err(ProxyError::PermissionDenied(format!(
                "kind {} not accepted by proxy (expected 1, 7, 41, or 42)",
                kind_u32
            )));
        }

        if kind_u32 == 7 {
            return self.translate_inbound_tag_targeted(
                event,
                external_pubkey,
                allowed_channels,
                Kind::Reaction,
                "reaction",
            );
        }

        // Find the channel-reference `#e` tag by looking up each `#e` value
        // against the channel map. This is more robust than assuming the first
        // `#e` tag is always the channel reference — reply/thread `#e` tags may
        // appear in any order depending on the client.
        let (event_id_str, channel_info) = event
            .tags
            .iter()
            .filter(|t| {
                let s = t.as_slice();
                s.len() >= 2 && s[0] == "e"
            })
            .find_map(|t| {
                let eid = t.as_slice().get(1)?;
                let info = self.channel_map.lookup_by_event_id(eid)?;
                Some((eid.clone(), info))
            })
            .ok_or_else(|| {
                ProxyError::PermissionDenied(
                    "kind:42 event must have an #e tag referencing a known channel".into(),
                )
            })?;

        // Enforce channel-level access control.
        if !allowed_channels.contains(&channel_info.uuid) {
            return Err(ProxyError::PermissionDenied(format!(
                "channel {} not in invite scope",
                channel_info.uuid
            )));
        }

        // Build translated tag list: replace the channel `#e` with `#h`, keep everything else.
        let mut new_tags: Vec<Tag> = Vec::new();
        // SAFETY: ["h", <uuid_string>] is always a valid 2-element tag structure
        new_tags.push(
            Tag::parse(["h", &channel_info.uuid.to_string()])
                .expect("SAFETY: [\"h\", uuid_string] is always a valid tag structure"),
        );
        for tag in event.tags.iter() {
            let s = tag.as_slice();
            // Only strip the specific `#e` tag whose value matches the channel event ID.
            // Preserve other `#e` tags (e.g. NIP-10 reply threading).
            if s.first().map(|v| v.as_str()) == Some("e")
                && s.get(1).map(|v| v.as_str()) == Some(event_id_str.as_str())
            {
                continue;
            }
            // Strip ALL `#h` tags — the authorized #h is already added above.
            // Prevents clients from injecting unauthorized channel associations.
            if s.first().map(|v| v.as_str()) == Some("h") {
                continue;
            }
            new_tags.push(tag.clone());
        }

        // Translate kind number.
        let buzz_kind = self.kind_translator.to_buzz(kind_u32);

        // Re-sign with the shadow key for the external user.
        let shadow_keys = self.shadow_keys.get_or_create(external_pubkey)?;

        // SAFETY: buzz_kind is derived from KindTranslator which maps to known Buzz kinds (9, 40002, 40003) that fit in u16
        let translated = EventBuilder::new(
            Kind::Custom(
                u16::try_from(buzz_kind)
                    .expect("SAFETY: buzz kind values (9, 40002, 40003) always fit in u16"),
            ),
            &event.content,
        )
        .tags(new_tags)
        .custom_created_at(event.created_at)
        .sign_with_keys(&shadow_keys)
        .map_err(|e| ProxyError::Upstream(format!("inbound signing failed: {e}")))?;

        self.cache_event_mapping(&translated.id.to_hex(), &event.id.to_hex());
        self.cache_internal_channel(&translated.id.to_hex(), channel_info.uuid);
        Ok(translated)
    }

    /// Shared inbound translation for tag-targeted events (kind:5 deletion, kind:7 reaction).
    ///
    /// # Security
    ///
    /// Collects ALL hex `#e` tags and verifies every one resolves to the **same**
    /// channel UUID that is in `allowed_channels`. A mixed-target event (targets
    /// spanning multiple channels) is rejected outright — this prevents a client
    /// from smuggling out-of-scope event IDs inside a single deletion or reaction.
    fn translate_inbound_tag_targeted(
        &self,
        event: &Event,
        external_pubkey: &str,
        allowed_channels: &[Uuid],
        kind: Kind,
        label: &str,
    ) -> Result<Event, ProxyError> {
        // Collect all hex #e tag values.
        let e_tag_values: Vec<String> = event
            .tags
            .iter()
            .filter_map(|tag| {
                let slice = tag.as_slice();
                if slice.len() >= 2
                    && slice[0] == "e"
                    && slice[1].len() == 64
                    && slice[1].chars().all(|ch| ch.is_ascii_hexdigit())
                {
                    Some(slice[1].clone())
                } else {
                    None
                }
            })
            .collect();

        if e_tag_values.is_empty() {
            return Err(ProxyError::PermissionDenied(format!(
                "kind:{} event must reference a target message via #e",
                event.kind.as_u16()
            )));
        }

        // Resolve every #e tag to an internal event ID and channel UUID.
        // All targets must resolve to the same channel — cross-channel targeting
        // is rejected to prevent authorization bypass.
        let mut resolved_channel: Option<Uuid> = None;
        // Resolve all external → internal ID mappings upfront. We store
        // the resolved pairs so the tag-building loop below never re-reads
        // the moka cache (which could evict entries between passes).
        let mut resolved_ids: Vec<(String, String)> = Vec::with_capacity(e_tag_values.len());
        for external_id in &e_tag_values {
            let internal_id = self.lookup_internal_event_id(external_id).ok_or_else(|| {
                ProxyError::PermissionDenied(format!(
                    "{label} target is unknown to the proxy; fetch the message first"
                ))
            })?;
            let channel_uuid = self.lookup_internal_channel(&internal_id).ok_or_else(|| {
                ProxyError::PermissionDenied(format!(
                    "{label} target is unknown to the proxy; fetch the message first"
                ))
            })?;
            match resolved_channel {
                None => resolved_channel = Some(channel_uuid),
                Some(existing) if existing != channel_uuid => {
                    return Err(ProxyError::PermissionDenied(format!(
                        "{label} targets span multiple channels — cross-channel targeting rejected"
                    )));
                }
                Some(_) => {}
            }
            resolved_ids.push((external_id.clone(), internal_id));
        }
        let id_map: std::collections::HashMap<&str, &str> = resolved_ids
            .iter()
            .map(|(ext, int)| (ext.as_str(), int.as_str()))
            .collect();

        let channel_uuid = resolved_channel.expect("e_tag_values non-empty guarantees Some");

        if !allowed_channels.contains(&channel_uuid) {
            return Err(ProxyError::PermissionDenied(format!(
                "{label} target channel not in invite scope"
            )));
        }

        // Build translated tag list: prepend #h, translate all #e values, drop
        // any client-supplied #h tags (prevent unauthorized channel injection).
        let mut new_tags: Vec<Tag> =
            vec![Tag::parse(["h", &channel_uuid.to_string()]).expect("h tag is always valid")];
        let mut saw_target = false;

        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            // Strip client-supplied #h tags (authorized #h already added above).
            if slice.first().map(|v| v.as_str()) == Some("h") {
                continue;
            }
            // Strip actor tags — untrusted client data should not persist.
            if slice.first().map(|v| v.as_str()) == Some("actor") {
                continue;
            }
            if slice.len() >= 2
                && slice[0] == "e"
                && slice[1].len() == 64
                && slice[1].chars().all(|ch| ch.is_ascii_hexdigit())
            {
                let internal_id = *id_map.get(slice[1].as_str()).ok_or_else(|| {
                    ProxyError::PermissionDenied(format!(
                        "{label} target is unknown to the proxy; fetch the message first"
                    ))
                })?;
                let mut parts = vec!["e".to_string(), internal_id.to_string()];
                parts.extend(slice.iter().skip(2).cloned());
                new_tags.push(
                    Tag::parse(parts).map_err(|e| {
                        ProxyError::Upstream(format!("{label} tag build failed: {e}"))
                    })?,
                );
                saw_target = true;
                continue;
            }
            new_tags.push(tag.clone());
        }

        if !saw_target {
            return Err(ProxyError::PermissionDenied(format!(
                "kind:{} event must reference a target message via #e",
                event.kind.as_u16()
            )));
        }

        let shadow_keys = self.shadow_keys.get_or_create(external_pubkey)?;
        let translated = EventBuilder::new(kind, &event.content)
            .tags(new_tags)
            .custom_created_at(event.created_at)
            .sign_with_keys(&shadow_keys)
            .map_err(|e| ProxyError::Upstream(format!("inbound signing failed: {e}")))?;

        self.cache_event_mapping(&translated.id.to_hex(), &event.id.to_hex());
        self.cache_internal_channel(&translated.id.to_hex(), channel_uuid);
        Ok(translated)
    }
}

impl Translator {
    /// Translate a NIP-28 REQ filter to Buzz format.
    ///
    /// - kind:42 / kind:1 → kind:9
    /// - Injects `#h` tag filters from `allowed_channels` so the client can
    ///   only receive events from channels they have access to.
    ///
    /// The returned filter is ready to be forwarded to the upstream Buzz relay.
    pub fn translate_filter_inbound(&self, filter: &Filter, allowed_channels: &[Uuid]) -> Filter {
        // Start with a clone and rebuild the kinds set.
        let mut f = filter.clone();

        if let Some(ref kinds) = filter.kinds {
            let new_kinds: Vec<Kind> = kinds
                .iter()
                .map(|k| {
                    let k_u32 = k.as_u16() as u32;
                    let buzz_k = self.kind_translator.to_buzz(k_u32);
                    // SAFETY: buzz kind values (9, 40002, 40003) always fit in u16
                    Kind::Custom(
                        u16::try_from(buzz_k).expect("SAFETY: buzz kind values always fit in u16"),
                    )
                })
                .collect();
            // Rebuild via the builder to stay consistent with nostr's internal state.
            f = f.remove_kinds(kinds.iter().cloned()).kinds(new_kinds);
        }

        // Check for client-supplied #e channel filters and translate to #h UUIDs.
        //
        // NIP-28 clients filter by channel using `#e <kind40_event_id>`. Buzz
        // uses `#h <uuid>` instead. If the client specified `#e` values, resolve
        // them to UUIDs (intersected with allowed_channels) and use those for the
        // `#h` injection. The `#e` filter must be removed from the translated
        // filter — if both `#e` and `#h` were present, the relay would AND them,
        // and since Buzz events carry `#h` but not `#e`, zero events would match.
        let e_tag_key = SingleLetterTag::lowercase(Alphabet::E);
        let had_e_filter = f.generic_tags.contains_key(&e_tag_key);
        let client_channel_uuids: Vec<String> =
            if let Some(e_values) = f.generic_tags.get(&e_tag_key) {
                e_values
                    .iter()
                    .filter_map(|event_id| self.channel_map.lookup_by_event_id(event_id))
                    .filter(|info| allowed_channels.contains(&info.uuid))
                    .map(|info| info.uuid.to_string())
                    .collect()
            } else {
                Vec::new()
            };

        // Remove the #e tag filter — Buzz uses #h, not #e.
        f.generic_tags.remove(&e_tag_key);

        // Inject #h tag constraints from the allowed channel list.
        // Three cases:
        //   1. Client specified #e and some resolved → use those (already intersected)
        //   2. Client specified #e but NONE resolved → deny-all (sentinel UUID)
        //   3. Client didn't specify #e → use all allowed_channels (or sentinel if empty)
        //
        // Case 2 is critical: an explicit filter that resolves to nothing must
        // return zero results, not widen to all channels.
        let sentinel = vec!["00000000-0000-0000-0000-000000000000".to_string()];
        let uuid_strings: Vec<String> = if !client_channel_uuids.is_empty() {
            // Case 1: client asked for specific channels, some resolved
            client_channel_uuids
        } else if had_e_filter {
            // Case 2: client asked for specific channels, none resolved → deny all
            sentinel
        } else if allowed_channels.is_empty() {
            // Case 3 with empty scope → deny all
            sentinel
        } else {
            // Case 3: no #e filter, use full allowed scope
            allowed_channels.iter().map(|u| u.to_string()).collect()
        };
        f = f.custom_tags(SingleLetterTag::lowercase(Alphabet::H), uuid_strings);

        f
    }
}

/// Extract plain text from V2 rich content JSON.
///
/// V2 content is a JSON object with a `"text"` field. Falls back to the raw
/// content string if parsing fails or the field is absent.
fn extract_plain_text(content: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_else(|| content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_map::{ChannelDto, ChannelMap};
    use buzz_core::kind::KIND_STREAM_MESSAGE;

    const TEST_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const TEST_SALT: &[u8] = b"test-salt-for-translate-tests";

    fn test_channel_map() -> ChannelMap {
        let keys = Keys::generate();
        let map = ChannelMap::new(keys);
        let dto = ChannelDto {
            id: TEST_UUID.to_string(),
            name: "test-channel".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            visibility: "open".to_string(),
            description: "A test channel".to_string(),
            created_by: "0101010101010101010101010101010101010101010101010101010101010101"
                .to_string(),
        };
        map.register(&dto)
            .expect("test channel registration must succeed");
        map
    }

    fn make_translator() -> (Translator, String) {
        let channel_map = Arc::new(test_channel_map());
        let shadow_mgr = Arc::new(
            ShadowKeyManager::new(TEST_SALT).expect("shadow key manager creation must succeed"),
        );
        let translator = Translator::new(
            shadow_mgr,
            channel_map.clone(),
            "http://localhost:3000",
            "buzz_test",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        );

        // Retrieve the kind:40 event ID for the test channel.
        let uuid: Uuid = TEST_UUID.parse().unwrap();
        let info = channel_map.lookup_by_uuid(&uuid).unwrap();
        (translator, info.kind40_event_id)
    }

    fn allowed() -> Vec<Uuid> {
        vec![TEST_UUID.parse().unwrap()]
    }

    fn no_channels() -> Vec<Uuid> {
        vec![]
    }

    #[tokio::test]
    async fn outbound_translates_stream_message() {
        let (translator, kind40_event_id) = make_translator();
        let author_keys = Keys::generate();

        // Build a synthetic kind:9 event with an #h tag.
        let h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let buzz_event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "hello world")
            .tags([h_tag])
            .sign_with_keys(&author_keys)
            .unwrap();

        let result = translator
            .translate_outbound(&buzz_event, &allowed())
            .await
            .expect("outbound translation must not error");

        let translated = result.expect("should produce a translated event");

        // Kind must be 42.
        assert_eq!(translated.kind.as_u16(), 42, "translated kind must be 42");

        // Must have an #e tag pointing to the kind:40 event ID.
        let has_e_tag = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "e" && s[1] == kind40_event_id
        });
        assert!(
            has_e_tag,
            "translated event must have #e tag with kind:40 event ID"
        );

        // Must NOT have an #h tag.
        let has_h_tag = translated
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("h"));
        assert!(!has_h_tag, "translated event must not retain #h tag");

        // Content must be preserved.
        assert_eq!(translated.content, "hello world");

        // Signature must be valid.
        translated
            .verify()
            .expect("translated event signature must be valid");
    }

    #[test]
    fn inbound_translates_channel_message() {
        let (translator, kind40_event_id) = make_translator();
        let client_keys = Keys::generate();
        let external_pubkey = client_keys.public_key().to_hex();

        // Build a synthetic kind:42 event with an #e tag.
        let e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let nip28_event = EventBuilder::new(Kind::Custom(42), "hello from client")
            .tags([e_tag])
            .sign_with_keys(&client_keys)
            .unwrap();

        let translated = translator
            .translate_inbound(&nip28_event, &external_pubkey, &allowed())
            .expect("inbound translation must not error");

        // Kind must be KIND_STREAM_MESSAGE.
        assert_eq!(
            translated.kind.as_u16(),
            KIND_STREAM_MESSAGE as u16,
            "translated kind must be KIND_STREAM_MESSAGE"
        );

        // Must have an #h tag pointing to the channel UUID.
        let has_h_tag = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "h" && s[1] == TEST_UUID
        });
        assert!(
            has_h_tag,
            "translated event must have #h tag with channel UUID"
        );

        // Must NOT have an #e tag.
        let has_e_tag = translated
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("e"));
        assert!(!has_e_tag, "translated event must not retain #e tag");

        // Content must be preserved.
        assert_eq!(translated.content, "hello from client");

        // Signature must be valid.
        translated
            .verify()
            .expect("translated event signature must be valid");
    }

    #[tokio::test]
    async fn outbound_rejects_channel_not_in_scope() {
        let (translator, _) = make_translator();
        let author_keys = Keys::generate();

        let h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let buzz_event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "secret")
            .tags([h_tag])
            .sign_with_keys(&author_keys)
            .unwrap();

        let result = translator
            .translate_outbound(&buzz_event, &no_channels())
            .await;

        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "expected PermissionDenied, got: {:?}",
            result
        );
    }

    #[test]
    fn inbound_rejects_channel_not_in_scope() {
        let (translator, kind40_event_id) = make_translator();
        let client_keys = Keys::generate();
        let external_pubkey = client_keys.public_key().to_hex();

        let e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let nip28_event = EventBuilder::new(Kind::Custom(42), "sneaky")
            .tags([e_tag])
            .sign_with_keys(&client_keys)
            .unwrap();

        let result = translator.translate_inbound(&nip28_event, &external_pubkey, &no_channels());

        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "expected PermissionDenied, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn v2_content_plain_text_extracted() {
        // extract_plain_text is a private helper; test it indirectly via outbound.
        let (translator, _) = make_translator();
        let author_keys = Keys::generate();

        let v2_content = r#"{"text":"hello v2","attachments":[]}"#;
        let h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let buzz_event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE_V2 as u16), v2_content)
            .tags([h_tag])
            .sign_with_keys(&author_keys)
            .unwrap();

        let translated = translator
            .translate_outbound(&buzz_event, &allowed())
            .await
            .unwrap()
            .expect("should produce translated event");

        assert_eq!(
            translated.content, "hello v2",
            "V2 rich content must be unwrapped to plain text"
        );
    }

    #[test]
    fn v2_content_fallback_on_non_json() {
        // When V2 content is not valid JSON, fall back to raw content.
        let content = extract_plain_text("not json at all");
        assert_eq!(content, "not json at all");
    }

    #[test]
    fn filter_inbound_translates_kind() {
        let (translator, _) = make_translator();

        let filter = Filter::new().kind(Kind::Custom(42));
        let translated = translator.translate_filter_inbound(&filter, &allowed());

        // The translated filter must contain KIND_STREAM_MESSAGE, not 42.
        let kinds = translated.kinds.as_ref().expect("filter must have kinds");
        assert!(
            kinds.contains(&Kind::Custom(KIND_STREAM_MESSAGE as u16)),
            "filter must contain KIND_STREAM_MESSAGE after translation"
        );
        assert!(
            !kinds.contains(&Kind::Custom(42)),
            "filter must not contain kind:42 after translation"
        );

        // Must have injected #h tag constraints.
        let h_tag = SingleLetterTag::lowercase(Alphabet::H);
        let has_h_filter = translated.generic_tags.contains_key(&h_tag);
        assert!(has_h_filter, "filter must have #h tag constraints injected");
    }

    #[test]
    fn filter_inbound_translates_e_tag_to_h() {
        let (translator, kind40_event_id) = make_translator();

        // Build a filter that includes a #e tag referencing the test channel's
        // kind:40 event ID — this is how NIP-28 clients subscribe to a channel.
        let e_tag_key = SingleLetterTag::lowercase(Alphabet::E);
        let filter = Filter::new()
            .kind(Kind::Custom(42))
            .custom_tags(e_tag_key, [kind40_event_id]);

        let translated = translator.translate_filter_inbound(&filter, &allowed());

        // The #e filter must be removed from the translated filter.
        assert!(
            !translated.generic_tags.contains_key(&e_tag_key),
            "#e tag filter must be removed from translated filter (would cause zero matches on Buzz relay)"
        );

        // The #h filter must be present and contain the channel UUID.
        let h_tag_key = SingleLetterTag::lowercase(Alphabet::H);
        let h_values = translated
            .generic_tags
            .get(&h_tag_key)
            .expect("translated filter must have #h tag constraint");

        assert!(
            h_values.contains(TEST_UUID),
            "#h filter must contain the channel UUID resolved from #e, got: {:?}",
            h_values
        );

        // The translated filter must NOT contain all allowed_channels blindly —
        // it should contain exactly the channel(s) resolved from the #e values.
        assert_eq!(
            h_values.len(),
            1,
            "#h filter must contain exactly the one channel resolved from #e, got: {:?}",
            h_values
        );
    }

    #[test]
    fn filter_inbound_e_tag_unknown_event_id_denies_all() {
        let (translator, _) = make_translator();

        // Use an event ID that doesn't exist in the channel map.
        let unknown_event_id = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let e_tag_key = SingleLetterTag::lowercase(Alphabet::E);
        let filter = Filter::new()
            .kind(Kind::Custom(42))
            .custom_tags(e_tag_key, [unknown_event_id]);

        let translated = translator.translate_filter_inbound(&filter, &allowed());

        // The #e filter must be removed.
        assert!(
            !translated.generic_tags.contains_key(&e_tag_key),
            "#e tag filter must be removed even when event ID is unknown"
        );

        // Since the client explicitly specified #e values but none resolved,
        // the filter must deny all — inject the sentinel UUID, NOT fall back
        // to all allowed_channels. Explicit filter → fail closed.
        let h_tag_key = SingleLetterTag::lowercase(Alphabet::H);
        let h_values = translated
            .generic_tags
            .get(&h_tag_key)
            .expect("translated filter must have #h tag constraint");

        let sentinel = "00000000-0000-0000-0000-000000000000";
        assert!(
            h_values.contains(sentinel),
            "unknown #e must inject sentinel UUID (deny-all), got: {:?}",
            h_values
        );
        assert!(
            !h_values.contains(TEST_UUID),
            "unknown #e must NOT fall back to allowed channels"
        );
    }

    #[test]
    fn inbound_preserves_reply_e_tags() {
        let (translator, kind40_event_id) = make_translator();
        let client_keys = Keys::generate();
        let external_pubkey = client_keys.public_key().to_hex();

        // A reply event has two #e tags: one for the channel, one for the
        // message being replied to (NIP-10 threading).
        let reply_event_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let channel_e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let reply_e_tag = Tag::parse(["e", reply_event_id]).unwrap();

        let nip28_event = EventBuilder::new(Kind::Custom(42), "replying to a message")
            .tags([channel_e_tag, reply_e_tag])
            .sign_with_keys(&client_keys)
            .unwrap();

        let translated = translator
            .translate_inbound(&nip28_event, &external_pubkey, &allowed())
            .expect("inbound translation must not error");

        // Must have the #h tag for the channel.
        let has_h_tag = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "h" && s[1] == TEST_UUID
        });
        assert!(
            has_h_tag,
            "translated event must have #h tag with channel UUID"
        );

        // The channel #e tag must be gone.
        let has_channel_e = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "e" && s[1] == kind40_event_id
        });
        assert!(!has_channel_e, "channel #e tag must be stripped");

        // The reply #e tag must be preserved.
        let has_reply_e = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "e" && s[1] == reply_event_id
        });
        assert!(
            has_reply_e,
            "reply #e tag must be preserved for NIP-10 threading"
        );

        translated
            .verify()
            .expect("translated event signature must be valid");
    }

    #[tokio::test]
    async fn outbound_preserves_non_channel_h_tags() {
        let (translator, kind40_event_id) = make_translator();
        let author_keys = Keys::generate();

        // An event with the channel #h tag AND an unrelated #h tag.
        let channel_h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let other_h_tag = Tag::parse(["h", "some-other-value"]).unwrap();

        let buzz_event = EventBuilder::new(
            Kind::Custom(KIND_STREAM_MESSAGE as u16),
            "message with extra h tag",
        )
        .tags([channel_h_tag, other_h_tag])
        .sign_with_keys(&author_keys)
        .unwrap();

        let result = translator
            .translate_outbound(&buzz_event, &allowed())
            .await
            .expect("outbound translation must not error");

        let translated = result.expect("should produce a translated event");

        // The channel #h tag must be replaced by #e.
        let has_channel_e = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "e" && s[1] == kind40_event_id
        });
        assert!(has_channel_e, "channel #e tag must be present");

        // The channel #h tag must be gone.
        let has_channel_h = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "h" && s[1] == TEST_UUID
        });
        assert!(!has_channel_h, "channel #h tag must be stripped");

        // The unrelated #h tag must be preserved.
        let has_other_h = translated.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "h" && s[1] == "some-other-value"
        });
        assert!(has_other_h, "non-channel #h tag must be preserved");

        translated
            .verify()
            .expect("translated event signature must be valid");
    }

    #[test]
    fn empty_allowed_channels_denies_all() {
        let (translator, _) = make_translator();

        let filter = Filter::new().kind(Kind::Custom(42));
        let translated = translator.translate_filter_inbound(&filter, &no_channels());

        // Must have an #h tag constraint.
        let h_tag = SingleLetterTag::lowercase(Alphabet::H);
        let h_values = translated
            .generic_tags
            .get(&h_tag)
            .expect("filter must have #h tag constraint even with empty allowed_channels");

        // The injected value must be the impossible sentinel UUID.
        assert!(
            h_values.contains("00000000-0000-0000-0000-000000000000"),
            "empty allowed_channels must inject impossible sentinel UUID, got: {:?}",
            h_values
        );
    }

    #[test]
    fn inbound_translates_edit_message() {
        use buzz_core::kind::KIND_STREAM_MESSAGE_EDIT;

        let (translator, kind40_event_id) = make_translator();
        let external_keys = Keys::generate();

        // Build a kind:41 event with #e referencing the channel.
        let e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let nip28_event = EventBuilder::new(Kind::ChannelMetadata, "updated metadata")
            .tags([e_tag])
            .sign_with_keys(&external_keys)
            .unwrap();

        let translated = translator
            .translate_inbound(
                &nip28_event,
                &external_keys.public_key().to_hex(),
                &allowed(),
            )
            .expect("inbound kind:41 must translate");

        // kind:41 → KIND_STREAM_MESSAGE_EDIT (40003)
        assert_eq!(
            translated.kind.as_u16() as u32,
            KIND_STREAM_MESSAGE_EDIT,
            "kind:41 must translate to KIND_STREAM_MESSAGE_EDIT"
        );

        // Must have #h tag (channel UUID), not #e.
        let has_h = translated
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("h"));
        assert!(has_h, "translated edit must have #h tag");

        translated
            .verify()
            .expect("translated edit signature must be valid");
    }

    #[test]
    fn inbound_rejects_unknown_kind() {
        let (translator, kind40_event_id) = make_translator();
        let external_keys = Keys::generate();

        let e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let event = EventBuilder::new(Kind::Custom(9999), "nope")
            .tags([e_tag])
            .sign_with_keys(&external_keys)
            .unwrap();

        let result =
            translator.translate_inbound(&event, &external_keys.public_key().to_hex(), &allowed());
        assert!(result.is_err(), "kind:9999 must be rejected inbound");
    }

    #[tokio::test]
    async fn outbound_translates_edit_message() {
        use buzz_core::kind::KIND_STREAM_MESSAGE_EDIT;

        let (translator, _) = make_translator();
        let author_keys = Keys::generate();

        let h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let buzz_event = EventBuilder::new(
            Kind::Custom(KIND_STREAM_MESSAGE_EDIT as u16),
            "edited content",
        )
        .tags([h_tag])
        .sign_with_keys(&author_keys)
        .unwrap();

        let result = translator
            .translate_outbound(&buzz_event, &allowed())
            .await
            .expect("outbound translation of edit must not error");

        let translated = result.expect("edit must produce a translated event, not None");

        // kind:40003 must translate to kind:41 (NIP-28 channel edit).
        assert_eq!(
            translated.kind.as_u16(),
            41,
            "kind:40003 must translate to kind:41"
        );

        // Content must be preserved.
        assert_eq!(translated.content, "edited content");

        // Must have #e tag (channel reference), not #h.
        let has_e_tag = translated
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("e"));
        assert!(has_e_tag, "translated edit must have #e tag");

        let has_h_tag = translated
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|v| v.as_str()) == Some("h"));
        assert!(!has_h_tag, "translated edit must not retain #h tag");

        translated
            .verify()
            .expect("translated edit signature must be valid");
    }

    #[test]
    fn inbound_strips_injected_h_tags() {
        let (translator, kind40_event_id) = make_translator();
        let external_keys = Keys::generate();

        // Client tries to inject an #h tag for an unauthorized channel.
        let e_tag = Tag::parse(["e", &kind40_event_id]).unwrap();
        let injected_h = Tag::parse(["h", "00000000-0000-0000-0000-000000000001"]).unwrap();
        let nip28_event = EventBuilder::new(Kind::Custom(42), "sneaky message")
            .tags([e_tag, injected_h])
            .sign_with_keys(&external_keys)
            .unwrap();

        let translated = translator
            .translate_inbound(
                &nip28_event,
                &external_keys.public_key().to_hex(),
                &allowed(),
            )
            .expect("inbound must succeed");

        // Only the authorized #h tag should be present.
        let h_tags: Vec<_> = translated
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|v| v.as_str()) == Some("h"))
            .collect();
        assert_eq!(h_tags.len(), 1, "only the authorized #h tag should survive");
        assert_eq!(
            h_tags[0].as_slice().get(1).map(|v| v.as_str()),
            Some(TEST_UUID),
            "the surviving #h tag must be the authorized channel UUID"
        );
    }

    #[tokio::test]
    async fn outbound_drops_unknown_kinds() {
        let (translator, _) = make_translator();
        let author_keys = Keys::generate();

        // A kind:9999 event (unknown Buzz kind) should be dropped.
        let h_tag = Tag::parse(["h", TEST_UUID]).unwrap();
        let event = EventBuilder::new(Kind::Custom(9999), "internal stuff")
            .tags([h_tag])
            .sign_with_keys(&author_keys)
            .unwrap();

        let result = translator
            .translate_outbound(&event, &allowed())
            .await
            .expect("outbound must not error");

        assert!(
            result.is_none(),
            "unknown kinds must be dropped, not passed through to clients"
        );
    }
}
