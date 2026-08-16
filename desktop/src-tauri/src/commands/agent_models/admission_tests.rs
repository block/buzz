use super::admission::{
    evaluate_offline_admission, OfflineModelAdmissionState, OfflineModelPolicy,
};
use crate::managed_agents::AgentModelInfo;

fn model(
    id: &str,
    instance_id: &str,
    context_length: u64,
    parallel: u64,
    vision: bool,
    tools: bool,
) -> AgentModelInfo {
    AgentModelInfo {
        id: id.to_string(),
        name: None,
        description: None,
        loaded_instance_ids: vec![instance_id.to_string()],
        is_loaded: true,
        max_context_length: Some(262_144),
        capabilities: Some(serde_json::json!({
            "vision": vision,
            "trained_for_tool_use": tools,
        })),
        loaded_context_length: Some(context_length),
        loaded_parallelism: Some(parallel),
    }
}

#[test]
fn admits_only_the_exact_qualified_64k_runtime() {
    let admission = evaluate_offline_admission(
        &OfflineModelPolicy::command_adviser(),
        &[model(
            "google/gemma-4-26b-a4b",
            "gemma4-26b-official",
            65_536,
            1,
            true,
            true,
        )],
    );

    assert_eq!(admission.state, OfflineModelAdmissionState::Ready);
    assert_eq!(admission.admitted_tier.as_deref(), Some("64k"));
    let runtime = admission.runtime.expect("runtime identity");
    assert_eq!(runtime.model_id, "google/gemma-4-26b-a4b");
    assert_eq!(runtime.instance_id, "gemma4-26b-official");
    assert_eq!(runtime.context_length, 65_536);
    assert_eq!(runtime.parallel, 1);
}

#[test]
fn rejects_unloaded_wrong_or_unqualified_runtime() {
    let policy = OfflineModelPolicy::command_adviser();
    assert_eq!(
        evaluate_offline_admission(&policy, &[]).state,
        OfflineModelAdmissionState::NotLoaded
    );
    assert_eq!(
        evaluate_offline_admission(
            &policy,
            &[model("qwen/qwen3.6-27b", "qwen", 65_536, 1, true, true)]
        )
        .state,
        OfflineModelAdmissionState::WrongModel
    );
    assert_eq!(
        evaluate_offline_admission(
            &policy,
            &[model(
                "google/gemma-4-26b-a4b",
                "gemma4-26b-official",
                32_768,
                1,
                true,
                true,
            )]
        )
        .state,
        OfflineModelAdmissionState::InsufficientContext
    );
    assert_eq!(
        evaluate_offline_admission(
            &policy,
            &[model(
                "google/gemma-4-26b-a4b",
                "gemma4-26b-official",
                65_536,
                2,
                true,
                true,
            )]
        )
        .state,
        OfflineModelAdmissionState::InvalidRuntime
    );
    assert_eq!(
        evaluate_offline_admission(
            &policy,
            &[model(
                "google/gemma-4-26b-a4b",
                "gemma4-26b-official",
                65_536,
                1,
                false,
                true,
            )]
        )
        .state,
        OfflineModelAdmissionState::MissingCapability
    );
}
