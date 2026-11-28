use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Audit action recorded for each event in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// An authorized governance command replaced a document.
    GovernanceChanged,
    /// A signed approval, withdrawal or check statement.
    GitEvidence,
    /// A Git change landed: the receipt of the decision that let it.
    GitLanded,
    /// A Git change was refused at the land: the receipt of that decision.
    GitRefused,
    /// A governed repository's content or state was shown to a key.
    GitRead,
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
            Self::GitEvidence => "git_evidence",
            Self::GovernanceChanged => "governance_changed",
            Self::GitLanded => "git_landed",
            Self::GitRefused => "git_refused",
            Self::GitRead => "git_read",
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
        Self::GitEvidence,
        Self::GovernanceChanged,
        Self::GitLanded,
        Self::GitRefused,
        Self::GitRead,
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
}
