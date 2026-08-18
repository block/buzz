use super::*;

fn signing_policy() -> NxtlinqManifestPolicyDraft {
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
fn signing_key_must_be_external_and_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let internal = project.join("private.key");
    std::fs::write(&internal, "SECRET").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&internal, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(validate_external_private_key(&project, &internal)
        .unwrap_err()
        .contains("outside the Agent workspace"));

    let external = root.path().join("owner-private.key");
    std::fs::write(&external, "SECRET").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_external_private_key(&project, &external)
            .unwrap_err()
            .contains("group/other"));
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(
        validate_external_private_key(&project, &external).unwrap(),
        std::fs::canonicalize(&external).unwrap()
    );
}

#[test]
fn signing_is_bound_to_the_approved_policy_fields() {
    let approved = signing_policy();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "name": approved.name,
        "version": approved.version,
        "scope": approved.scope,
        "aud": approved.aud,
        "capabilities": approved.capabilities,
        "publicKey": "PUBLIC"
    }))
    .unwrap();
    assert_manifest_policy(&manifest, &signing_policy()).unwrap();

    let mut changed: Value = serde_json::from_slice(&manifest).unwrap();
    changed["scope"] = serde_json::json!(["expanded"]);
    assert!(
        assert_manifest_policy(&serde_json::to_vec(&changed).unwrap(), &signing_policy())
            .unwrap_err()
            .contains("approved proposal")
    );
}
