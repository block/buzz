use buzz_auth::{AgentBudgetScope, AgentEventBudget, AgentEventVerdict, LimitType, RateLimiter};
use buzz_core::TenantContext;
use nostr::PublicKey;

// Desktop startup establishes several independent live subscriptions at once.
// Preserve the configured average rate while allowing that bounded burst. This
// is still a fixed-window limiter, so a Redis-backed token bucket would be a
// better long-term fit for smoother refill behavior.
const WS_BURST_WINDOW_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionError {
    Exceeded { reset_in_secs: u64 },
    Unavailable,
}

pub(crate) async fn check_principal<L: RateLimiter>(
    limiter: &L,
    tenant: &TenantContext,
    pubkey: &PublicKey,
    limit_type: LimitType,
    window_secs: u64,
    limit: u64,
) -> Result<(), AdmissionError> {
    match limiter
        .check_and_increment(tenant, pubkey, limit_type, window_secs, limit)
        .await
    {
        Ok(result) if result.allowed => Ok(()),
        Ok(result) => Err(AdmissionError::Exceeded {
            reset_in_secs: result.reset_in_secs,
        }),
        Err(error) => {
            tracing::warn!(error = %error, "shared rate-limit admission unavailable");
            Err(AdmissionError::Unavailable)
        }
    }
}

/// The channel whose agent budget `event` is charged to: the channel ingest
/// scopes it to by its own `h` tag, or `None`.
///
/// Admission runs before validation, so this is the channel the event claims.
/// An event whose channel ingest derives some other way (a deletion or
/// reaction without an `h` tag, for example) is charged to the owner's
/// no-channel budget. Claiming a channel the agent cannot write to only fails
/// later, at ingest. In every case the owner's overall budget is charged too.
pub(crate) fn agent_budget_channel(event: &nostr::Event) -> Option<uuid::Uuid> {
    let kind = buzz_core::kind::event_kind_u32(event);
    if crate::handlers::ingest::is_global_only_kind(kind) {
        return None;
    }
    crate::handlers::ingest::extract_channel_id(event)
}

/// Admits a non-ephemeral event published by an agent of `owner` against that
/// owner's channel and overall budgets. Returns the verdict with the metrics
/// bucket it belongs to.
pub(crate) async fn check_agent_event<L: RateLimiter>(
    limiter: &L,
    tenant: &TenantContext,
    owner: &PublicKey,
    event: &nostr::Event,
    limits: &buzz_auth::RateLimitConfig,
) -> (Result<(), AdmissionError>, &'static str) {
    let budget = AgentEventBudget {
        window_secs: 60,
        channel_limit: limits.agent_channel_events_per_min,
        owner_limit: limits.agent_owner_events_per_min,
    };
    match limiter
        .check_agent_event(tenant, owner, agent_budget_channel(event), budget)
        .await
    {
        Ok(AgentEventVerdict::Admitted) => (Ok(()), AGENT_EVENTS_BUCKET),
        Ok(AgentEventVerdict::Exceeded {
            scope,
            reset_in_secs,
        }) => (
            Err(AdmissionError::Exceeded { reset_in_secs }),
            match scope {
                AgentBudgetScope::Channel => "agent_channel_events",
                AgentBudgetScope::Owner => "agent_owner_events",
            },
        ),
        Err(error) => {
            tracing::warn!(error = %error, "shared agent event admission unavailable");
            (Err(AdmissionError::Unavailable), AGENT_EVENTS_BUCKET)
        }
    }
}

/// Metrics bucket for agent event admission that no single budget refused.
const AGENT_EVENTS_BUCKET: &str = "agent_events";

