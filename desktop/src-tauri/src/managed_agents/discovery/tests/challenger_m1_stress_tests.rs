use crate::managed_agents::custom_harnesses::{check_id_collision, load_custom_harnesses};
use crate::managed_agents::discovery::{
    default_agent_args, known_acp_runtime, normalize_agent_args, normalize_command_identity,
};
use std::fs;

// =============================================================================
// 1. COLLISION GUARD PERMUTATIONS & STRESS TESTS
// =============================================================================

#[test]
fn stress_collision_guard_case_permutations_of_antigravity() {
    let permutations = [
        "antigravity",
        "Antigravity",
        "AntiGravity",
        "ANTIGRAVITY",
        "aNtIgRaViTy",
        "AnTiGrAvItY",
        "aNTIGRAVITY",
        "antigravitY",
        "ANTIgravITY",
    ];

    for id in permutations {
        assert!(
            check_id_collision(id).is_err(),
            "Collision guard MUST reject case permutation: {id}"
        );
    }
}

#[test]
fn stress_collision_guard_all_tier1_case_permutations() {
    let tier1_variants = [
        ("goose", ["Goose", "GOOSE", "gOoSe"]),
        ("claude", ["Claude", "CLAUDE", "cLaUdE"]),
        ("codex", ["Codex", "CODEX", "cOdEx"]),
        ("buzz-agent", ["Buzz-Agent", "BUZZ-AGENT", "buzz-AGENT"]),
        ("antigravity", ["Antigravity", "ANTIGRAVITY", "AntiGravity"]),
    ];

    for (canonical, variants) in tier1_variants {
        assert!(
            check_id_collision(canonical).is_err(),
            "Canonical tier-1 id {canonical} must be reserved"
        );
        for variant in variants {
            assert!(
                check_id_collision(variant).is_err(),
                "Tier-1 variant {variant} must be reserved case-insensitively"
            );
        }
    }
}

#[test]
fn stress_collision_guard_non_colliding_ids_pass() {
    let valid_custom_ids = [
        "my-antigravity",
        "antigravity-custom",
        "custom-antigravity",
        "antigravity2",
        "anti-gravity",
        "google-antigravity-custom",
        "agy-custom",
        "custom_agent",
        "antigravity_plugin",
    ];

    for id in valid_custom_ids {
        assert!(
            check_id_collision(id).is_ok(),
            "Valid custom id {id} must pass collision check"
        );
    }
}

