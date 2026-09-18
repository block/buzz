//! UC route selection must change only the route, never effort or model identity.
use buzz_agent::model_capabilities::{resolve, DatabricksV2Route};

#[test]
fn gpt_fqn_route_preserves_neutral_capabilities() {
    let fallback = resolve("databricks_v2", "catalog.schema.unknown");
    for model in [
        "catalog.schema.goose-gpt-6-astra",
        "catalog.schema.gpt-5",
        "catalog.schema.goose-gpt-5-5",
        "catalog.schema.gpt-10",
        "catalog.schema.GOOSE-GPT-6-ASTRA",
        "catalog.schema.team-gpt-6-astra",
    ] {
        let got = resolve(" DATABRICKS-V2 ", model);
        assert_eq!(
            got.databricks_v2_wire_route,
            DatabricksV2Route::OpenaiResponses,
            "{model}"
        );
        assert_eq!(got.thinking_mode, fallback.thinking_mode, "{model}");
        assert_eq!(got.supported_efforts, fallback.supported_efforts, "{model}");
        assert_eq!(got.default_effort, fallback.default_effort, "{model}");
        assert_eq!(
            got.normalization_policy, fallback.normalization_policy,
            "{model}"
        );
        assert_eq!(got.registry_label, None, "{model}");
    }
}

#[test]
fn unrelated_fqns_do_not_select_responses() {
    for model in [
        "gpt-6.schema.other",
        "catalog.gpt-5.other",
        "catalog.schema.claude-gpt-6",
        "catalog.schema.mygpt-6-astra",
        "catalog.schema.gpt-4",
        "catalog.schema.gpt-4o",
        "catalog.schema.gpt-6x",
        "catalog.schema.gpt-oss-120b",
        "catalog.schema.gpt-",
        "catalog.schema.gpt-６",
        "catalog.schema.gpt-4294967296",
        "catalog.schema.kimi-k3",
    ] {
        assert_eq!(
            resolve("databricks_v2", model).databricks_v2_wire_route,
            DatabricksV2Route::MlflowChat,
            "{model}"
        );
    }
    // The new rule is not an endpoint-family or cross-provider capability change.
    assert_eq!(
        resolve("databricks_v2", "goose-gpt-6-astra").databricks_v2_wire_route,
        DatabricksV2Route::MlflowChat
    );
    assert_eq!(
        resolve("openai", "catalog.schema.goose-gpt-6-astra").databricks_v2_wire_route,
        DatabricksV2Route::NotApplicable
    );
}
