//! Bidirectional UUID ↔ NIP-28 kind:40 event ID mapping.
//!
//! Initialized from the relay's REST API (`GET /api/channels`). Synthesizes
//! deterministic kind:40 channel creation events so that the same
//! (name, created_at, server_keys) always produces the same event ID across
//! restarts.

use dashmap::DashMap;
use nostr::prelude::*;
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

/// Minimal DTO for deserializing `GET /api/channels` response.
#[derive(Debug, Deserialize)]
pub struct ChannelDto {
    /// UUID string (e.g. `"550e8400-e29b-41d4-a716-446655440000"`).
    pub id: String,
    /// Human-readable channel name.
    pub name: String,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// Channel visibility (e.g. `"public"` / `"private"`).
    pub visibility: String,
    /// Optional channel description.
    #[serde(default)]
    pub description: String,
    /// Hex-encoded pubkey of the creator.
    #[serde(default)]
    pub created_by: String,
}

/// All relevant information about a mapped channel.
#[derive(Debug, Clone)]
pub struct ChannelInfo {
    /// The Buzz-internal UUID.
    pub uuid: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Hex pubkey of the original creator.
    pub created_by: String,
    /// Hex-encoded NIP-28 kind:40 event ID (deterministic).
    pub kind40_event_id: String,
    /// Creation time as UNIX timestamp (seconds).
    pub created_at_unix: u64,
}

/// Bidirectional UUID ↔ kind:40 event ID map.
///
/// Thread-safe via [`DashMap`]; clone-friendly via `Arc` wrapping at the call
/// site.
pub struct ChannelMap {
    by_uuid: DashMap<Uuid, ChannelInfo>,
    by_event_id: DashMap<String, Uuid>,
    server_keys: Keys,
}

impl ChannelMap {
    /// Synthesize a deterministic NIP-28 kind:40 channel creation event.
    ///
    /// The event ID is a pure function of `(uuid, created_at_unix, server_keys)`.
    /// Calling this twice with the same inputs yields the same [`Event`] and
    /// therefore the same event ID.
    ///
    /// **Important**: The `uuid` (not the human-readable name) is used as the
    /// identity anchor in the content JSON. This ensures the kind:40 event ID
    /// remains stable even if the channel is renamed. The real name belongs in
    /// kind:41 (mutable metadata), not here.
    pub fn synthesize_kind40(&self, uuid: &str, created_at_unix: u64) -> Event {
        let content = serde_json::json!({
            "name": uuid,  // Use UUID for deterministic ID stability — name goes in kind:41
            "about": "",
            "picture": ""
        })
        .to_string();

        // nostr 0.36: EventBuilder::new(kind, content).tags( tags)
        // SAFETY: signing with a pre-validated Keys instance cannot fail
        EventBuilder::new(Kind::ChannelCreation, content)
            .tags([])
            .custom_created_at(Timestamp::from(created_at_unix))
            .sign_with_keys(&self.server_keys)
            .expect("SAFETY: signing with valid keys cannot fail")
    }

    /// Synthesize a NIP-28 kind:41 channel metadata event that references the
    /// kind:40 event via an `e` tag.
    pub fn synthesize_kind41(&self, info: &ChannelInfo) -> Event {
        let content = serde_json::json!({
            "name": info.name,
            "about": info.description,
            "picture": ""
        })
        .to_string();

        // SAFETY: kind40_event_id is always a valid hex string — it was produced by event.id.to_hex() in register()
        let e_tag = Tag::event(
            EventId::from_hex(&info.kind40_event_id)
                .expect("SAFETY: kind40_event_id is always valid hex from event.id.to_hex()"),
        );

        // SAFETY: signing with a pre-validated Keys instance cannot fail
        EventBuilder::new(Kind::ChannelMetadata, content)
            .tags([e_tag])
            .custom_created_at(Timestamp::from(info.created_at_unix))
            .sign_with_keys(&self.server_keys)
            .expect("SAFETY: signing with valid keys cannot fail")
    }
}

impl ChannelMap {
    /// Create an empty [`ChannelMap`] with the given server signing keys.
    pub fn new(server_keys: Keys) -> Self {
        Self {
            by_uuid: DashMap::new(),
            by_event_id: DashMap::new(),
            server_keys,
        }
    }

