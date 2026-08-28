use super::*;

fn trigger() -> TriggerContext {
    TriggerContext {
        channel_id: Uuid::nil().to_string(),
        webhook_fields: HashMap::from([("document_uri".into(), "file://case.pdf".into())]),
        ..TriggerContext::default()
    }
}

#[test]
fn blueprint_is_deterministic_and_preserves_dependencies() {
    let run_id = Uuid::nil();
    let step = Step {
        id: "analysis".into(),
        name: None,
        if_expr: None,
        timeout_secs: None,
        depends_on: vec!["ingest".into()],
        action: ActionDef::RunAgent {
            agent: "helena".into(),
            identity: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".into(),
            prompt: "Analyze {{trigger.document_uri}}".into(),
            output_schema: json!({
                "type": "object",
                "required": ["decision"]
            }),
        },
    };
    let first =
        task_blueprint(run_id, Uuid::nil(), &trigger(), 0, &step).expect("blueprint should build");
    let second =
        task_blueprint(run_id, Uuid::nil(), &trigger(), 0, &step).expect("blueprint should build");
    assert_eq!(first.input, second.input);
    assert_eq!(first.depends_on, json!(["ingest"]));
    assert_eq!(first.input["timeout_secs"], DEFAULT_AGENT_TIMEOUT_SECONDS);
    assert_eq!(first.idempotency_key, format!("{run_id}:analysis"));
    assert_eq!(
        first.output_schema,
        Some(json!({ "type": "object", "required": ["decision"] }))
    );
}

#[test]
fn prepares_deterministic_document_manifest() {
    let payload = json!({
        "source_name": "case.txt",
        "content_type": "text/plain",
        "source_base64": "Y2FzZQ==",
        "pages": [{
            "physical_page": 1,
            "logical_label": "fls. 1",
            "text": "Synthetic evidence"
        }]
    });
    let prepared = prepare_ingestion(&payload.to_string()).expect("payload should ingest");
    assert_eq!(prepared.content["source_name"], "case.txt");
    assert_eq!(prepared.content["page_count"], 1);
    assert_eq!(prepared.metadata["chunk_count"], 1);
    assert_eq!(prepared.artifact_sha256.len(), 32);
    assert_eq!(prepared.manifest_hash.len(), 32);
}

#[test]
fn rejects_invalid_document_base64() {
    let payload = json!({
        "source_name": "case.txt",
        "content_type": "text/plain",
        "source_base64": "%%%",
        "pages": []
    });
    let error = prepare_ingestion(&payload.to_string()).expect_err("invalid base64 must fail");
    assert!(error.contains("source_base64 is invalid"));
}

#[test]
fn ingestion_blueprint_strips_raw_document_from_trigger() {
    let mut trigger = trigger();
    trigger.webhook_fields.insert(
        "document_input".into(),
        json!({
            "source_name": "case.txt",
            "content_type": "text/plain",
            "source_base64": "Y2FzZQ==",
            "pages": []
        })
        .to_string(),
    );
    let step = Step {
        id: "ingest".into(),
        name: None,
        if_expr: None,
        timeout_secs: None,
        depends_on: vec![],
        action: ActionDef::IngestDocument {
            source: "{{trigger.document_input}}".into(),
            output: "document_manifest".into(),
        },
    };
    let blueprint = task_blueprint(Uuid::nil(), Uuid::nil(), &trigger, 0, &step)
        .expect("ingestion blueprint should build");
    assert!(blueprint.input["action"]["source"]
        .as_str()
        .is_some_and(|value| value.contains("source_base64")));
    assert!(blueprint.input["trigger"]["webhook_fields"]
        .get("document_input")
        .is_none());
}

#[test]
fn workflow_owner_approval_resolves_to_exact_pubkey() {
    let owner = [7_u8; 32];
    assert_eq!(
        resolve_approver_spec("@workflow-owner", &owner).expect("owner should resolve"),
        hex::encode(owner)
    );
    assert!(resolve_approver_spec("@mutable-display-name", &owner).is_err());
}

