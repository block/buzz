//! Presence status types shared across REST, MCP, and WebSocket surfaces.

use serde::{Deserialize, Serialize};

/// Allowed presence statuses for the REST/MCP surface.
///
/// The WebSocket path (kind:20001) accepts arbitrary status strings for
/// forward-compatibility; this enum is the curated set for structured APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    /// User is actively online.
    Online,
    /// User is away / idle.
    Away,
    /// User is offline; clears the presence entry.
    Offline,
}

impl PresenceStatus {
    /// Returns the lowercase string representation stored in Redis.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Offline => "offline",
        }
    }
}

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let online: PresenceStatus = serde_json::from_str(r#""online""#).unwrap();
        assert_eq!(online, PresenceStatus::Online);
        assert_eq!(serde_json::to_string(&online).unwrap(), r#""online""#);

        let away: PresenceStatus = serde_json::from_str(r#""away""#).unwrap();
        assert_eq!(away, PresenceStatus::Away);

        let offline: PresenceStatus = serde_json::from_str(r#""offline""#).unwrap();
        assert_eq!(offline, PresenceStatus::Offline);
    }

    #[test]
    fn serde_rejects_unknown_variant() {
        let result: Result<PresenceStatus, _> = serde_json::from_str(r#""invisible""#);
        assert!(result.is_err());
    }

    #[test]
    fn as_str_matches_serde() {
        assert_eq!(PresenceStatus::Online.as_str(), "online");
        assert_eq!(PresenceStatus::Away.as_str(), "away");
        assert_eq!(PresenceStatus::Offline.as_str(), "offline");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", PresenceStatus::Online), "online");
        assert_eq!(format!("{}", PresenceStatus::Away), "away");
        assert_eq!(format!("{}", PresenceStatus::Offline), "offline");
    }
}