    /// Initialize from `GET /api/channels` REST endpoint.
    ///
    /// Fetches all channels, synthesizes a deterministic kind:40 event for each
    /// one, and populates both lookup tables.
    pub async fn init_from_rest(
        server_keys: Keys,
        api_base: &str,
        api_token: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let map = Self::new(server_keys);

        // SAFETY: default builder with only timeout config cannot fail
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("SAFETY: default builder with only timeout config cannot fail");
        let channels: Vec<ChannelDto> = client
            .get(format!("{}/api/channels", api_base))
            .header("Authorization", format!("Bearer {}", api_token))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        for ch in &channels {
            map.register(ch)?;
        }

        info!(
            count = channels.len(),
            "channel map initialized from REST API"
        );
        Ok(map)
    }

    /// Look up channel info by Buzz UUID.
    pub fn lookup_by_uuid(&self, uuid: &Uuid) -> Option<ChannelInfo> {
        self.by_uuid.get(uuid).map(|r| r.clone())
    }

    /// Look up channel info by NIP-28 kind:40 event ID (hex).
    pub fn lookup_by_event_id(&self, event_id: &str) -> Option<ChannelInfo> {
        self.by_event_id
            .get(event_id)
            .and_then(|uuid| self.by_uuid.get(&*uuid).map(|r| r.clone()))
    }

    /// Return all channel infos (e.g. for serving kind:40 `REQ` responses).
    pub fn all_channels(&self) -> Vec<ChannelInfo> {
        self.by_uuid.iter().map(|r| r.value().clone()).collect()
    }

    /// Register a new channel (e.g. from a kind:40099 system message) and
    /// return its [`ChannelInfo`].
    ///
    /// Idempotent: registering the same UUID twice overwrites with the latest
    /// data (event ID will be identical for the same inputs).
    pub fn register(
        &self,
        dto: &ChannelDto,
    ) -> Result<ChannelInfo, Box<dyn std::error::Error + Send + Sync>> {
        let uuid = dto.id.parse::<Uuid>()?;
        let created_at_unix =
            chrono::DateTime::parse_from_rfc3339(&dto.created_at)?.timestamp() as u64;

        let kind40 = self.synthesize_kind40(&dto.id, created_at_unix);
        let event_id = kind40.id.to_hex();

        let info = ChannelInfo {
            uuid,
            name: dto.name.clone(),
            description: dto.description.clone(),
            created_by: dto.created_by.clone(),
            kind40_event_id: event_id.clone(),
            created_at_unix,
        };

        // Clean up stale by_event_id entry if this UUID was previously registered
        // with a different event ID (e.g. after channel rename or data change).
        if let Some(old_info) = self.by_uuid.get(&uuid) {
            if old_info.kind40_event_id != event_id {
                self.by_event_id.remove(&old_info.kind40_event_id);
            }
        }
        self.by_uuid.insert(uuid, info.clone());
        self.by_event_id.insert(event_id, uuid);
        Ok(info)
    }

    /// Number of channels in the map.
    pub fn len(&self) -> usize {
        self.by_uuid.len()
    }

    /// Returns `true` if no channels have been registered.
    pub fn is_empty(&self) -> bool {
        self.by_uuid.is_empty()
    }

