use super::*;

#[test]
fn config_requires_an_existing_json_trust_store() {
    let root = tempfile::tempdir().unwrap();
    let trust = root.path().join("trusted-signers.json");
    std::fs::write(&trust, "not json").unwrap();
    let config = NxtlinqAuthorizationConfig {
        trust_store: Some(trust.display().to_string()),
        receipt_root: root.path().join("receipts").display().to_string(),
    };
    assert!(validate_config(&config).unwrap_err().contains("valid JSON"));
    std::fs::write(&trust, r#"{"signers":[]}"#).unwrap();
    validate_config(&config).unwrap();
    assert!(root.path().join("receipts").is_dir());
}

#[test]
fn missing_saved_trust_store_is_not_reused_as_a_default() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("old-operator/trusted-signers.json");
    assert_eq!(
        existing_trust_store(Some(missing.display().to_string())),
        None
    );

    let current = root.path().join("current-operator/trusted-signers.json");
    std::fs::create_dir_all(current.parent().unwrap()).unwrap();
    std::fs::write(&current, r#"{"trustedSigners":[]}"#).unwrap();
    assert_eq!(
        existing_trust_store(Some(format!("  {}  ", current.display()))),
        Some(current.display().to_string())
    );
}

#[test]
fn generated_trust_store_embeds_only_the_verified_public_signer() {
    let document = generated_trust_store_document(
        "review-owner-2026",
        "-----BEGIN PUBLIC KEY-----\nPUBLIC\n-----END PUBLIC KEY-----\n",
    );
    assert_eq!(
        document["trustedSigners"][0]["keyId"],
        "review-owner-2026"
    );
    assert_eq!(
        document["trustedSigners"][0]["publicKey"],
        "-----BEGIN PUBLIC KEY-----\nPUBLIC\n-----END PUBLIC KEY-----"
    );
    assert!(document["trustedSigners"][0].get("publicKeyPath").is_none());
    assert!(document.to_string().find("PRIVATE").is_none());
}

fn policy() -> NxtlinqManifestPolicyDraft {
    NxtlinqManifestPolicyDraft {
        name: "review-agent".into(),
        version: "1.0.0".into(),
        scope: vec!["demo:structured-capabilities".into()],
        aud: vec![NXTLINQ_AUDIENCE.into()],
        capabilities: [
            serde_json::json!({
                "type": "filesystem:read",
                "include": ["README.md", "src/**"],
                "exclude": policy::REQUIRED_SENSITIVE_EXCLUDES
            }),
            serde_json::json!({
                "type": "mcp:connect",
                "servers": ["buzz-dev-mcp"]
            }),
        ]
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect(),
        exp: None,
    }
}

#[test]
fn manifest_draft_preserves_signer_material_and_rejects_secrets() {
    let current = serde_json::to_vec(&serde_json::json!({
        "name": "old",
        "version": "0.1.0",
        "scope": ["old"],
        "issuedAt": 1,
        "publicKey": "PUBLIC",
        "signerKeyId": "owner-key",
        "contentHash": "content",
        "artifactHash": "artifact"
    }))
    .unwrap();
    let (_, proposed) = proposed_manifest(&current, &policy()).unwrap();
    let value: Value = serde_json::from_str(&proposed).unwrap();
    assert_eq!(value["publicKey"], "PUBLIC");
    assert_eq!(value["signerKeyId"], "owner-key");
    assert_eq!(value["name"], "review-agent");

    let mut unsafe_policy = policy();
    unsafe_policy.capabilities[0].insert("privateKey".into(), Value::String("secret".into()));
    assert!(validate_policy(&unsafe_policy)
        .unwrap_err()
        .contains("unsupported constraint"));
}

#[test]
fn manifest_apply_is_bound_to_the_reviewed_digest() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let nxtlinq = project.join("nxtlinq");
    std::fs::create_dir_all(&nxtlinq).unwrap();
    std::fs::write(
        nxtlinq.join("agent.manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "old",
            "version": "0.1.0",
            "scope": ["old"],
            "issuedAt": 1,
            "publicKey": "PUBLIC",
            "contentHash": "content",
            "artifactHash": "artifact"
        }))
        .unwrap(),
    )
    .unwrap();
    let preview = preview_manifest_policy(project.to_str().unwrap(), &policy()).unwrap();
    assert!(preview.changed);
    assert!(preview.unified_diff.contains("filesystem:read"));

    std::fs::write(nxtlinq.join("agent.manifest.json"), b"{}").unwrap();
    let error = apply_manifest_policy(
        project.to_str().unwrap(),
        &policy(),
        &preview.current_sha256,
    )
    .unwrap_err();
    assert!(error.contains("changed after preview"));
    assert_eq!(
        std::fs::read_to_string(nxtlinq.join("agent.manifest.json")).unwrap(),
        "{}"
    );
}

