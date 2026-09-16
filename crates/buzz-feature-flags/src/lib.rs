#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Typed boolean feature flags for Buzz server components.

use buzz_core::{CommunityId, PublicKey};

#[cfg(feature = "launchdarkly")]
/// LaunchDarkly-backed evaluator adapter.
pub mod launchdarkly;

/// Typed boolean feature definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BooleanFlag {
    key: &'static str,
    default: bool,
}

impl BooleanFlag {
    /// Construct a typed boolean flag from its stable key and declared default.
    pub const fn new(key: &'static str, default: bool) -> Self {
        Self { key, default }
    }

    /// Stable key for this flag.
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Declared default used when an evaluator is unavailable or cannot return a value.
    pub const fn default(self) -> bool {
        self.default
    }
}

/// Stable targeting context for one Buzz community and an optional actor.
///
/// Community is always required because the same Nostr identity can participate
/// in multiple isolated Buzz communities. Community-only contexts support
/// background and system evaluations that have no actor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationContext {
    community: CommunityId,
    actor_pubkey: Option<PublicKey>,
}

impl EvaluationContext {
    /// Construct a community-only evaluation context.
    pub const fn for_community(community: CommunityId) -> Self {
        Self {
            community,
            actor_pubkey: None,
        }
    }

    /// Construct an evaluation context for an actor within one community.
    pub const fn for_actor(community: CommunityId, actor_pubkey: PublicKey) -> Self {
        Self {
            community,
            actor_pubkey: Some(actor_pubkey),
        }
    }

    /// Community that owns this evaluation.
    pub const fn community(&self) -> CommunityId {
        self.community
    }

    /// Optional stable actor key for actor-targeted evaluation.
    pub const fn actor_pubkey(&self) -> Option<&PublicKey> {
        self.actor_pubkey.as_ref()
    }
}

/// Evaluates typed boolean feature flags.
pub trait BooleanFlagEvaluator: Send + Sync {
    /// Resolve a boolean flag for one Buzz evaluation context.
    ///
    /// Implementations must return [`BooleanFlag::default`] whenever no valid
    /// boolean value is available, including provider absence or failure.
    fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool;
}

/// Static evaluator that should return each flag's declared default.
#[derive(Debug, Default, Clone, Copy)]
pub struct StaticEvaluator;

impl BooleanFlagEvaluator for StaticEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
        flag.default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;

    fn community(id: &str) -> CommunityId {
        CommunityId::from_uuid(id.parse().expect("valid community UUID"))
    }

    fn actor() -> PublicKey {
        PublicKey::from_hex("c4f0623bdc8c4f7ecab9f7457f501f3e8f4efcf8f8f6ef6f4d76f42f5bb6f2cb")
            .expect("valid pubkey")
    }

    #[test]
    fn static_evaluator_returns_declared_defaults_for_community() {
        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

        assert!(StaticEvaluator.evaluate_bool(BooleanFlag::new("enabled", true), &context));
        assert!(!StaticEvaluator.evaluate_bool(BooleanFlag::new("disabled", false), &context));
    }

    #[test]
    fn non_launchdarkly_evaluator_can_target_community_through_public_trait() {
        struct CommunityEvaluator {
            enabled_community: CommunityId,
        }

        impl BooleanFlagEvaluator for CommunityEvaluator {
            fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool {
                if context.community() == self.enabled_community {
                    true
                } else {
                    flag.default()
                }
            }
        }

        let enabled_community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let other_community = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let evaluator: &dyn BooleanFlagEvaluator = &CommunityEvaluator { enabled_community };
        let flag = BooleanFlag::new("community-targeted", false);

        assert!(evaluator.evaluate_bool(flag, &EvaluationContext::for_community(enabled_community)));
        assert!(!evaluator.evaluate_bool(flag, &EvaluationContext::for_community(other_community)));
    }

    #[test]
    fn same_actor_in_two_communities_is_distinguishable() {
        let actor = actor();
        let community_a = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let community_b = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");

        let context_a = EvaluationContext::for_actor(community_a, actor);
        let context_b = EvaluationContext::for_actor(community_b, actor);

        assert_eq!(context_a.actor_pubkey(), context_b.actor_pubkey());
        assert_ne!(context_a.community(), context_b.community());
    }
}
