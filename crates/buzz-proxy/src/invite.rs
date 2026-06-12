//! Invite token management for guest authentication via NIP-42 AUTH tags.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::ProxyError;

/// An invite token granting a guest access to one or more channels.
#[derive(Debug, Clone)]
pub struct InviteToken {
    /// The raw token string presented by the guest during NIP-42 AUTH.
    pub token: String,
    /// Channels this token grants access to.
    pub channel_ids: Vec<Uuid>,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
    /// Maximum number of times the token may be used.
    pub max_uses: u32,
    /// Number of times the token has been used so far.
    pub uses: u32,
}

impl InviteToken {
    /// Create a new invite token with zero uses.
    pub fn new(
        token: impl Into<String>,
        channel_ids: Vec<Uuid>,
        expires_at: DateTime<Utc>,
        max_uses: u32,
    ) -> Self {
        Self {
            token: token.into(),
            channel_ids,
            expires_at,
            max_uses,
            uses: 0,
        }
    }

    /// Returns `Ok(())` if the token is not expired and has remaining uses.
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), ProxyError> {
        if now >= self.expires_at {
            return Err(ProxyError::InviteExpired);
        }
        if self.uses >= self.max_uses {
            return Err(ProxyError::InviteExhausted);
        }
        Ok(())
    }

    /// Returns `true` if the token passes validation at `now`.
    pub fn is_valid(&self, now: DateTime<Utc>) -> bool {
        self.validate(now).is_ok()
    }

    /// Increments the use counter by one (saturating).
    pub fn consume(&mut self) {
        self.uses = self.uses.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn future(secs: i64) -> DateTime<Utc> {
        Utc::now() + Duration::seconds(secs)
    }

    fn past(secs: i64) -> DateTime<Utc> {
        Utc::now() - Duration::seconds(secs)
    }

    #[test]
    fn test_invite_token_validation() {
        let token = InviteToken::new("tok-valid", vec![], future(3600), 5);
        assert!(token.validate(Utc::now()).is_ok());
        assert!(token.is_valid(Utc::now()));
    }

    #[test]
    fn test_invite_token_expired() {
        let token = InviteToken::new("tok-expired", vec![], past(1), 5);
        let err = token.validate(Utc::now()).unwrap_err();
        assert!(matches!(err, ProxyError::InviteExpired));
        assert!(!token.is_valid(Utc::now()));
    }

    #[test]
    fn test_invite_token_exhausted() {
        let mut token = InviteToken::new("tok-used-up", vec![], future(3600), 2);
        token.uses = 2;
        let err = token.validate(Utc::now()).unwrap_err();
        assert!(matches!(err, ProxyError::InviteExhausted));
        assert!(!token.is_valid(Utc::now()));
    }

    #[test]
    fn test_invite_token_consume_increments_uses() {
        let mut token = InviteToken::new("tok-consume", vec![], future(3600), 3);
        assert_eq!(token.uses, 0);
        token.consume();
        assert_eq!(token.uses, 1);
        token.consume();
        assert_eq!(token.uses, 2);
        // Still valid (uses < max_uses)
        assert!(token.is_valid(Utc::now()));
        token.consume();
        assert!(!token.is_valid(Utc::now()));
    }

    #[test]
    fn test_invite_token_consume_saturates_at_max() {
        let mut token = InviteToken::new("tok-sat", vec![], future(3600), 1);
        // Consume beyond max_uses — should not overflow
        token.uses = u32::MAX;
        token.consume(); // saturating_add should not panic
        assert_eq!(token.uses, u32::MAX);
    }
}
