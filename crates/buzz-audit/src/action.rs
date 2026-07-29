use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Audit action recorded for each event in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// A Nostr event was created.
    EventCreated,
    /// A Nostr event was deleted.
    EventDeleted,
    /// A channel was created.
    ChannelCreated,
    /// A channel's metadata was updated.
    ChannelUpdated,
    /// A channel was deleted.
    ChannelDeleted,
    /// A member was added to a channel.
    MemberAdded,
    /// A member was removed from a channel.
    MemberRemoved,
    /// A client successfully authenticated.
    AuthSuccess,
    /// A client authentication attempt failed.
    AuthFailure,
    /// A client exceeded the rate limit.
    RateLimitExceeded,
    /// A media file was uploaded via the Blossom endpoint.
    MediaUploaded,
}

impl AuditAction {
    /// Stable string representation used in hash computation and DB storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EventCreated => "event_created",
            Self::EventDeleted => "event_deleted",
            Self::ChannelCreated => "channel_created",
            Self::ChannelUpdated => "channel_updated",
            Self::ChannelDeleted => "channel_deleted",
            Self::MemberAdded => "member_added",
            Self::MemberRemoved => "member_removed",
            Self::AuthSuccess => "auth_success",
            Self::AuthFailure => "auth_failure",
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::MediaUploaded => "media_uploaded",
        }
    }

    const ALL: &'static [Self] = &[
        Self::EventCreated,
        Self::EventDeleted,
        Self::ChannelCreated,
        Self::ChannelUpdated,
        Self::ChannelDeleted,
        Self::MemberAdded,
        Self::MemberRemoved,
        Self::AuthSuccess,
        Self::AuthFailure,
        Self::RateLimitExceeded,
        Self::MediaUploaded,
    ];
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuditAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .find(|a| a.as_str() == s)
            .cloned()
            .ok_or_else(|| format!("unknown audit action: {s:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_variants() {
        for action in AuditAction::ALL {
            let parsed: AuditAction = action.to_string().parse().unwrap();
            assert_eq!(&parsed, action);
        }
    }

    #[test]
    fn unknown_action_returns_err() {
        assert!("totally_bogus".parse::<AuditAction>().is_err());
    }

    #[test]
    fn serde_json_roundtrip_all_variants() {
        // The action is stored as a string column and deserialized from DB
        // rows; the serde representation must match as_str/FromStr exactly.
        for action in AuditAction::ALL {
            let json = serde_json::to_string(action).unwrap();
            let back: AuditAction = serde_json::from_str(&json).unwrap();
            assert_eq!(action, &back);
        }
    }

    #[test]
    fn serde_uses_snake_case() {
        // The serde rename_all = "snake_case" must agree with as_str().
        // If someone adds a variant but forgets to update as_str(), or vice
        // versa, this catches the drift.
        for action in AuditAction::ALL {
            let json = serde_json::to_string(action).unwrap();
            // serde produces a quoted string like "\"event_created\"".
            let expected = format!("\"{}\"", action.as_str());
            assert_eq!(
                json, expected,
                "serde representation differs from as_str() for {action:?}"
            );
        }
    }

    #[test]
    fn as_str_values_are_unique() {
        // Two variants sharing the same as_str() would be ambiguous on parse
        // and on DB read — this is a silent correctness bug.
        let strs: Vec<&str> = AuditAction::ALL.iter().map(|a| a.as_str()).collect();
        let unique: std::collections::HashSet<&str> = strs.iter().copied().collect();
        assert_eq!(strs.len(), unique.len(), "duplicate as_str() values");
    }

    #[test]
    fn as_str_and_from_str_are_inverses() {
        // Round-trip through the string representation used in DB storage.
        for action in AuditAction::ALL {
            let s = action.as_str();
            let parsed: AuditAction = s.parse().unwrap();
            assert_eq!(&parsed, action);
        }
    }
}
