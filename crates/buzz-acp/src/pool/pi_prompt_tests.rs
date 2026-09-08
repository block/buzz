use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn pi_agent(fail: bool) -> (Fixture, OwnedAgent) {
    let fixture =
        Fixture(std::env::temp_dir().join(format!("buzz-pi-pool-test-{}", Uuid::new_v4())));
    fs::create_dir(&fixture.0).unwrap();
    let script = fixture.0.join("pi-acp");
    // This adapter fixture consumes the launcher's production pending snapshot
    // at session/new. It deliberately advertises no ACP system-prompt support.
    fs::write(&script, r#"#!/bin/sh
set -eu
count=0
while IFS= read -r line; do
  dir=$(dirname "$PI_ACP_PI_COMMAND")
  token=$(cat "$dir/pending")
  cp "$dir/$token.md" "$PI_TEST_CAPTURE/prompt-$count"
  printf '%s\n' "$dir/$token.md" > "$PI_TEST_CAPTURE/path-$count"
  printf '%s\n' "$line" > "$PI_TEST_CAPTURE/request-$count"
  if [ "$PI_TEST_FAIL" = true ]; then
    printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32603,"message":"native launch failed"}}\n' "$count"
  else
    printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"00000000-0000-4000-8000-%012d"}}\n' "$count" "$count"
  fi
  count=$((count + 1))
done
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let acp = AcpClient::spawn(
        script.to_str().unwrap(),
        &[],
        &[
            (
                "PI_TEST_CAPTURE".into(),
                fixture.0.to_string_lossy().into_owned(),
            ),
            ("PI_TEST_FAIL".into(), fail.to_string()),
        ],
        false,
    )
    .await
    .unwrap();
    let agent = OwnedAgent {
        index: 0,
        acp,
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: None,
        agent_name: "pi-acp".into(),
        goose_system_prompt_supported: None,
        protocol_version: 1,
    };
    (fixture, agent)
}

async fn create(
    agent: &mut OwnedAgent,
    ctx: &PromptContext,
    core: Option<&str>,
) -> Result<String, AcpError> {
    create_session_and_apply_model(
        agent,
        ctx,
        core,
        NewSessionChannelContext {
            huddle_instructions: None,
            canvas: None,
            name: None,
            scope: None,
            channel_type: None,
        },
    )
    .await
}

#[tokio::test]
async fn pi_session_uses_composed_system_prompt_and_skips_legacy_first_turn() {
    let (fixture, mut agent) = pi_agent(false).await;
    let mut ctx = tests::make_prompt_context_no_owner();
    ctx.base_prompt = Some("platform instructions".into());
    ctx.system_prompt = Some("### Quality bar\nBe precise.".into());
    ctx.team_instructions = Some("team instructions".into());
    let core = "<core-memory>\nsession memory\n</core-memory>";
    let id = create(&mut agent, &ctx, Some(core)).await.unwrap();
    let system = fs::read_to_string(fixture.0.join("prompt-0")).unwrap();
    let mut last = 0;
    for section in [
        "<base>",
        "<agent-instructions>",
        "<team-instructions>",
        "<core-memory>",
    ] {
        assert_eq!(system.matches(section).count(), 1, "{section}");
        let pos = system.find(section).unwrap();
        assert!(pos >= last);
        last = pos;
    }
    assert!(system.contains("### Quality bar\nBe precise."));
    assert!(system.contains("session memory"));
    let wire: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture.0.join("request-0")).unwrap()).unwrap();
    assert!(wire["params"].get("systemPrompt").is_none());
    assert!(agent.has_system_prompt_support());
    let channel_id = Uuid::new_v4();
    let batch = crate::queue::FlushBatch {
        scope: SessionScope::Conversation { channel_id },
        channel_id,
        events: vec![crate::queue::BatchEvent {
            event: nostr::EventBuilder::new(nostr::Kind::Custom(9), "hello")
                .sign_with_keys(&nostr::Keys::generate())
                .unwrap(),
            prompt_tag: "message".into(),
            received_at: std::time::Instant::now(),
        }],
        cancelled_events: vec![],
        cancel_reason: None,
    };
    let user = crate::queue::format_prompt(
        &batch,
        &crate::queue::FormatPromptArgs {
            has_system_prompt_support: agent.has_system_prompt_support(),
            base_prompt: ctx.base_prompt.as_deref(),
            system_prompt: ctx.system_prompt.as_deref(),
            team_instructions: ctx.team_instructions.as_deref(),
            agent_core: Some(core),
            ..Default::default()
        },
    )
    .join("\n\n");
    assert!(user.starts_with("<context>"), "{user}");
    for tag in [
        "<base>",
        "<agent-instructions>",
        "<core-memory>",
        "<team-instructions>",
    ] {
        assert!(!user.contains(tag));
    }
    let path = fs::read_to_string(fixture.0.join("path-0")).unwrap();
    assert!(std::path::Path::new(path.trim()).is_file());
    agent.state.heartbeat_session = Some(id);
    agent.state.invalidate(&PromptSource::Heartbeat);
    assert!(!std::path::Path::new(path.trim()).exists());
    agent.acp.shutdown().await;
}