pub(crate) fn ws_admission_budget(per_second_limit: u64) -> (u64, u64) {
    (
        WS_BURST_WINDOW_SECS,
        per_second_limit.saturating_mul(WS_BURST_WINDOW_SECS),
    )
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use buzz_auth::{AuthError, RateLimitResult, RateLimiter};
    use buzz_core::CommunityId;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

    use super::*;

    enum StubOutcome {
        Denied,
        Failed,
    }

    struct StubLimiter {
        outcome: StubOutcome,
        calls: AtomicUsize,
    }

    impl RateLimiter for StubLimiter {
        async fn check_and_increment(
            &self,
            _ctx: &TenantContext,
            _pubkey: &PublicKey,
            _limit_type: LimitType,
            _window_secs: u64,
            _limit: u64,
        ) -> Result<RateLimitResult, AuthError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.outcome {
                StubOutcome::Denied => Ok(RateLimitResult::denied(11, 10, 1)),
                StubOutcome::Failed => Err(AuthError::Internal("redis unavailable".to_owned())),
            }
        }

        async fn check_agent_event(
            &self,
            _ctx: &TenantContext,
            _owner: &PublicKey,
            _channel: Option<Uuid>,
            _budget: AgentEventBudget,
        ) -> Result<AgentEventVerdict, AuthError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.outcome {
                StubOutcome::Denied => Ok(AgentEventVerdict::Exceeded {
                    scope: AgentBudgetScope::Owner,
                    reset_in_secs: 1,
                }),
                StubOutcome::Failed => Err(AuthError::Internal("redis unavailable".to_owned())),
            }
        }

        async fn check_ip_connection(
            &self,
            _ip: &IpAddr,
            _window_secs: u64,
            _limit: u64,
        ) -> Result<RateLimitResult, AuthError> {
            match self.outcome {
                StubOutcome::Denied => Ok(RateLimitResult::denied(11, 10, 1)),
                StubOutcome::Failed => Err(AuthError::Internal("redis unavailable".to_owned())),
            }
        }
    }

    fn tenant() -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            "relay.example.com",
        )
    }

    #[test]
    fn websocket_budget_preserves_rate_with_a_bounded_burst() {
        assert_eq!(ws_admission_budget(10), (5, 50));
    }

    #[test]
    fn websocket_budget_saturates_on_overflow() {
        assert_eq!(ws_admission_budget(u64::MAX), (5, u64::MAX));
    }

    #[tokio::test]
    async fn denied_shared_counter_rejects_admission() {
        let limiter = StubLimiter {
            outcome: StubOutcome::Denied,
            calls: AtomicUsize::new(0),
        };
        let keys = Keys::generate();

        let result = check_principal(
            &limiter,
            &tenant(),
            &keys.public_key(),
            LimitType::WsEvents,
            1,
            10,
        )
        .await;

        assert_eq!(result, Err(AdmissionError::Exceeded { reset_in_secs: 1 }));
        assert_eq!(limiter.calls.load(Ordering::Relaxed), 1);
    }

    /// The budgeted channel is the one ingest scopes the event to by its own
    /// `h` tag; kinds ingest never channel-scopes have none.
    #[test]
    fn agent_budget_channel_follows_ingest_scope() {
        let channel = Uuid::new_v4();
        let event = |kind: u16, tags: &[[&str; 2]]| {
            tags.iter()
                .fold(EventBuilder::new(Kind::from(kind), ""), |builder, tag| {
                    builder.tag(Tag::parse(*tag).unwrap())
                })
                .sign_with_keys(&Keys::generate())
                .unwrap()
        };
        let h = ["h", &channel.to_string()];
        let artifact = buzz_core::kind::KIND_ARTIFACT as u16;
        assert_eq!(agent_budget_channel(&event(9, &[h])), Some(channel));
        assert_eq!(agent_budget_channel(&event(artifact, &[h])), Some(channel));
        assert_eq!(agent_budget_channel(&event(9, &[])), None);
        let target = "0".repeat(64);
        assert_eq!(agent_budget_channel(&event(5, &[["e", &target]])), None);
        assert_eq!(agent_budget_channel(&event(0, &[h])), None, "profile");
        assert_eq!(
            agent_budget_channel(&event(9, &[["h", "not-a-uuid"], h])),
            Some(channel),
            "the first valid channel, as ingest reads it"
        );
    }

    #[tokio::test]
    async fn agent_event_verdicts_map_to_admission_and_bucket() {
        let owner = Keys::generate().public_key();
        let event = EventBuilder::new(Kind::from(9), "")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let limits = buzz_auth::RateLimitConfig::default();
        let check = |outcome| async {
            let limiter = StubLimiter {
                outcome,
                calls: AtomicUsize::new(0),
            };
            let verdict = check_agent_event(&limiter, &tenant(), &owner, &event, &limits).await;
            assert_eq!(limiter.calls.load(Ordering::Relaxed), 1);
            verdict
        };

        assert_eq!(
            check(StubOutcome::Denied).await,
            (
                Err(AdmissionError::Exceeded { reset_in_secs: 1 }),
                "agent_owner_events"
            )
        );
        assert_eq!(
            check(StubOutcome::Failed).await,
            (Err(AdmissionError::Unavailable), "agent_events")
        );
        assert_eq!(
            check_agent_event(
                &buzz_auth::AlwaysAllowRateLimiter,
                &tenant(),
                &owner,
                &event,
                &limits
            )
            .await,
            (Ok(()), "agent_events")
        );
    }

    #[tokio::test]
    async fn shared_counter_failure_rejects_admission() {
        let limiter = StubLimiter {
            outcome: StubOutcome::Failed,
            calls: AtomicUsize::new(0),
        };
        let keys = Keys::generate();

        let result = check_principal(
            &limiter,
            &tenant(),
            &keys.public_key(),
            LimitType::ApiCalls,
            60,
            300,
        )
        .await;

        assert_eq!(result, Err(AdmissionError::Unavailable));
        assert_eq!(limiter.calls.load(Ordering::Relaxed), 1);
    }
}
