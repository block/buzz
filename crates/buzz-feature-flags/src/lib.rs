#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Typed boolean and integer feature flags for Buzz server components.

use buzz_core::{CommunityId, PublicKey};

pub mod environment;
pub use environment::{EnvironmentDiagnostic, EnvironmentEvaluator};

pub mod flags;
pub use flags::{BooleanFlag, IntegerFlag};

#[cfg(feature = "launchdarkly")]
/// LaunchDarkly-backed evaluator adapter.
pub mod launchdarkly;

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

/// Evaluates typed feature flags.
pub trait FlagEvaluator: Send + Sync {
    /// Resolve a boolean flag for one Buzz evaluation context.
    ///
    /// Implementations must return [`BooleanFlag::default`] whenever no valid
    /// boolean value is available, including provider absence or failure.
    fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool;

    /// Resolve an integer flag for one Buzz evaluation context.
    ///
    /// Implementations must return [`IntegerFlag::default`] whenever no valid
    /// integer value is available, including provider absence or failure.
    fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64;
}

/// Static evaluator that should return each flag's declared default.
#[derive(Debug, Default, Clone, Copy)]
pub struct StaticEvaluator;

impl FlagEvaluator for StaticEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
        flag.default()
    }

    fn evaluate_int(&self, flag: IntegerFlag, _context: &EvaluationContext) -> i64 {
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
    fn static_evaluator_returns_declared_integer_defaults_for_community() {
        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

        assert_eq!(
            StaticEvaluator.evaluate_int(IntegerFlag::new("positive", 7), &context),
            7
        );
        assert_eq!(
            StaticEvaluator.evaluate_int(IntegerFlag::new("negative", -9), &context),
            -9
        );
    }

    #[test]
    fn non_launchdarkly_evaluator_can_target_community_through_public_trait() {
        struct CommunityEvaluator {
            enabled_community: CommunityId,
        }

        impl FlagEvaluator for CommunityEvaluator {
            fn evaluate_bool(&self, flag: BooleanFlag, context: &EvaluationContext) -> bool {
                if context.community() == self.enabled_community {
                    true
                } else {
                    flag.default()
                }
            }

            fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64 {
                if context.community() == self.enabled_community {
                    41
                } else {
                    flag.default()
                }
            }
        }

        let enabled_community = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let other_community = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let evaluator: &dyn FlagEvaluator = &CommunityEvaluator { enabled_community };

        assert!(evaluator.evaluate_bool(
            BooleanFlag::new("community-targeted-bool", false),
            &EvaluationContext::for_community(enabled_community),
        ));
        assert!(!evaluator.evaluate_bool(
            BooleanFlag::new("community-targeted-bool", false),
            &EvaluationContext::for_community(other_community),
        ));

        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("community-targeted-int", -1),
                &EvaluationContext::for_community(enabled_community),
            ),
            41
        );
        assert_eq!(
            evaluator.evaluate_int(
                IntegerFlag::new("community-targeted-int", -1),
                &EvaluationContext::for_community(other_community),
            ),
            -1
        );
    }

    #[test]
    fn same_actor_in_two_communities_is_distinguishable_for_integer_evaluation() {
        struct CommunityRolloutEvaluator {
            enabled_community: CommunityId,
            expected_actor: PublicKey,
        }

        impl FlagEvaluator for CommunityRolloutEvaluator {
            fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
                flag.default()
            }

            fn evaluate_int(&self, flag: IntegerFlag, context: &EvaluationContext) -> i64 {
                if context.community() == self.enabled_community
                    && context.actor_pubkey() == Some(&self.expected_actor)
                {
                    99
                } else {
                    flag.default()
                }
            }
        }

        let actor = actor();
        let community_a = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let community_b = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        let evaluator: &dyn FlagEvaluator = &CommunityRolloutEvaluator {
            enabled_community: community_a,
            expected_actor: actor,
        };
        let flag = IntegerFlag::new("community-rollout", -11);

        assert_eq!(
            evaluator.evaluate_int(flag, &EvaluationContext::for_actor(community_a, actor)),
            99
        );
        assert_eq!(
            evaluator.evaluate_int(flag, &EvaluationContext::for_actor(community_b, actor)),
            -11
        );
    }
}
