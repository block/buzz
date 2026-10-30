use std::sync::Arc;

use buzz_core::{CommunityId, PublicKey};
use buzz_feature_flags::{
    BooleanFlag, EnvironmentDiagnostic, EnvironmentEvaluator, EvaluationContext, FlagEvaluator,
    IntegerFlag,
};

const ENV_NAMESPACE: &str = "BUZZ_FEATURE_FLAG_";

fn community(id: &str) -> CommunityId {
    CommunityId::from_uuid(id.parse().expect("valid community UUID"))
}

fn actor(hex: &str) -> PublicKey {
    PublicKey::from_hex(hex).expect("valid pubkey")
}

#[test]
fn parses_booleans_case_insensitive_with_trim() {
    let evaluator = EnvironmentEvaluator::from_pairs([
        (
            format!("{ENV_NAMESPACE}RELAY_BOOL_TRUE"),
            "  TrUe \t".to_owned(),
        ),
        (
            format!("{ENV_NAMESPACE}RELAY_BOOL_FALSE"),
            "\n false  ".to_owned(),
        ),
    ]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.bool.true", false), &context));
    assert!(!evaluator.evaluate_bool(BooleanFlag::new("relay.bool.false", true), &context));
}

#[test]
fn parses_signed_negative_integers_with_trim() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}DB_EVENTS_QUERY_VERSION"),
        "  -17 ".to_owned(),
    )]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("db.events.query-version", 1), &context),
        -17
    );
}

#[test]
fn missing_flag_values_fall_back_to_declared_defaults() {
    let evaluator = EnvironmentEvaluator::from_pairs([] as [(String, String); 0]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.bool.missing", true), &context));
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("relay.int.missing", -9), &context),
        -9
    );
}

#[test]
fn invalid_and_empty_values_fall_back_to_declared_defaults() {
    let evaluator = EnvironmentEvaluator::from_pairs([
        (
            format!("{ENV_NAMESPACE}RELAY_BOOL_INVALID"),
            "maybe".to_owned(),
        ),
        (format!("{ENV_NAMESPACE}RELAY_BOOL_EMPTY"), "   ".to_owned()),
        (
            format!("{ENV_NAMESPACE}RELAY_INT_INVALID"),
            "12.5".to_owned(),
        ),
    ]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(!evaluator.evaluate_bool(BooleanFlag::new("relay.bool.invalid", false), &context));
    assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.bool.empty", true), &context));
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("relay.int.invalid", 88), &context),
        88
    );
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("!!!", -4), &context),
        -4,
    );
}

#[test]
fn evaluation_context_is_ignored_for_process_wide_values() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}RELAY_PROCESS_WIDE"),
        "true".to_owned(),
    )]);
    let community_a = community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let community_b = community("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    let actor_a = actor("c4f0623bdc8c4f7ecab9f7457f501f3e8f4efcf8f8f6ef6f4d76f42f5bb6f2cb");
    let actor_b = actor("f1e2d3c4b5a697887766554433221100ffeeddccbbaa99887766554433221100");

    assert!(evaluator.evaluate_bool(
        BooleanFlag::new("relay.process-wide", false),
        &EvaluationContext::for_actor(community_a, actor_a),
    ));
    assert!(evaluator.evaluate_bool(
        BooleanFlag::new("relay.process-wide", false),
        &EvaluationContext::for_actor(community_b, actor_b),
    ));
    assert!(evaluator.evaluate_bool(
        BooleanFlag::new("relay.process-wide", false),
        &EvaluationContext::for_community(community_a),
    ));
}

#[test]
fn normalization_replaces_non_alnum_runs_and_trims_separators() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}RELAY_FOO_BAR_BAZ_9"),
        "true".to_owned(),
    )]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(
        BooleanFlag::new("___relay..foo---bar***baz??9___", false),
        &context,
    ));
}

#[test]
fn colliding_flag_keys_share_normalized_environment_name() {
    let evaluator =
        EnvironmentEvaluator::from_pairs([(format!("{ENV_NAMESPACE}A_B"), "true".to_owned())]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(BooleanFlag::new("a-b", false), &context));
    assert!(evaluator.evaluate_bool(BooleanFlag::new("a_b", false), &context));
}

#[test]
fn snapshot_is_immutable_after_construction() {
    let mut source = vec![(format!("{ENV_NAMESPACE}SNAPSHOT_BOOL"), "true".to_owned())];

    let evaluator = EnvironmentEvaluator::from_pairs(
        source
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );

    source[0].1 = "false".to_owned();
    source.push((format!("{ENV_NAMESPACE}SNAPSHOT_INT"), "99".to_owned()));

    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    assert!(evaluator.evaluate_bool(BooleanFlag::new("snapshot.bool", false), &context));
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("snapshot.int", -1), &context),
        -1
    );
}

#[test]
fn debug_does_not_expose_environment_contents() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}SECRET_FLAG"),
        "super-secret-value".to_owned(),
    )]);

    let debug = format!("{evaluator:?}");
    assert!(!debug.contains("super-secret-value"));
    assert!(!debug.contains("SECRET_FLAG"));
}