#[test]
fn stress_custom_harness_loader_drops_antigravity_shadowing_files() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Case 1: exact id "antigravity"
    fs::write(
        dir.path().join("antigravity.json"),
        r#"{"id":"antigravity","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();

    // Case 2: uppercase id "ANTIGRAVITY"
    fs::write(
        dir.path().join("shadow_upper.json"),
        r#"{"id":"ANTIGRAVITY","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();

    // Case 3: mixed case id "AntiGravity"
    fs::write(
        dir.path().join("shadow_mixed.json"),
        r#"{"id":"AntiGravity","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();

    // Case 4: whitespace in id (should fail validation)
    fs::write(
        dir.path().join("invalid_ws1.json"),
        r#"{"id":" antigravity","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("invalid_ws2.json"),
        r#"{"id":"antigravity ","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("invalid_ws3.json"),
        r#"{"id":"anti gravity","label":"Fake Antigravity","command":"fake-agy"}"#,
    )
    .unwrap();

    // Case 5: a valid custom harness with a non-colliding ID
    fs::write(
        dir.path().join("valid_custom.json"),
        r#"{"id":"my-antigravity-runner","label":"My Antigravity Runner","command":"my-runner"}"#,
    )
    .unwrap();

    // Case 6: a file NAMED antigravity_named.json but containing valid custom id
    fs::write(
        dir.path().join("antigravity_named.json"),
        r#"{"id":"custom-runner","label":"Custom Runner","command":"custom-bin"}"#,
    )
    .unwrap();

    let loaded = load_custom_harnesses(dir.path());
    let loaded_ids: Vec<&str> = loaded.iter().map(|d| d.id.as_str()).collect();

    assert_eq!(
        loaded.len(),
        2,
        "Only non-colliding valid definitions should load, got: {loaded_ids:?}"
    );
    assert!(loaded_ids.contains(&"my-antigravity-runner"));
    assert!(loaded_ids.contains(&"custom-runner"));
    assert!(!loaded_ids.contains(&"antigravity"));
    assert!(!loaded_ids.contains(&"ANTIGRAVITY"));
    assert!(!loaded_ids.contains(&"AntiGravity"));
}

// =============================================================================
// 3. COMMAND NORMALIZATION & CASING STRESS TESTS
// =============================================================================

#[test]
fn stress_normalize_command_identity_all_permutations() {
    let test_cases = [
        ("agy_acp_server", "agy-acp-server"),
        ("agy_acp_server.par", "agy-acp-server"),
        ("AGY_ACP_SERVER.PAR", "agy-acp-server"),
        ("Agy_Acp_Server.Par", "agy-acp-server"),
        ("agy_acp_server.exe", "agy-acp-server"),
        ("AGY_ACP_SERVER.EXE", "agy-acp-server"),
        ("agy-acp-server", "agy-acp-server"),
        ("agy-acp-server.par", "agy-acp-server"),
        ("google_antigravity", "google-antigravity"),
        ("google-antigravity", "google-antigravity"),
        ("GOOGLE_ANTIGRAVITY.PAR", "google-antigravity"),
        ("antigravity", "antigravity"),
        ("AntiGravity", "antigravity"),
        ("ANTIGRAVITY", "antigravity"),
        ("agy", "agy"),
        ("AGY", "agy"),
        // Path variations
        ("/opt/google/bin/agy_acp_server.par", "agy-acp-server"),
        ("/usr/local/bin/agy_acp_server", "agy-acp-server"),
        (r"C:\Google\Bin\agy_acp_server.exe", "agy-acp-server"),
        ("C:/Google/Bin/agy_acp_server.par", "agy-acp-server"),
        (
            r"C:\Program Files\Google Antigravity\agy_acp_server.exe",
            "agy-acp-server",
        ),
        // Whitespace handling
        ("  agy_acp_server.par  ", "agy-acp-server"),
        ("\tagy_acp_server.exe\n", "agy-acp-server"),
        ("  antigravity  ", "antigravity"),
        // Underscores and hyphens
        ("agy__acp--server.par", "agy--acp--server"),
        ("my_custom_agent.par", "my-custom-agent"),
    ];

    for (input, expected) in test_cases {
        let normalized = normalize_command_identity(input);
        assert_eq!(
            normalized, expected,
            "normalize_command_identity({input:?}) failed: got {normalized:?}, expected {expected:?}"
        );
    }
}

#[test]
fn stress_known_acp_runtime_lookup_permutations() {
    let lookup_cases = [
        "antigravity",
        "Antigravity",
        "ANTIGRAVITY",
        "google-antigravity",
        "Google_Antigravity",
        "GOOGLE_ANTIGRAVITY",
        "agy",
        "AGY",
    ];

    for query in lookup_cases {
        let runtime = known_acp_runtime(query);
        assert!(
            runtime.is_some(),
            "known_acp_runtime({query:?}) should resolve to antigravity"
        );
        let rt = runtime.unwrap();
        assert_eq!(
            rt.id, "antigravity",
            "Resolved runtime ID for {query:?} must be 'antigravity', got: {:?}",
            rt.id
        );
    }
}

// =============================================================================
// 4. DEFAULT AGENT ARGS & LEGACY ACP NORMALIZATION TESTS
// =============================================================================

#[test]
fn stress_default_agent_args_and_normalization() {
    // agy / antigravity (real CLI) -> empty on all platforms
    for cmd in [
        "antigravity",
        "Antigravity",
        "agy",
        "AGY",
        "google-antigravity",
        "/opt/bin/agy",
    ] {
        let args = default_agent_args(cmd);
        assert!(args.is_some());
        assert_eq!(
            args.unwrap(),
            Vec::<String>::new(),
            "agy/antigravity must be empty for {cmd}"
        );
        assert_eq!(normalize_agent_args(cmd, Vec::new()), Vec::<String>::new());
        assert_eq!(
            normalize_agent_args(cmd, vec!["   ".into()]),
            Vec::<String>::new()
        );
        assert_eq!(
            normalize_agent_args(cmd, vec!["acp".into()]),
            Vec::<String>::new()
        );
        let custom = vec!["--custom-flag".to_string(), "foo".to_string()];
        assert_eq!(normalize_agent_args(cmd, custom.clone()), custom);
    }
    // agy_acp_server spellings normalize to the same zero-arg identity.
    for cmd in [
        "agy_acp_server",
        "agy_acp_server.par",
        "AGY_ACP_SERVER.PAR",
        "/opt/bin/agy_acp_server.par",
        r"C:\tools\agy_acp_server.exe",
    ] {
        assert_eq!(default_agent_args(cmd), Some(Vec::<String>::new()));
    }
}

#[test]
fn stress_goose_preserves_acp_argument() {
    let goose_args = normalize_agent_args("goose", vec!["acp".into()]);
    assert_eq!(
        goose_args,
        vec!["acp"],
        "Goose must preserve 'acp' argument"
    );
}
