use buzz_auth::{AuthError, AuthenticatedClientPeer, LimitType, RateLimiter};
use buzz_core::TenantContext;
use buzz_pubsub::rate_limiter::{
    ClientStatusAdmissionLimits, ClientStatusAdmissionResult, RedisRateLimiter,
};
use nostr::PublicKey;

use crate::authorization_runtime::ClientStatusAdmissionPolicy;

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

pub(crate) trait ClientStatusAdmissionLimiter: Send + Sync {
    fn check_client_status_admission(
        &self,
        tenant: &TenantContext,
        actor: &PublicKey,
        peer: &AuthenticatedClientPeer,
        limits: ClientStatusAdmissionLimits,
    ) -> impl std::future::Future<Output = Result<ClientStatusAdmissionResult, AuthError>> + Send;
}

impl ClientStatusAdmissionLimiter for RedisRateLimiter {
    async fn check_client_status_admission(
        &self,
        tenant: &TenantContext,
        actor: &PublicKey,
        peer: &AuthenticatedClientPeer,
        limits: ClientStatusAdmissionLimits,
    ) -> Result<ClientStatusAdmissionResult, AuthError> {
        RedisRateLimiter::check_client_status_admission(self, tenant, actor, peer, limits).await
    }
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

/// Admit optional client status without changing the completed AUTH decision.
///
/// A denial or Redis failure withholds only the presentation. The caller must
/// return before any status evidence or durable audit allocation occurs.
pub(crate) async fn check_client_status_presentation<L: ClientStatusAdmissionLimiter>(
    limiter: &L,
    tenant: &TenantContext,
    actor: &PublicKey,
    peer: &AuthenticatedClientPeer,
    policy: ClientStatusAdmissionPolicy,
) -> Result<(), AdmissionError> {
    match limiter
        .check_client_status_admission(
            tenant,
            actor,
            peer,
            ClientStatusAdmissionLimits::new(
                policy.max_presentations_per_domain(),
                policy.max_presentations_per_actor(),
                policy.max_presentations_per_peer(),
            ),
        )
        .await
    {
        Ok(result) if result.allowed => Ok(()),
        Ok(result) => Err(AdmissionError::Exceeded {
            reset_in_secs: result.reset_in_secs,
        }),
        Err(error) => {
            tracing::warn!(error = %error, "client status admission unavailable");
            Err(AdmissionError::Unavailable)
        }
    }
}

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
    use std::sync::Mutex;

    use buzz_auth::{AuthorizationEventCapacityPolicy, RateLimitResult};
    use buzz_core::CommunityId;
    use nostr::Keys;
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

    struct StubStatusLimiter {
        allowed: bool,
        seen_peers: Mutex<Vec<[u8; 32]>>,
    }

    impl ClientStatusAdmissionLimiter for StubStatusLimiter {
        async fn check_client_status_admission(
            &self,
            _tenant: &TenantContext,
            _actor: &PublicKey,
            peer: &AuthenticatedClientPeer,
            _limits: ClientStatusAdmissionLimits,
        ) -> Result<ClientStatusAdmissionResult, AuthError> {
            self.seen_peers
                .lock()
                .map_err(|_| AuthError::Internal("status test lock unavailable".to_owned()))?
                .push(*peer.admission_key());
            Ok(ClientStatusAdmissionResult {
                allowed: self.allowed,
                reset_in_secs: 11,
            })
        }
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

    fn status_policy() -> ClientStatusAdmissionPolicy {
        ClientStatusAdmissionPolicy::new(
            AuthorizationEventCapacityPolicy::new(10, 1 << 20, 16 << 10).expect("capacity"),
            10,
            2,
            3,
        )
        .expect("status policy")
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

    #[tokio::test]
    async fn status_consumer_preserves_authenticated_clients_behind_proxy_fan_in() {
        let limiter = StubStatusLimiter {
            allowed: true,
            seen_peers: Mutex::new(Vec::new()),
        };
        let actor_a = Keys::generate().public_key();
        let actor_b = Keys::generate().public_key();
        let peer_a = AuthenticatedClientPeer::for_test([0x11; 32]);
        let peer_b = AuthenticatedClientPeer::for_test([0x22; 32]);

        assert_eq!(
            check_client_status_presentation(
                &limiter,
                &tenant(),
                &actor_a,
                &peer_a,
                status_policy(),
            )
            .await,
            Ok(())
        );
        assert_eq!(
            check_client_status_presentation(
                &limiter,
                &tenant(),
                &actor_b,
                &peer_b,
                status_policy(),
            )
            .await,
            Ok(())
        );
        assert_eq!(
            *limiter.seen_peers.lock().expect("status peer log"),
            vec![[0x11; 32], [0x22; 32]]
        );
    }

    #[tokio::test]
    async fn status_denial_withholds_presentation() {
        let limiter = StubStatusLimiter {
            allowed: false,
            seen_peers: Mutex::new(Vec::new()),
        };
        let actor = Keys::generate().public_key();
        let peer = AuthenticatedClientPeer::for_test([0x33; 32]);

        assert_eq!(
            check_client_status_presentation(&limiter, &tenant(), &actor, &peer, status_policy(),)
                .await,
            Err(AdmissionError::Exceeded { reset_in_secs: 11 })
        );
    }
}
