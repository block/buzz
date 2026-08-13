use super::*;
use std::os::unix::fs::PermissionsExt;

const TARGET_AGENT_PUBKEY: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn config(program: String, args: Vec<String>) -> HeartbeatPreflightConfig {
    let program_sha256 =
        hash_file(&std::fs::File::open(&program).expect("open helper for owner pin"))
            .expect("hash helper");
    HeartbeatPreflightConfig {
        version: 1,
        target_agent_pubkey: TARGET_AGENT_PUBKEY.into(),
        target_channel: "5e06068b-0c7d-444c-9a48-080c45b65931".into(),
        declaration_manifest_digest: "d".repeat(64),
        heartbeat_interval_seconds: Some(3_600),
        program,
        program_sha256,
        macos_designated_requirement: None,
        macos_team_identifier: None,
        args,
        required_sources: vec![
            RequiredSourceScope {
                source: "gmail".into(),
                account: "owner@example.com".into(),
                scope: "inbox".into(),
                policy_id: "gmail.required".into(),
            },
            RequiredSourceScope {
                source: "slack".into(),
                account: "owner-workspace".into(),
                scope: "inbox".into(),
                policy_id: "slack.required".into(),
            },
        ],
        ledger_instance_id: "ledger-primary".into(),
        timeout_ms: 10_000,
        max_output_bytes: 4096,
        forward_env: vec![],
    }
}

fn temp_script(name: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let directory = std::env::current_dir()
        .expect("current directory")
        .join("target")
        .join("heartbeat-preflight-tests")
        .join(format!("{}-{}", name, Uuid::new_v4()));
    std::fs::create_dir_all(&directory).expect("create test directory");
    let mut directory_permissions = std::fs::metadata(&directory)
        .expect("directory metadata")
        .permissions();
    directory_permissions.set_mode(0o700);
    std::fs::set_permissions(&directory, directory_permissions).expect("secure test directory");
    let path = directory.join(format!("helper;{name} script"));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write helper");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).expect("make helper executable");
    (directory, path)
}

