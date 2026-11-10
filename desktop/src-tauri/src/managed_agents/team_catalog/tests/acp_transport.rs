use super::*;

#[test]
fn portable_transport_is_projected_validated_and_hashed() {
    let mut one = member("m1", "One");
    let legacy_hash = local_member_projection_hash(&one);
    one.acp_command = Some("buzz-janet-acp".to_string());
    let content = build_team_catalog_content(&team(), &[one.clone()]).unwrap();
    assert_eq!(
        content.members[0].acp_command.as_deref(),
        Some("buzz-janet-acp")
    );
    assert_ne!(local_member_projection_hash(&one), legacy_hash);
    let json = team_catalog_content_json(&content).unwrap();
    let parsed: TeamCatalogContent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, content);
    validate_member(&parsed.members[0]).unwrap();
    for command in ["/tmp/buzz-janet-acp", r"C:\buzz-janet-acp.cmd", "sh"] {
        let mut foreign = parsed.members[0].clone();
        foreign.acp_command = Some(command.to_string());
        assert!(validate_member(&foreign)
            .unwrap_err()
            .contains("ACP command"));
        one.acp_command = Some(command.to_string());
        let exported = build_team_catalog_content(&team(), &[one.clone()]).unwrap();
        assert!(exported.members[0].acp_command.is_none());
    }
}