#[test]
fn unsupported_coordinator_action_remains_explicit_in_input() {
    let step = Step {
        id: "publish".into(),
        name: None,
        if_expr: None,
        timeout_secs: None,
        depends_on: vec!["approval".into()],
        action: ActionDef::PublishArtifact {
            artifact: "decision".into(),
        },
    };
    let blueprint = task_blueprint(Uuid::nil(), Uuid::nil(), &trigger(), 0, &step)
        .expect("blueprint should build");
    assert_eq!(blueprint.input["action"]["action"], "publish_artifact");
    assert_eq!(blueprint.input["action"]["artifact"], "decision");
}
#[test]
fn synthetic_tribunal_pilot_preserves_full_gated_dag_and_independent_identities() {
    let yaml = include_str!("../../../examples/agent-workflows/tribunal/workflow.yaml");
    let (definition, _) = WorkflowEngine::parse_yaml(yaml).expect("tribunal workflow should parse");
    let by_id = definition
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<HashMap<_, _>>();
    assert_eq!(definition.steps.len(), 11);
    assert_eq!(
        by_id["analysis_barrier"].depends_on,
        ["defense_analysis", "contradictor_analysis"]
    );
    assert_eq!(
        by_id["debate_barrier"].depends_on,
        ["defense_debate", "contradictor_debate"]
    );
    assert_eq!(by_id["judicial_review"].depends_on, ["debate_barrier"]);
    assert_eq!(by_id["verify_citations"].depends_on, ["judicial_review"]);
    assert_eq!(by_id["human_approval"].depends_on, ["verify_citations"]);
    assert_eq!(by_id["publish"].depends_on, ["human_approval"]);

    let identities = definition
        .steps
        .iter()
        .filter_map(|step| match &step.action {
            ActionDef::RunAgent { identity, .. } | ActionDef::VerifyArtifact { identity, .. } => {
                Some(identity.as_str())
            }
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 4);
    assert!(matches!(
        by_id["verify_citations"].action,
        ActionDef::VerifyArtifact { .. }
    ));
    assert!(matches!(
        by_id["human_approval"].action,
        ActionDef::RequestApproval { .. }
    ));
    assert!(matches!(
        by_id["publish"].action,
        ActionDef::PublishArtifact { .. }
    ));

    let run_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    let blueprints = definition
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            task_blueprint(run_id, channel_id, &trigger(), index as i32, step)
                .expect("every tribunal step should materialize")
        })
        .collect::<Vec<_>>();
    assert_eq!(blueprints.len(), definition.steps.len());
    assert_eq!(blueprints[10].depends_on, json!(["human_approval"]));
}

#[test]
fn golden_3218_page_manifest_is_deterministic_and_coordinate_safe() {
    let pages = (1..=3_218)
        .map(|page| crate::document::ExtractedPage {
            physical_page: page,
            logical_label: Some(format!("fls. {page}")),
            text: format!("Synthetic legal record page {page}"),
        })
        .collect::<Vec<_>>();
    let source = b"synthetic-3218-page-golden-source";
    let build = || {
        crate::document::build_document_manifest(
            crate::document::DocumentInput {
                source_name: "autos-3218.pdf",
                content_type: "application/pdf",
                source_bytes: source,
                pages: &pages,
            },
            crate::document::IngestLimits::default(),
        )
        .expect("3,218-page manifest should fit configured limits")
    };
    let first = build();
    let second = build();
    assert_eq!(first.manifest_sha256, second.manifest_sha256);
    assert_eq!(first.page_count, 3_218);
    assert_eq!(first.chunks.len(), 3_218);
    let last = first
        .chunks
        .last()
        .expect("golden manifest should have chunks");
    assert_eq!(last.physical_page, 3_218);
    assert_eq!(last.logical_label.as_deref(), Some("fls. 3218"));
    assert!(first
        .chunks
        .iter()
        .all(|chunk| chunk.physical_page <= 3_218));
    crate::document::verify_document_manifest(&first, source)
        .expect("golden manifest integrity should verify");
}