fn echo_result_body(status: &str, status_fields: &str) -> String {
    let (status_fields, status_arguments) = if status == "checked" {
        (
            format!(
                "\"witness_run_id\":\"slack-run-%s\",\"receipt_digest\":\"{}\",\"acceptance_context\":\"%s\",{status_fields}",
                "b".repeat(64)
            ),
            r#""$requested" "$invocation" "$invocation""#,
        )
    } else {
        (status_fields.to_string(), r#""$requested""#)
    };
    format!(
        r#"IFS= read -r request
turn=${{request#*\"turn_id\":\"}}
turn=${{turn%%\"*}}
invocation=${{request#*\"invocation_id\":\"}}
invocation=${{invocation%%\"*}}
requested=${{request#*\"requested_at\":\"}}
requested=${{requested%%\"*}}
printf '{{\"version\":1,\"turn_id\":\"%s\",\"invocation_id\":\"%s\",\"target_agent_pubkey\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"target_channel\":\"5e06068b-0c7d-444c-9a48-080c45b65931\",\"declaration_manifest_digest\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"required_sources\":[{{\"source\":\"gmail\",\"account\":\"owner@example.com\",\"scope\":\"inbox\",\"policy_id\":\"gmail.required\"}},{{\"source\":\"slack\",\"account\":\"owner-workspace\",\"scope\":\"inbox\",\"policy_id\":\"slack.required\"}}],\"ledger_instance_id\":\"ledger-primary\",\"authority_commit\":\"1111111111111111111111111111111111111111\",\"remote_readback_commit\":\"1111111111111111111111111111111111111111\",\"outcomes\":[{{\"required_source\":{{\"source\":\"gmail\",\"account\":\"owner@example.com\",\"scope\":\"inbox\",\"policy_id\":\"gmail.required\"}},\"status\":\"checked\",\"checked_at\":\"%s\",\"receipt_id\":\"gmail:receipt\",\"witness_run_id\":\"gmail-run-%s\",\"receipt_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"acceptance_context\":\"%s\",\"item_count\":0}},{{\"required_source\":{{\"source\":\"slack\",\"account\":\"owner-workspace\",\"scope\":\"inbox\",\"policy_id\":\"slack.required\"}},\"status\":\"{status}\",\"checked_at\":\"%s\",\"receipt_id\":\"slack:receipt\",{status_fields}}}],\"committed_material\":[]}}\n' "$turn" "$invocation" "$requested" "$invocation" "$invocation" {status_arguments}"#
    )
}

#[tokio::test]
async fn blocked_manifest_is_visible_and_fails_closed() {
    for reason_code in ["upstream_blocked", "not_configured", "failed", "missing"] {
        let (directory, path) = temp_script(
            reason_code,
            &echo_result_body("blocked", &format!("\"reason_code\":\"{reason_code}\"")),
        );
        let error = run_heartbeat_preflight(
            &config(path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-1",
        )
        .await
        .expect_err("a blocked required source must suppress the heartbeat");
        assert!(matches!(
            &error,
            HeartbeatPreflightError::IncompleteSweep(blocked)
                if blocked == &format!("slack:{reason_code}")
        ));
        assert!(
            error.to_string().contains(&format!("slack:{reason_code}")),
            "the blocked source and reason must remain visible"
        );
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }
}

#[tokio::test]
async fn required_policy_is_reread_and_loss_fails_closed_before_gateway() {
    let (directory, program) = temp_script(
        "required-policy",
        &echo_result_body("checked", "\"item_count\":0"),
    );
    let policy = config(program.to_string_lossy().into_owned(), vec![]);
    let policy_path = directory.join("owner-policy.json");
    let policy_bytes = serde_json::to_vec(&policy).expect("serialize owner policy");
    std::fs::write(&policy_path, &policy_bytes).expect("write owner policy");
    std::fs::set_permissions(&policy_path, std::fs::Permissions::from_mode(0o600))
        .expect("secure owner policy");
    let authority = HeartbeatPreflightAuthority::required_file(
        policy_path.clone(),
        hex::encode(Sha256::digest(&policy_bytes)),
        TARGET_AGENT_PUBKEY,
        3_600,
    )
    .expect("valid required policy");

    run_heartbeat_preflight(&authority, TARGET_AGENT_PUBKEY, "required-turn-1")
        .await
        .expect("first current policy run");
    std::fs::write(&policy_path, b"{}").expect("replace owner policy");
    assert!(matches!(
        run_heartbeat_preflight(&authority, TARGET_AGENT_PUBKEY, "required-turn-2").await,
        Err(HeartbeatPreflightError::PolicyDigestMismatch)
    ));
    std::fs::remove_file(&policy_path).expect("remove owner policy");
    assert!(matches!(
        run_heartbeat_preflight(&authority, TARGET_AGENT_PUBKEY, "required-turn-3").await,
        Err(HeartbeatPreflightError::PolicyUnavailable(_))
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn same_invocation_retry_is_idempotent_but_prior_context_replay_is_rejected() {
    let durable_result_body = echo_result_body("checked", "\"item_count\":0");
    let body = format!(
        r#"result_file="$0.terminal.json"
counter_file="$0.connector-count"
if [ -s "$result_file" ]; then
  IFS= read -r _request
  /bin/cat "$result_file"
  exit 0
fi
count=0
if [ -f "$counter_file" ]; then IFS= read -r count < "$counter_file"; fi
count=$((count + 1))
printf '%s\n' "$count" > "$counter_file"
exec 3>&1
exec > "$result_file"
{durable_result_body}
exec 1>&3
/bin/cat "$result_file""#
    );
    let (directory, path) = temp_script("same-run", &body);
    let config = config(path.to_string_lossy().into_owned(), vec![]);
    let invocation = HeartbeatPreflightInvocation::with_requested_at("same-turn", Utc::now());

    let first = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, &invocation)
        .await
        .expect("first attempt");
    tokio::time::sleep(Duration::from_millis(5)).await;
    let retry = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, &invocation)
        .await
        .expect("same invocation retry");
    assert_eq!(
        first, retry,
        "an idempotent retry must consume the byte-identical terminal receipt"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.connector-count", path.display()))
            .expect("read connector count")
            .trim(),
        "1",
        "the durable gateway must not execute the source connector again"
    );

    let different_invocation =
        HeartbeatPreflightInvocation::with_requested_at("different-turn", Utc::now());
    let error = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, &different_invocation)
        .await
        .expect_err("a result carrying prior acceptance contexts must not cross runs");
    assert!(matches!(error, HeartbeatPreflightError::InvalidResult(_)));
    assert_eq!(
        std::fs::read_to_string(format!("{}.connector-count", path.display()))
            .expect("read connector count after rejected replay")
            .trim(),
        "1"
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn committed_material_is_hash_commit_scope_and_size_bound_before_prompt_rendering() {
    let (directory, path) = temp_script(
        "committed-material",
        &echo_result_body("checked", "\"item_count\":0"),
    );
    let config = config(path.to_string_lossy().into_owned(), vec![]);
    let invocation = HeartbeatPreflightInvocation::with_requested_at(
        "material-turn",
        Utc::now() - chrono::Duration::seconds(1),
    );
    let mut result = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, &invocation)
        .await
        .expect("base checked result");
    let sanitized_text = "sanitized committed evidence";
    result.committed_material.push(CommittedMaterialItem {
        required_source: config.required_sources[0].clone(),
        entry_id: "gmail:item-1".into(),
        authority_commit: result.authority_commit.clone(),
        content_sha256: hex::encode(Sha256::digest(sanitized_text.as_bytes())),
        sanitized_text: Some(sanitized_text.into()),
        ledger_pointer: None,
    });
    result.outcomes[0].item_count = Some(1);
    result
        .validate(
            &config,
            &invocation.turn_id,
            &invocation.turn_id,
            invocation.requested_at,
        )
        .expect("valid committed material");

    let mut wrong_channel = result.clone();
    wrong_channel.target_channel = "different-channel".into();
    assert!(matches!(
        wrong_channel.validate(
            &config,
            &invocation.turn_id,
            &invocation.turn_id,
            invocation.requested_at,
        ),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut wrong_declaration = result.clone();
    wrong_declaration.declaration_manifest_digest = "e".repeat(64);
    assert!(matches!(
        wrong_declaration.validate(
            &config,
            &invocation.turn_id,
            &invocation.turn_id,
            invocation.requested_at,
        ),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));
    assert!(result
        .prompt_section()
        .expect("render valid committed material")
        .contains(sanitized_text));

    let mut omitted_material = result.clone();
    omitted_material.outcomes[0].item_count = Some(0);
    assert!(matches!(
        omitted_material.validate(
            &config,
            &invocation.turn_id,
            &invocation.turn_id,
            invocation.requested_at,
        ),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut aggregate_pointer = result.clone();
    aggregate_pointer.committed_material[0].sanitized_text = None;
    aggregate_pointer.committed_material[0].ledger_pointer =
        Some("ledger:batch/gmail-material-1".into());
    aggregate_pointer
        .validate(
            &config,
            &invocation.turn_id,
            &invocation.turn_id,
            invocation.requested_at,
        )
        .expect("bounded aggregate ledger pointer");

    let mut wrong_commit = result.clone();
    wrong_commit.committed_material[0].authority_commit = "2".repeat(40);
    assert!(matches!(
        wrong_commit.prompt_section(),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut relabeled_payload = result.clone();
    relabeled_payload.committed_material[0].sanitized_text =
        Some("different bytes under the prior digest".into());
    assert!(matches!(
        relabeled_payload.prompt_section(),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut oversized_payload = result.clone();
    let oversized = "x".repeat(MAX_COMMITTED_MATERIAL_TEXT_BYTES + 1);
    oversized_payload.committed_material[0].content_sha256 =
        hex::encode(Sha256::digest(oversized.as_bytes()));
    oversized_payload.committed_material[0].sanitized_text = Some(oversized);
    assert!(matches!(
        oversized_payload.prompt_section(),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut too_many_items = aggregate_pointer.clone();
    too_many_items.committed_material = (0..=MAX_COMMITTED_MATERIAL_ITEMS)
        .map(|index| CommittedMaterialItem {
            entry_id: format!("gmail:item-{index}"),
            ..aggregate_pointer.committed_material[0].clone()
        })
        .collect();
    too_many_items.outcomes[0].item_count =
        Some(u64::try_from(too_many_items.committed_material.len()).expect("bounded test count"));
    assert!(matches!(
        too_many_items.prompt_section(),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let mut oversized_section = result;
    let chunk = "x".repeat(MAX_COMMITTED_MATERIAL_TEXT_BYTES);
    oversized_section.committed_material = (0..9)
        .map(|index| CommittedMaterialItem {
            required_source: config.required_sources[0].clone(),
            entry_id: format!("gmail:chunk-{index}"),
            authority_commit: oversized_section.authority_commit.clone(),
            content_sha256: hex::encode(Sha256::digest(chunk.as_bytes())),
            sanitized_text: Some(chunk.clone()),
            ledger_pointer: None,
        })
        .collect();
    oversized_section.outcomes[0].item_count = Some(9);
    assert!(matches!(
        oversized_section.prompt_section(),
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn omitted_committed_material_contract_fails_closed() {
    let body = echo_result_body("checked", "\"item_count\":0")
        .replace(",\\\"committed_material\\\":[]", "");
    let (directory, path) = temp_script("missing-material-contract", &body);
    assert!(matches!(
        run_heartbeat_preflight(
            &config(path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "missing-material-turn"
        )
        .await,
        Err(HeartbeatPreflightError::MalformedResult)
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn required_policy_target_mismatch_is_not_treated_as_absent() {
    let (directory, program) = temp_script("required-target", "exit 0");
    let mut policy = config(program.to_string_lossy().into_owned(), vec![]);
    policy.target_agent_pubkey = "b".repeat(64);
    let policy_path = directory.join("owner-policy.json");
    let policy_bytes = serde_json::to_vec(&policy).expect("serialize owner policy");
    std::fs::write(&policy_path, &policy_bytes).expect("write owner policy");
    let error = HeartbeatPreflightAuthority::required_file(
        policy_path,
        hex::encode(Sha256::digest(&policy_bytes)),
        TARGET_AGENT_PUBKEY,
        3_600,
    )
    .expect_err("mistargeted required policy must fail startup");
    assert!(matches!(
        error,
        HeartbeatPreflightError::TargetAgentMismatch
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn required_policy_must_carry_the_exact_positive_owner_cadence() {
    let (directory, program) = temp_script("required-cadence", "exit 0");
    let policy_path = directory.join("owner-policy.json");
    let mut policy = config(program.to_string_lossy().into_owned(), vec![]);
    for (policy_cadence, designation_cadence) in [(None, 3_600), (Some(60), 3_600)] {
        policy.heartbeat_interval_seconds = policy_cadence;
        let bytes = serde_json::to_vec(&policy).expect("serialize owner policy");
        std::fs::write(&policy_path, &bytes).expect("write owner policy");
        let error = HeartbeatPreflightAuthority::required_file(
            policy_path.clone(),
            hex::encode(Sha256::digest(&bytes)),
            TARGET_AGENT_PUBKEY,
            designation_cadence,
        )
        .expect_err("missing or mismatched required cadence must fail startup");
        assert!(matches!(error, HeartbeatPreflightError::InvalidConfig(_)));
    }
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn omitted_required_source_fails_closed() {
    let body = r#"IFS= read -r request
turn=${request#*\"turn_id\":\"}; turn=${turn%%\"*}
invocation=${request#*\"invocation_id\":\"}; invocation=${invocation%%\"*}
requested=${request#*\"requested_at\":\"}; requested=${requested%%\"*}
printf '{\"version\":1,\"turn_id\":\"%s\",\"invocation_id\":\"%s\",\"target_agent_pubkey\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"target_channel\":\"5e06068b-0c7d-444c-9a48-080c45b65931\",\"declaration_manifest_digest\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"required_sources\":[{\"source\":\"gmail\",\"account\":\"owner@example.com\",\"scope\":\"inbox\",\"policy_id\":\"gmail.required\"},{\"source\":\"slack\",\"account\":\"owner-workspace\",\"scope\":\"inbox\",\"policy_id\":\"slack.required\"}],\"ledger_instance_id\":\"ledger-primary\",\"authority_commit\":\"1111111111111111111111111111111111111111\",\"remote_readback_commit\":\"1111111111111111111111111111111111111111\",\"outcomes\":[{\"required_source\":{\"source\":\"gmail\",\"account\":\"owner@example.com\",\"scope\":\"inbox\",\"policy_id\":\"gmail.required\"},\"status\":\"checked\",\"checked_at\":\"%s\",\"receipt_id\":\"gmail:receipt\",\"witness_run_id\":\"gmail-run-%s\",\"receipt_digest\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"acceptance_context\":\"%s\",\"item_count\":0}],\"committed_material\":[]}\n' "$turn" "$invocation" "$requested" "$invocation" "$invocation""#;
    let (directory, path) = temp_script("partial", body);
    let error = run_heartbeat_preflight(
        &config(path.to_string_lossy().into_owned(), vec![]),
        TARGET_AGENT_PUBKEY,
        "turn-1",
    )
    .await
    .expect_err("partial manifest must fail");
    assert!(matches!(error, HeartbeatPreflightError::InvalidResult(_)));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn timeout_malformed_and_oversized_output_fail_closed() {
    let (timeout_dir, timeout_path) = temp_script("timeout", "/bin/sleep 2");
    let mut timeout_config = config(timeout_path.to_string_lossy().into_owned(), vec![]);
    timeout_config.timeout_ms = 100;
    assert!(matches!(
        run_heartbeat_preflight(&timeout_config, TARGET_AGENT_PUBKEY, "turn-timeout").await,
        Err(HeartbeatPreflightError::Timeout(100))
    ));

    let (malformed_dir, malformed_path) = temp_script("malformed", "printf 'not-json'");
    assert!(matches!(
        run_heartbeat_preflight(
            &config(malformed_path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-malformed"
        )
        .await,
        Err(HeartbeatPreflightError::MalformedResult)
    ));

    let (oversized_dir, oversized_path) =
        temp_script("oversized", "/usr/bin/head -c 5000 /dev/zero");
    let oversized = config(oversized_path.to_string_lossy().into_owned(), vec![]);
    assert!(matches!(
        run_heartbeat_preflight(&oversized, TARGET_AGENT_PUBKEY, "turn-oversized").await,
        Err(HeartbeatPreflightError::OutputTooLarge)
    ));

    for directory in [timeout_dir, malformed_dir, oversized_dir] {
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }
}

#[tokio::test]
async fn executable_path_and_args_are_literal_and_invocation_ids_are_unique() {
    let body = format!(
        "[ \"$1\" = 'arg;touch should-not-exist' ] || exit 9\n{}",
        echo_result_body("checked", "\"item_count\":0")
    );
    let (directory, path) = temp_script("literal", &body);
    let config = config(
        path.to_string_lossy().into_owned(),
        vec!["arg;touch should-not-exist".into()],
    );
    let first = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, "turn-1")
        .await
        .expect("first run");
    let second = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, "turn-2")
        .await
        .expect("second run");
    assert_ne!(first.invocation_id, second.invocation_id);
    assert!(!directory.join("should-not-exist").exists());
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn consecutive_runs_accept_distinct_equal_authority_and_readback_commits() {
    let body = r#"state=$1
if [ -e "$state" ]; then
  commit=2222222222222222222222222222222222222222
else
  : > "$state"
  commit=1111111111111111111111111111111111111111
fi
IFS= read -r request
turn=${request#*\"turn_id\":\"}; turn=${turn%%\"*}
invocation=${request#*\"invocation_id\":\"}; invocation=${invocation%%\"*}
requested=${request#*\"requested_at\":\"}; requested=${requested%%\"*}
printf '{"version":1,"turn_id":"%s","invocation_id":"%s","target_agent_pubkey":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_channel":"5e06068b-0c7d-444c-9a48-080c45b65931","declaration_manifest_digest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","required_sources":[{"source":"gmail","account":"owner@example.com","scope":"inbox","policy_id":"gmail.required"},{"source":"slack","account":"owner-workspace","scope":"inbox","policy_id":"slack.required"}],"ledger_instance_id":"ledger-primary","authority_commit":"%s","remote_readback_commit":"%s","outcomes":[{"required_source":{"source":"gmail","account":"owner@example.com","scope":"inbox","policy_id":"gmail.required"},"status":"checked","checked_at":"%s","receipt_id":"gmail:receipt","witness_run_id":"gmail-run-%s","receipt_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","acceptance_context":"%s","item_count":0},{"required_source":{"source":"slack","account":"owner-workspace","scope":"inbox","policy_id":"slack.required"},"status":"checked","checked_at":"%s","receipt_id":"slack:receipt","witness_run_id":"slack-run-%s","receipt_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","acceptance_context":"%s","item_count":0}],"committed_material":[]}\n' "$turn" "$invocation" "$commit" "$commit" "$requested" "$invocation" "$invocation" "$requested" "$invocation" "$invocation""#;
    let (directory, path) = temp_script("moving-commit", body);
    let state_path = directory.join("run-state");
    let config = config(
        path.to_string_lossy().into_owned(),
        vec![state_path.to_string_lossy().into_owned()],
    );

    let first = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, "turn-commit-1")
        .await
        .expect("first committed sweep");
    let second = run_heartbeat_preflight(&config, TARGET_AGENT_PUBKEY, "turn-commit-2")
        .await
        .expect("second committed sweep");

    assert_eq!(
        first.authority_commit,
        "1111111111111111111111111111111111111111"
    );
    assert_eq!(first.remote_readback_commit, first.authority_commit);
    assert_eq!(
        second.authority_commit,
        "2222222222222222222222222222222222222222"
    );
    assert_eq!(second.remote_readback_commit, second.authority_commit);
    assert_ne!(first.authority_commit, second.authority_commit);
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn config_is_strict_and_requires_absolute_program() {
    let raw = r#"{"version":1,"program":"relative","required_sources":["gmail"],"unknown":true}"#;
    assert!(HeartbeatPreflightConfig::parse(raw).is_err());

    let raw = r#"{"version":1,"program":"relative","required_sources":["gmail"]}"#;
    assert!(HeartbeatPreflightConfig::parse(raw).is_err());
}

#[test]
fn production_macos_policy_requires_both_code_identity_pins() {
    let (directory, path) = temp_script("code-identity-config", "exit 0");
    let mut candidate = config(path.to_string_lossy().into_owned(), vec![]);
    assert!(candidate.validate_macos_identity_pins(true).is_err());

    candidate.macos_designated_requirement = Some("identifier com.example.gateway".into());
    assert!(candidate.validate_macos_identity_pins(true).is_err());
    candidate.macos_team_identifier = Some("TEAMIDENTIFIER".into());
    candidate
        .validate_macos_identity_pins(true)
        .expect("both pins satisfy the production config gate");
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn codesign_test_requirement_is_passed_as_one_expression() {
    assert_eq!(
        codesign_requirement_arg("identifier \"com.example.gateway\" and anchor apple generic"),
        "-R=identifier \"com.example.gateway\" and anchor apple generic"
    );
}

#[test]
fn production_path_policy_rejects_non_root_owned_components() {
    use std::os::unix::fs::MetadataExt;

    let (directory, path) = temp_script("root-owner", "exit 0");
    if std::fs::metadata(&path).expect("helper metadata").uid() != 0 {
        let error = validate_program_path_with_ownership(&path, true)
            .expect_err("production path must be all-root-owned");
        assert!(matches!(error, HeartbeatPreflightError::UnsafeProgram(_)));
    }
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn policy_activates_only_for_exact_target_pubkey() {
    let (directory, path) = temp_script("target", "exit 0");
    let owner_pin =
        hash_file(&std::fs::File::open(&path).expect("open helper")).expect("hash helper");
    let raw = serde_json::json!({
        "version": 1,
        "target_agent_pubkey": "a".repeat(64),
        "target_channel": "5e06068b-0c7d-444c-9a48-080c45b65931",
        "declaration_manifest_digest": "d".repeat(64),
        "program": path,
        "program_sha256": owner_pin,
        "required_sources": [{
            "source": "gmail",
            "account": "owner@example.com",
            "scope": "inbox",
            "policy_id": "gmail.required"
        }],
        "ledger_instance_id": "ledger-primary",
    })
    .to_string();

    assert!(
        HeartbeatPreflightConfig::parse_for_agent(&raw, &"b".repeat(64))
            .expect("other target is valid")
            .is_none()
    );
    assert!(
        HeartbeatPreflightConfig::parse_for_agent(&raw, &"a".repeat(64))
            .expect("target config parses")
            .is_some()
    );

    let malformed_for_target = serde_json::json!({
        "target_agent_pubkey": "a".repeat(64),
        "program": "not-an-absolute-program",
    })
    .to_string();
    assert!(
        HeartbeatPreflightConfig::parse_for_agent(&malformed_for_target, &"b".repeat(64))
            .expect("another agent must ignore non-target policy details")
            .is_none()
    );
    assert!(
        HeartbeatPreflightConfig::parse_for_agent(&malformed_for_target, &"a".repeat(64)).is_err()
    );
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn python_gateway_request_fixture_matches_the_exact_rust_wire_contract() {
    const FIXTURE: &str = include_str!("../../tests/fixtures/gateway_heartbeat_request_v1.json");
    assert_eq!(FIXTURE.len(), 1_164, "fixture must retain its terminal LF");
    assert_eq!(
        hex::encode(Sha256::digest(FIXTURE.as_bytes())),
        "20f95a5e342168f459819730e3ee5b95d31b8d975719e6a4a66fa6b91e29e5a5"
    );

    let fixture: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("parse canonical Python fixture");
    let required_sources: Vec<RequiredSourceScope> = serde_json::from_value(
        fixture
            .get("required_sources")
            .expect("fixture required sources")
            .clone(),
    )
    .expect("Python source rows must match the strict four-field Rust schema");
    let request = HeartbeatPreflightRequest {
        version: 1,
        kind: "buzz_heartbeat_preflight",
        turn_id: "heartbeat-turn-0001",
        invocation_id: "heartbeat-turn-0001",
        target_agent_pubkey: "62f23f0a26022c4b95bbbf70999a3a55382c6f44184eb43aed28054d4774d87d",
        target_channel: "5e06068b-0c7d-444c-9a48-080c45b65931",
        declaration_manifest_digest:
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        requested_at: "2026-08-11T15:00:00Z".into(),
        required_sources: &required_sources,
        ledger_instance_id: "ledger-instance-0001",
    };
    assert_eq!(
        serde_json::to_value(request).expect("serialize Rust wire request"),
        fixture
    );

    let mut extended_source = fixture["required_sources"][0].clone();
    extended_source
        .as_object_mut()
        .expect("fixture source object")
        .insert("zone".into(), "cloud".into());
    assert!(serde_json::from_value::<RequiredSourceScope>(extended_source).is_err());
}

#[test]
fn forwarding_denies_all_secrets_and_allows_only_gateway_ipc_metadata() {
    let (directory, path) = temp_script("env", "exit 0");
    let base = config(path.to_string_lossy().into_owned(), vec![]);
    for denied in [
        "BUZZ_PRIVATE_KEY",
        "NOSTR_PRIVATE_KEY",
        "BUZZ_AUTH_TAG",
        "BUZZ_RELAY_URL",
        "BUZZ_ACP_REQUIRED_AGENT_OWNER",
        "BUZZ_RELAY_TOKEN",
        "BUZZ_AUTH_SECRET",
        "BUZZ_ACP_HEARTBEAT_PREFLIGHT_CONFIG",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "PROVIDER_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "SOME_PASSWORD",
    ] {
        let mut candidate = base.clone();
        candidate.forward_env = vec![denied.into()];
        assert!(candidate.validate().is_err(), "{denied} must be denied");
    }

    for allowed in SAFE_FORWARDED_ENV_KEYS {
        let mut candidate = base.clone();
        candidate.forward_env = vec![(*allowed).into()];
        candidate.validate().expect("safe IPC metadata key");

        let mut wrong_case = base.clone();
        wrong_case.forward_env = vec![allowed.to_ascii_lowercase()];
        assert!(wrong_case.validate().is_err());
    }
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[test]
fn model_env_scrub_removes_ambient_and_explicit_case_variants() {
    let mut command = Command::new("ignored");
    command
        .env("buzz_heartbeat_gateway_socket", "agent-controlled")
        .env("buzz_acp_heartbeat_interval", "1")
        .env("buzz_acp_required_agent_owner", "a".repeat(64));
    scrub_agent_subprocess_env(&mut command);

    let env: BTreeMap<_, _> = command
        .as_std()
        .get_envs()
        .map(|(key, value)| (key.to_os_string(), value.map(ToOwned::to_owned)))
        .collect();
    assert_eq!(
        env.get(std::ffi::OsStr::new("buzz_heartbeat_gateway_socket")),
        Some(&None)
    );
    assert_eq!(
        env.get(std::ffi::OsStr::new("buzz_acp_heartbeat_interval")),
        Some(&None)
    );
    assert_eq!(
        env.get(std::ffi::OsStr::new("buzz_acp_required_agent_owner")),
        Some(&None)
    );
}

#[tokio::test]
async fn execution_revalidates_target_and_rejects_constructed_denied_env() {
    let (directory, path) = temp_script("execution-revalidation", "exit 99");
    let base = config(path.to_string_lossy().into_owned(), vec![]);

    assert!(matches!(
        run_heartbeat_preflight(&base, &"b".repeat(64), "turn-wrong-target").await,
        Err(HeartbeatPreflightError::TargetAgentMismatch)
    ));

    let mut denied = base;
    denied.forward_env = vec!["BUZZ_PRIVATE_KEY".into()];
    assert!(matches!(
        run_heartbeat_preflight(&denied, TARGET_AGENT_PUBKEY, "turn-denied-env").await,
        Err(HeartbeatPreflightError::InvalidConfig(_))
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn replacement_symlink_and_unsafe_modes_are_rejected() {
    use std::os::unix::fs::symlink;

    let (replacement_dir, replacement_path) = temp_script(
        "replacement",
        &echo_result_body("checked", "\"item_count\":0"),
    );
    let replacement_config = config(replacement_path.to_string_lossy().into_owned(), vec![]);
    let replacement = replacement_dir.join("replacement-helper");
    std::fs::write(&replacement, "#!/bin/sh\nexit 0\n").expect("write replacement");
    let mut permissions = std::fs::metadata(&replacement)
        .expect("replacement metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&replacement, permissions).expect("replacement mode");
    std::fs::rename(&replacement, &replacement_path).expect("replace executable path");
    assert!(matches!(
        run_heartbeat_preflight(&replacement_config, TARGET_AGENT_PUBKEY, "turn-replaced").await,
        Err(HeartbeatPreflightError::ProgramIdentityMismatch)
    ));

    let (symlink_dir, symlink_target) = temp_script("symlink", "exit 0");
    let symlink_path = symlink_dir.join("linked-helper");
    symlink(&symlink_target, &symlink_path).expect("create symlink");
    let mut symlink_config = config(symlink_target.to_string_lossy().into_owned(), vec![]);
    symlink_config.program = symlink_path.to_string_lossy().into_owned();
    assert!(matches!(
        run_heartbeat_preflight(&symlink_config, TARGET_AGENT_PUBKEY, "turn-symlink").await,
        Err(HeartbeatPreflightError::UnsafeProgram(_))
    ));

    let symlinked_parent = symlink_dir.join("linked-parent");
    symlink(&symlink_dir, &symlinked_parent).expect("create parent-component symlink");
    let mut component_config = config(symlink_target.to_string_lossy().into_owned(), vec![]);
    component_config.program = symlinked_parent
        .join(symlink_target.file_name().expect("helper filename"))
        .to_string_lossy()
        .into_owned();
    assert!(matches!(
        run_heartbeat_preflight(
            &component_config,
            TARGET_AGENT_PUBKEY,
            "turn-component-symlink"
        )
        .await,
        Err(HeartbeatPreflightError::UnsafeProgram(_))
    ));

    let (mode_dir, mode_path) = temp_script("mode", "exit 0");
    let mode_config = config(mode_path.to_string_lossy().into_owned(), vec![]);
    let mut mode_permissions = std::fs::metadata(&mode_path)
        .expect("mode metadata")
        .permissions();
    mode_permissions.set_mode(0o722);
    std::fs::set_permissions(&mode_path, mode_permissions).expect("unsafe executable mode");
    assert!(matches!(
        run_heartbeat_preflight(&mode_config, TARGET_AGENT_PUBKEY, "turn-mode").await,
        Err(HeartbeatPreflightError::UnsafeProgram(_))
    ));

    let (parent_dir, parent_path) = temp_script("parent-mode", "exit 0");
    let parent_config = config(parent_path.to_string_lossy().into_owned(), vec![]);
    let mut parent_permissions = std::fs::metadata(&parent_dir)
        .expect("parent metadata")
        .permissions();
    parent_permissions.set_mode(0o777);
    std::fs::set_permissions(&parent_dir, parent_permissions).expect("unsafe parent mode");
    assert!(matches!(
        run_heartbeat_preflight(&parent_config, TARGET_AGENT_PUBKEY, "turn-parent-mode").await,
        Err(HeartbeatPreflightError::UnsafeProgram(_))
    ));

    // Restore directory/file modes so cleanup is deterministic.
    std::fs::set_permissions(&parent_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore parent mode");
    std::fs::set_permissions(&mode_path, std::fs::Permissions::from_mode(0o700))
        .expect("restore executable mode");
    for directory in [replacement_dir, symlink_dir, mode_dir, parent_dir] {
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }
}

#[test]
fn immediate_pre_exec_recheck_detects_toctou_replacement() {
    let (directory, path) = temp_script("toctou", &echo_result_body("checked", "\"item_count\":0"));
    let config = config(path.to_string_lossy().into_owned(), vec![]);
    let verified = verify_program(&config).expect("initial program verification");

    let replacement = directory.join("toctou-replacement");
    assert!(matches!(
        verified.recheck_before_exec_with_hook(&config, || {
            std::fs::write(&replacement, "#!/bin/sh\nexit 0\n").expect("write TOCTOU replacement");
            std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700))
                .expect("secure replacement mode");
            std::fs::rename(&replacement, &path).expect("replace after initial verification");
        }),
        Err(HeartbeatPreflightError::ProgramIdentityMismatch)
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn commit_mismatch_fails_closed() {
    let body = echo_result_body("checked", "\"item_count\":0").replace(
        "\\\"remote_readback_commit\\\":\\\"1111111111111111111111111111111111111111\\\"",
        "\\\"remote_readback_commit\\\":\\\"2222222222222222222222222222222222222222\\\"",
    );
    let (directory, path) = temp_script("commit-mismatch", &body);
    assert!(matches!(
        run_heartbeat_preflight(
            &config(path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-commit"
        )
        .await,
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn invalid_or_omitted_commit_fields_fail_closed() {
    let valid = echo_result_body("checked", "\"item_count\":0");
    let invalid_authority = valid.replacen(
        "\\\"authority_commit\\\":\\\"1111111111111111111111111111111111111111\\\"",
        "\\\"authority_commit\\\":\\\"not-an-object-id\\\"",
        1,
    );
    let (invalid_dir, invalid_path) = temp_script("invalid-commit", &invalid_authority);
    assert!(matches!(
        run_heartbeat_preflight(
            &config(invalid_path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-invalid-commit"
        )
        .await,
        Err(HeartbeatPreflightError::InvalidResult(_))
    ));

    let omitted_readback = valid.replace(
        ",\\\"remote_readback_commit\\\":\\\"1111111111111111111111111111111111111111\\\"",
        "",
    );
    let (omitted_dir, omitted_path) = temp_script("omitted-commit", &omitted_readback);
    assert!(matches!(
        run_heartbeat_preflight(
            &config(omitted_path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-omitted-commit"
        )
        .await,
        Err(HeartbeatPreflightError::MalformedResult)
    ));

    let unknown_field = valid.replacen(
        "\\\"version\\\":1",
        "\\\"version\\\":1,\\\"unexpected\\\":true",
        1,
    );
    let (unknown_dir, unknown_path) = temp_script("unknown-result-field", &unknown_field);
    assert!(matches!(
        run_heartbeat_preflight(
            &config(unknown_path.to_string_lossy().into_owned(), vec![]),
            TARGET_AGENT_PUBKEY,
            "turn-unknown-field"
        )
        .await,
        Err(HeartbeatPreflightError::MalformedResult)
    ));

    for directory in [invalid_dir, omitted_dir, unknown_dir] {
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }
}

#[tokio::test]
async fn timeout_kills_descendant_process_group() {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let (directory, path) = temp_script(
        "descendant",
        "/bin/sleep 30 &\nprintf '%s' \"$!\" > \"$1\"\nexit 0",
    );
    let pid_path = directory.join("descendant.pid");
    let mut timeout_config = config(
        path.to_string_lossy().into_owned(),
        vec![pid_path.to_string_lossy().into_owned()],
    );
    timeout_config.timeout_ms = 3_000;
    assert!(matches!(
        run_heartbeat_preflight(&timeout_config, TARGET_AGENT_PUBKEY, "turn-descendant").await,
        Err(HeartbeatPreflightError::Timeout(3_000))
    ));
    let pid: i32 = std::fs::read_to_string(&pid_path)
        .expect("descendant pid")
        .parse()
        .expect("numeric descendant pid");
    let mut gone = false;
    for _ in 0..80 {
        if kill(Pid::from_raw(pid), None).is_err() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(gone, "preflight descendant {pid} survived timeout");
    std::fs::remove_dir_all(directory).expect("remove test directory");
}

#[tokio::test]
async fn completed_preflight_outcomes_kill_detached_descendants() {
    let cases = [
        (
            "descendant-success",
            echo_result_body("checked", "\"item_count\":0"),
            None,
        ),
        (
            "descendant-malformed",
            "printf 'not-json'".to_string(),
            Some("malformed"),
        ),
        ("descendant-nonzero", "exit 9".to_string(), Some("nonzero")),
    ];
    let mut fixtures = Vec::new();

    for (name, terminal_body, expected_error) in cases {
        let body = format!(
            "marker=$1\n( /bin/sleep 0.25; : > \"$marker\" ) </dev/null >/dev/null 2>&1 &\n{terminal_body}"
        );
        let (directory, path) = temp_script(name, &body);
        let marker = directory.join("descendant-survived");
        let result = run_heartbeat_preflight(
            &config(
                path.to_string_lossy().into_owned(),
                vec![marker.to_string_lossy().into_owned()],
            ),
            TARGET_AGENT_PUBKEY,
            name,
        )
        .await;
        match expected_error {
            None => {
                result.expect("valid terminal result");
            }
            Some("malformed") => {
                assert!(matches!(
                    result,
                    Err(HeartbeatPreflightError::MalformedResult)
                ));
            }
            Some("nonzero") => {
                assert!(matches!(
                    result,
                    Err(HeartbeatPreflightError::UnsuccessfulExit)
                ));
            }
            Some(unexpected) => panic!("unexpected test case {unexpected}"),
        }
        fixtures.push((directory, marker));
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    for (directory, marker) in fixtures {
        assert!(
            !marker.exists(),
            "preflight descendant escaped after terminal outcome: {}",
            marker.display()
        );
        std::fs::remove_dir_all(directory).expect("remove test directory");
    }
}