    /// The server signing keys (needed for kind:40/41 synthesis by other modules).
    pub fn server_keys(&self) -> &Keys {
        &self.server_keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map() -> ChannelMap {
        let keys = Keys::generate();
        ChannelMap::new(keys)
    }

    fn sample_dto(id: &str, name: &str) -> ChannelDto {
        ChannelDto {
            id: id.to_string(),
            name: name.to_string(),
            created_at: "2024-01-15T12:00:00Z".to_string(),
            visibility: "public".to_string(),
            description: "A test channel".to_string(),
            created_by: "deadbeef".to_string(),
        }
    }

    #[test]
    fn test_synthesize_kind40_deterministic() {
        let map = make_map();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let event_a = map.synthesize_kind40(uuid, 1_700_000_000);
        let event_b = map.synthesize_kind40(uuid, 1_700_000_000);
        assert_eq!(
            event_a.id, event_b.id,
            "same inputs must produce same event ID"
        );
    }

    #[test]
    fn test_synthesize_kind40_different_inputs() {
        let map = make_map();
        let uuid_a = "550e8400-e29b-41d4-a716-446655440000";
        let uuid_b = "660f9511-f3ac-52e5-b827-557766551111";
        let event_a = map.synthesize_kind40(uuid_a, 1_700_000_000);
        let event_b = map.synthesize_kind40(uuid_b, 1_700_000_000);
        let event_c = map.synthesize_kind40(uuid_a, 1_700_000_001);
        assert_ne!(event_a.id, event_b.id, "different UUIDs → different IDs");
        assert_ne!(
            event_a.id, event_c.id,
            "different timestamps → different IDs"
        );
    }

    #[test]
    fn test_register_and_lookup_by_uuid() {
        let map = make_map();
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let dto = sample_dto(uuid_str, "general");

        let info = map.register(&dto).expect("register should succeed");
        assert_eq!(info.name, "general");
        assert_eq!(info.uuid.to_string(), uuid_str);

        let uuid: Uuid = uuid_str.parse().unwrap();
        let found = map.lookup_by_uuid(&uuid).expect("should find by uuid");
        assert_eq!(found.kind40_event_id, info.kind40_event_id);
    }

    #[test]
    fn test_lookup_by_event_id() {
        let map = make_map();
        let uuid_str = "550e8400-e29b-41d4-a716-446655440001";
        let dto = sample_dto(uuid_str, "announcements");

        let info = map.register(&dto).expect("register should succeed");

        let found = map
            .lookup_by_event_id(&info.kind40_event_id)
            .expect("should find by event_id");
        assert_eq!(found.uuid.to_string(), uuid_str);
        assert_eq!(found.name, "announcements");
    }

    #[test]
    fn test_all_channels_returns_all() {
        let map = make_map();
        let uuids = [
            "550e8400-e29b-41d4-a716-446655440002",
            "550e8400-e29b-41d4-a716-446655440003",
            "550e8400-e29b-41d4-a716-446655440004",
        ];
        for (i, uuid_str) in uuids.iter().enumerate() {
            let dto = sample_dto(uuid_str, &format!("channel-{}", i));
            map.register(&dto).expect("register should succeed");
        }

        let all = map.all_channels();
        assert_eq!(all.len(), 3, "all_channels should return all 3 channels");
    }

    #[test]
    fn test_len_and_is_empty() {
        let map = make_map();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        let dto = sample_dto("550e8400-e29b-41d4-a716-446655440005", "test");
        map.register(&dto).unwrap();

        assert!(!map.is_empty());
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_synthesize_kind41_references_kind40() {
        let map = make_map();
        let uuid_str = "550e8400-e29b-41d4-a716-446655440006";
        let dto = sample_dto(uuid_str, "meta-test");
        let info = map.register(&dto).unwrap();

        let kind41 = map.synthesize_kind41(&info);
        assert_eq!(kind41.kind, Kind::ChannelMetadata);

        // Verify the e tag references the kind:40 event ID
        let has_e_tag = kind41.tags.iter().any(|tag| {
            tag.as_slice().first().map(|s| s == "e").unwrap_or(false)
                && tag
                    .as_slice()
                    .get(1)
                    .map(|s| s == &info.kind40_event_id)
                    .unwrap_or(false)
        });
        assert!(
            has_e_tag,
            "kind:41 must have e tag pointing to kind:40 event"
        );
    }

    #[test]
    fn register_cleans_up_stale_event_id() {
        let keys = Keys::generate();
        let map = ChannelMap::new(keys);

        let dto1 = ChannelDto {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "original-name".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            visibility: "open".to_string(),
            description: "test".to_string(),
            created_by: "0101010101010101010101010101010101010101010101010101010101010101"
                .to_string(),
        };
        let info1 = map.register(&dto1).unwrap();

        // Re-register with different created_at (changes the synthesized event ID)
        let dto2 = ChannelDto {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "original-name".to_string(),
            created_at: "2026-06-01T00:00:00Z".to_string(),
            visibility: "open".to_string(),
            description: "test".to_string(),
            created_by: "0101010101010101010101010101010101010101010101010101010101010101"
                .to_string(),
        };
        let info2 = map.register(&dto2).unwrap();

        // Old event ID should no longer resolve
        assert!(
            map.lookup_by_event_id(&info1.kind40_event_id).is_none(),
            "stale event ID must be cleaned up"
        );
        // New event ID should resolve
        assert!(
            map.lookup_by_event_id(&info2.kind40_event_id).is_some(),
            "new event ID must resolve"
        );
    }
}