#[test]
fn malformed_present_values_default_and_diagnose_once_per_key_and_type() {
    let evaluator = EnvironmentEvaluator::from_pairs([
        (
            format!("{ENV_NAMESPACE}RELAY_INVALID_BOOL"),
            "raw-secret-bool".to_owned(),
        ),
        (
            format!("{ENV_NAMESPACE}RELAY_INVALID_INT"),
            "raw-secret-int".to_owned(),
        ),
        ("DATABASE_URL".to_owned(), "unrelated-secret".to_owned()),
    ]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    let bool_flag = BooleanFlag::new("relay.invalid-bool", true);
    let int_flag = IntegerFlag::new("relay.invalid-int", -12);

    assert!(evaluator.evaluate_bool(bool_flag, &context));
    assert!(evaluator.evaluate_bool(bool_flag, &context));
    assert_eq!(evaluator.evaluate_int(int_flag, &context), -12);
    assert_eq!(evaluator.evaluate_int(int_flag, &context), -12);

    let diagnostics = evaluator.take_diagnostics();
    assert_eq!(
        diagnostics,
        vec![
            EnvironmentDiagnostic::InvalidBooleanValue {
                flag_key: "relay.invalid-bool",
            },
            EnvironmentDiagnostic::InvalidIntegerValue {
                flag_key: "relay.invalid-int",
            },
        ]
    );

    let rendered = format!("{diagnostics:?}");
    assert!(!rendered.contains("raw-secret-bool"));
    assert!(!rendered.contains("raw-secret-int"));
    assert!(!rendered.contains("unrelated-secret"));
    assert!(!rendered.contains("DATABASE_URL"));

    assert!(evaluator.evaluate_bool(bool_flag, &context));
    assert_eq!(evaluator.evaluate_int(int_flag, &context), -12);
    assert!(evaluator.take_diagnostics().is_empty());
}

#[test]
fn trait_object_evaluation_reports_through_retained_concrete_owner() {
    let owner = Arc::new(EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}RELAY_INVALID"),
        "not-a-boolean".to_owned(),
    )]));
    let evaluator: Arc<dyn FlagEvaluator> = owner.clone();
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.invalid", true), &context));
    assert_eq!(
        owner.take_diagnostics(),
        vec![EnvironmentDiagnostic::InvalidBooleanValue {
            flag_key: "relay.invalid",
        }]
    );
}

#[test]
fn same_flag_key_reports_distinct_boolean_and_integer_diagnostics() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}RELAY_SHARED_KEY"),
        "invalid-for-both-types".to_owned(),
    )]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(!evaluator.evaluate_bool(BooleanFlag::new("relay.shared-key", false), &context));
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("relay.shared-key", -4), &context),
        -4
    );
    assert_eq!(
        evaluator.take_diagnostics(),
        vec![
            EnvironmentDiagnostic::InvalidBooleanValue {
                flag_key: "relay.shared-key",
            },
            EnvironmentDiagnostic::InvalidIntegerValue {
                flag_key: "relay.shared-key",
            },
        ]
    );
}

#[test]
fn cloned_evaluators_share_dedup_before_and_after_drain() {
    let evaluator = EnvironmentEvaluator::from_pairs([(
        format!("{ENV_NAMESPACE}RELAY_CLONED"),
        "not-a-boolean".to_owned(),
    )]);
    let clone = evaluator.clone();
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    let flag = BooleanFlag::new("relay.cloned", false);

    assert!(!evaluator.evaluate_bool(flag, &context));
    assert!(!clone.evaluate_bool(flag, &context));
    assert_eq!(
        clone.take_diagnostics(),
        vec![EnvironmentDiagnostic::InvalidBooleanValue {
            flag_key: "relay.cloned",
        }]
    );
    assert!(evaluator.take_diagnostics().is_empty());

    assert!(!evaluator.evaluate_bool(flag, &context));
    assert!(!clone.evaluate_bool(flag, &context));
    assert!(evaluator.take_diagnostics().is_empty());
    assert!(clone.take_diagnostics().is_empty());
}

#[test]
fn missing_values_default_without_diagnostics() {
    let evaluator = EnvironmentEvaluator::from_pairs([] as [(String, String); 0]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

    assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.missing-bool", true), &context));
    assert_eq!(
        evaluator.evaluate_int(IntegerFlag::new("relay.missing-int", -4), &context),
        -4
    );
    assert!(evaluator.take_diagnostics().is_empty());
}

#[cfg(unix)]
#[test]
fn namespaced_non_unicode_value_defaults_and_diagnoses_once() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let evaluator = EnvironmentEvaluator::from_pairs([(
        OsString::from(format!("{ENV_NAMESPACE}RELAY_NON_UNICODE")),
        OsString::from_vec(vec![0x80]),
    )]);
    let context =
        EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    let flag = BooleanFlag::new("relay.non-unicode", false);

    assert!(!evaluator.evaluate_bool(flag, &context));
    assert!(!evaluator.evaluate_bool(flag, &context));
    assert_eq!(
        evaluator.take_diagnostics(),
        vec![EnvironmentDiagnostic::InvalidBooleanValue {
            flag_key: "relay.non-unicode",
        }]
    );
    assert!(!evaluator.evaluate_bool(flag, &context));
    assert!(evaluator.take_diagnostics().is_empty());
}