#[test]
fn conversational_setup_refuses_a_workspace_private_key() {
    let root = tempfile::tempdir().unwrap();
    let nxtlinq = root.path().join("nxtlinq");
    std::fs::create_dir_all(&nxtlinq).unwrap();
    std::fs::write(nxtlinq.join("private.key"), "SECRET").unwrap();
    std::fs::write(nxtlinq.join("agent.manifest.json"), "{}").unwrap();

    let error = manifest_path(root.path().to_str().unwrap()).unwrap_err();
    assert!(error.contains("outside the Agent workspace"));
}

#[test]
fn initialization_status_is_typed_and_fails_closed() {
    const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEALzS/8vAIW8BQrdfk6cnLUGuD4hr5NiEHxNrH33oFc7c=\n-----END PUBLIC KEY-----\n";

    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let canonical_project = std::fs::canonicalize(&project).unwrap();
    let project_path = canonical_project.to_str().unwrap();
    assert_eq!(
        initialization_status(project_path).status,
        NxtlinqAttestInitializationState::Missing
    );

    let nxtlinq = project.join("nxtlinq");
    std::fs::create_dir(&nxtlinq).unwrap();
    std::fs::write(nxtlinq.join("private.key"), "SECRET").unwrap();
    assert_eq!(
        initialization_status(project_path).status,
        NxtlinqAttestInitializationState::WorkspacePrivateKey
    );

    std::fs::remove_file(nxtlinq.join("private.key")).unwrap();
    std::fs::write(nxtlinq.join("public.key"), PUBLIC_KEY).unwrap();
    std::fs::write(
        nxtlinq.join("agent.manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "publicKey": PUBLIC_KEY.trim(),
            "signerKeyId": "project-owner-2026"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        initialization_status(project_path).status,
        NxtlinqAttestInitializationState::Invalid
    );

    std::fs::write(
        nxtlinq.join("agent.manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": "my-agent",
            "version": "1.0.0",
            "scope": ["tool:ExampleTool"],
            "issuedAt": 1,
            "publicKey": PUBLIC_KEY.trim(),
            "signerKeyId": "project-owner-2026",
            "contentHash": "<set by attest sign>",
            "artifactHash": "<set by attest sign>",
            "attestCliVersion": NXTLINQ_ATTEST_VERSION
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        initialization_status(project_path).status,
        NxtlinqAttestInitializationState::Initialized
    );

    std::fs::write(nxtlinq.join("public.key"), "different").unwrap();
    assert_eq!(
        initialization_status(project_path).status,
        NxtlinqAttestInitializationState::Invalid
    );
}

#[test]
fn manifest_diff_preserves_unchanged_context_for_split_review() {
    let old = "{\n  \"name\": \"before\",\n  \"version\": \"1.0.0\"\n}\n";
    let new = "{\n  \"name\": \"after\",\n  \"version\": \"1.0.0\"\n}\n";
    let diff = full_unified_diff(old, new, Path::new("nxtlinq/agent.manifest.json"));

    assert!(diff.contains("-  \"name\": \"before\""));
    assert!(diff.contains("+  \"name\": \"after\""));
    assert!(diff.contains("   \"version\": \"1.0.0\""));
    assert!(!diff.contains("-  \"version\": \"1.0.0\""));
    assert!(!diff.contains("+  \"version\": \"1.0.0\""));
}
