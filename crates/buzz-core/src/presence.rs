//! Presence status types shared across REST, MCP, and WebSocket surfaces.

use serde::{Deserialize, Serialize};

/// Interval used by connected clients to refresh their signed presence lease.
///
/// The relay lease is deliberately three intervals long, leaving one complete
/// interval of scheduling/network margin after a single dropped refresh.
pub const PRESENCE_HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Lifetime of a presence lease in the relay's shared store.
///
/// Keep this at three times [`PRESENCE_HEARTBEAT_INTERVAL_SECS`]. A client that
/// misses one refresh then has another full heartbeat interval to recover
/// before the relay expires its lease.
pub const PRESENCE_LEASE_TTL_SECS: u64 = PRESENCE_HEARTBEAT_INTERVAL_SECS * 3;

/// Refresh interval for the owner-encrypted managed-agent runtime lease.
pub const MANAGED_AGENT_RUNTIME_LEASE_INTERVAL_SECS: u64 = 15;

/// Lifetime of a managed-agent runtime lease.
///
/// Three intervals tolerate one lost encrypted observer frame while retaining
/// a bounded stale-to-unknown transition.
pub const MANAGED_AGENT_RUNTIME_LEASE_TTL_SECS: u64 = MANAGED_AGENT_RUNTIME_LEASE_INTERVAL_SECS * 3;

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

    #[test]
    fn lease_survives_one_missed_heartbeat_with_a_full_interval_of_margin() {
        let one_missed_refresh_gap = PRESENCE_HEARTBEAT_INTERVAL_SECS * 2;
        assert!(one_missed_refresh_gap < PRESENCE_LEASE_TTL_SECS);
        assert_eq!(
            PRESENCE_LEASE_TTL_SECS - one_missed_refresh_gap,
            PRESENCE_HEARTBEAT_INTERVAL_SECS
        );
    }

    #[test]
    fn managed_runtime_lease_survives_one_missed_frame_then_expires() {
        let one_missed_refresh_gap = MANAGED_AGENT_RUNTIME_LEASE_INTERVAL_SECS * 2;
        assert!(one_missed_refresh_gap < MANAGED_AGENT_RUNTIME_LEASE_TTL_SECS);
        assert_eq!(
            MANAGED_AGENT_RUNTIME_LEASE_TTL_SECS - one_missed_refresh_gap,
            MANAGED_AGENT_RUNTIME_LEASE_INTERVAL_SECS
        );
    }
}