#[tokio::test]
async fn pi_base_disabled_still_delivers_profile_and_memory_in_system_role() {
    let (fixture, mut agent) = pi_agent(false).await;
    let mut ctx = tests::make_prompt_context_no_owner();
    ctx.system_prompt = Some("profile only".into());
    create(&mut agent, &ctx, Some("<core-memory>memory</core-memory>"))
        .await
        .unwrap();
    let system = fs::read_to_string(fixture.0.join("prompt-0")).unwrap();
    assert!(!system.contains("<base>"));
    assert!(system.contains("<agent-instructions>\nprofile only\n</agent-instructions>"));
    assert!(system.contains("<core-memory>"));
    agent.acp.shutdown().await;
}

#[tokio::test]
async fn pi_failed_create_does_not_keep_snapshot_or_live_adapter() {
    let (fixture, mut agent) = pi_agent(true).await;
    let ctx = tests::make_prompt_context_no_owner();
    assert!(create(&mut agent, &ctx, None).await.is_err());
    assert!(agent.state.pi_prompts.is_empty());
    let path = fs::read_to_string(fixture.0.join("path-0")).unwrap();
    assert!(!std::path::Path::new(path.trim()).exists());
    assert!(create(&mut agent, &ctx, None).await.is_err());
}

#[tokio::test]
async fn pi_snapshots_follow_all_scope_invalidation_paths() {
    for invalidation in ["scope", "channel", "all"] {
        let (fixture, mut agent) = pi_agent(false).await;
        let ctx = tests::make_prompt_context_no_owner();
        let channel_id = Uuid::new_v4();
        let scope = SessionScope::Thread {
            channel_id,
            root_event_id: "a".repeat(64),
        };
        let sibling = SessionScope::Thread {
            channel_id: Uuid::new_v4(),
            root_event_id: "b".repeat(64),
        };
        for scope in [&scope, &sibling] {
            let id = create(&mut agent, &ctx, None).await.unwrap();
            agent.state.sessions.insert(scope.clone(), id);
        }
        match invalidation {
            "scope" => {
                agent.state.invalidate_scope(&scope);
            }
            "channel" => {
                agent.state.invalidate_channel(&channel_id);
            }
            _ => agent.state.invalidate_all(),
        }
        let path = |index| fs::read_to_string(fixture.0.join(format!("path-{index}"))).unwrap();
        assert!(!std::path::Path::new(path(0).trim()).exists());
        assert_eq!(
            std::path::Path::new(path(1).trim()).exists(),
            invalidation != "all"
        );
        agent.acp.shutdown().await;
    }
}
