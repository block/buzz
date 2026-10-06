use super::*;
use std::os::unix::fs::PermissionsExt;

fn owned_pi(acp: AcpClient, protocol_version: u32) -> OwnedAgent {
    OwnedAgent {
        index: 0,
        acp,
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: None,
        agent_name: BUZZ_PI_ACP_NAME.into(),
        goose_system_prompt_supported: None,
        protocol_version,
    }
}

fn fixture_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("buzz pi transport {}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn script_at(dir: &std::path::Path, script: &str) -> std::path::PathBuf {
    let path = dir.join(BUZZ_PI_ACP_NAME);
    std::fs::write(&path, format!("#!/bin/bash\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[tokio::test]
async fn pi_composed_prompt_uses_meta_without_capability_negotiation() {
    for version in [1, 2] {
        let dir = fixture_dir();
        let init = serde_json::json!({"jsonrpc":"2.0", "id":0, "result": {
            "protocolVersion":version, "agentInfo":{"name":"buzz-pi-acp", "version":"fixture"}, "agentCapabilities":{}
        }});
        let path = script_at(
            &dir,
            &format!(
                r#"
            printf '%s\n' "$@" > '{dir}/args'
            read -r request
            echo '{init}'
            read -r request
            printf '%s\n' "$request" > '{dir}/request'
            echo '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"fixture"}}}}'
            read -r request
        "#,
                dir = dir.display()
            ),
        );
        let mut acp = AcpClient::spawn(path.to_str().unwrap(), &[], &[], false)
            .await
            .unwrap();
        acp.initialize().await.unwrap();
        let mut agent = owned_pi(acp, version);
        assert!(agent.has_system_prompt_support());
        let mut ctx = tests::make_prompt_context_no_owner();
        ctx.base_prompt = Some("BUZZ_BASE".into());
        ctx.system_prompt = Some("BUZZ_PERSONA".into());
        ctx.team_instructions = Some("BUZZ_TEAM".into());
        ctx.session_title = Some("Pi fixture".into());
        let core = "<core-memory>BUZZ_CORE</core-memory>";
        let canvas = "<channel-canvas>BUZZ_CANVAS</channel-canvas>";
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            Some(core),
            NewSessionChannelContext {
                huddle_instructions: Some("BUZZ_HUDDLE"),
                canvas: Some(canvas),
                name: Some("channel"),
                scope: None,
                channel_type: None,
            },
        )
        .await
        .unwrap();
        agent.acp.shutdown().await;
        let request: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("request")).unwrap()).unwrap();
        let params = &request["params"];
        assert!(params.get("systemPrompt").is_none());
        assert!(params["_meta"]["sessionTitle"]
            .as_str()
            .unwrap()
            .contains("Pi fixture"));
        let standing = crate::queue::StandingContext {
            base_prompt: ctx.base_prompt.as_deref(),
            system_prompt: ctx.system_prompt.as_deref(),
            team_instructions: ctx.team_instructions.as_deref(),
            agent_core: Some(core),
            huddle_instructions: Some("BUZZ_HUDDLE"),
            agent_canvas: Some(canvas),
        };
        let user = prepend_standing_for_legacy(2, &standing, "EVENT");
        for marker in [
            "BUZZ_BASE",
            "BUZZ_PERSONA",
            "BUZZ_TEAM",
            "BUZZ_CORE",
            "BUZZ_HUDDLE",
            "BUZZ_CANVAS",
        ] {
            assert_eq!(
                params["_meta"]["systemPrompt"]
                    .as_str()
                    .unwrap()
                    .matches(marker)
                    .count(),
                1
            );
            assert!(!user.contains(marker));
        }

        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        assert_eq!(
            args,
            format!(
                "--\n--skill\n{}\n",
                std::env::current_dir()
                    .unwrap()
                    .join(".agents/skills")
                    .display()
            )
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn pi_launch_preserves_existing_skills_in_explicit_workspace() {
    const FIXTURE_ENV: &str = "BUZZ_TEST_PI_LAUNCH_WORKSPACE";
    if let Some(dir) = std::env::var_os(FIXTURE_ENV) {
        let path = std::path::PathBuf::from(dir).join(BUZZ_PI_ACP_NAME);
        let mut client = AcpClient::spawn(
            path.to_str().unwrap(),
            &["--".into(), "--skill".into(), "/extra skills".into()],
            &[],
            false,
        )
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), client.initialize())
            .await
            .unwrap()
            .unwrap_err();
        client.shutdown().await;
        return;
    }

    let dir = fixture_dir();
    let workspace = dir.join("chosen workspace");
    std::fs::create_dir_all(workspace.join(".agents/skills")).unwrap();
    // Canonicalize macOS's /var -> /private/var before comparing with getcwd.
    let workspace = workspace.canonicalize().unwrap();
    script_at(
        &dir,
        r#"printf '%s\n' "$@" > "$(dirname "$0")/args"
pwd -P > "$(dirname "$0")/cwd""#,
    );
    // Re-enter only this test in a separate process so parallel tests never
    // share a mutated CWD. This models Desktop setting its harness child's CWD.
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pool::pi_prompt_tests::pi_launch_preserves_existing_skills_in_explicit_workspace",
                "--nocapture",
            ])
            .kill_on_drop(true)
            .env(FIXTURE_ENV, &dir)
            .current_dir(&workspace)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("cwd")).unwrap().trim(),
        workspace.to_str().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("args")).unwrap(),
        format!(
            "--\n--skill\n/extra skills\n--skill\n{}\n",
            workspace.join(".agents/skills").display()
        )
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn upstream_pi_acp_launch_does_not_receive_managed_skills() {
    let dir = fixture_dir();
    let path = dir.join("pi-acp");
    std::fs::write(
        &path,
        format!(
            r#"#!/bin/bash
printf '%s' "$*" > '{dir}/args'
read -r request
echo '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentInfo":{{"name":"pi-acp","version":"fixture"}},"agentCapabilities":{{}}}}}}'
read -r request
"#,
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();

    let mut client = AcpClient::spawn(path.to_str().unwrap(), &[], &[], false)
        .await
        .unwrap();
    client.initialize().await.unwrap();
    client.shutdown().await;

    assert_eq!(std::fs::read_to_string(dir.join("args")).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
#[ignore = "requires BUZZ_TEST_PI_ACP pointing to a built fork and Pi on PATH"]
async fn real_pi_preserves_buzz_prompt_and_launch_skills_on_restore() {
    use base64::Engine;
    let adapter = std::env::var("BUZZ_TEST_PI_ACP").expect("set BUZZ_TEST_PI_ACP");
    let dir = fixture_dir();
    let home = dir.join("home");
    let workspace = dir.join("workspace");
    let skill = dir.join("extra skills");
    for path in [&home, &workspace, &skill] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: buzz-fixture\ndescription: BUZZ_SKILL_MARKER\n---\nSynthetic instructions.\n",
    )
    .unwrap();
    let path = script_at(&dir, &format!(
        "export HOME='{}' PI_CODING_AGENT_DIR='{}/agent' ANTHROPIC_API_KEY=synthetic-test-key PI_ACP_PI_COMMAND=pi\nexec node '{}' \"$@\"",
        home.display(), dir.display(), adapter.replace('\'', "'\\''")
    ));
    let args = vec![
        "--".into(),
        "--offline".into(),
        "--no-extensions".into(),
        "--no-context-files".into(),
        "--skill".into(),
        skill.to_string_lossy().into_owned(),
    ];
    let mut acp = AcpClient::spawn(path.to_str().unwrap(), &args, &[], false)
        .await
        .unwrap();
    acp.initialize().await.unwrap();
    let mut agent = owned_pi(acp, 1);
    assert!(agent.has_system_prompt_support());
    let mut ctx = tests::make_prompt_context_no_owner();
    ctx.cwd = workspace.to_string_lossy().into_owned();
    ctx.base_prompt = Some("BUZZ_BASE".into());
    ctx.system_prompt = Some("BUZZ_PERSONA".into());
    ctx.team_instructions = Some("BUZZ_TEAM".into());
    ctx.session_title = Some("Pi fixture".into());
    let id = create_session_and_apply_model(
        &mut agent,
        &ctx,
        Some("<core-memory>BUZZ_CORE</core-memory>"),
        NewSessionChannelContext {
            huddle_instructions: Some("BUZZ_HUDDLE"),
            canvas: Some("<channel-canvas>BUZZ_CANVAS</channel-canvas>"),
            name: None,
            scope: None,
            channel_type: None,
        },
    )
    .await
    .unwrap();
    let metadata = std::fs::read_dir(home.join(".pi/buzz-pi-acp/sessions"))
        .unwrap()
        .find_map(|entry| {
            let contents = std::fs::read_to_string(entry.ok()?.path()).ok()?;
            let metadata: serde_json::Value = serde_json::from_str(&contents).ok()?;
            (metadata["session"]["sessionId"].as_str() == Some(id.as_str())).then_some(metadata)
        })
        .expect("adapter should persist metadata for the new session");
    let transcript = metadata["session"]["sessionFile"].as_str().unwrap();
    let timestamp = "2026-01-01T00:00:00.000Z";
    std::fs::create_dir_all(std::path::Path::new(transcript).parent().unwrap()).unwrap();
    std::fs::write(transcript, format!("{}\n{}\n",
        serde_json::json!({"type":"session","version":3,"id":id,"timestamp":timestamp,"cwd":ctx.cwd}),
        serde_json::json!({"type":"message","id":"00000001","parentId":null,"timestamp":timestamp,"message":{"role":"user","content":[{"type":"text","text":"fixture"}],"timestamp":1767225600000u64}})
    )).unwrap();
    agent
        .acp
        .session_new_full(
            &ctx.cwd,
            vec![],
            Some(SystemPromptTransport::PiMeta("OTHER_SESSION")),
            None,
        )
        .await
        .unwrap();
    for restart in [false, true] {
        if restart {
            agent.acp.shutdown().await;
            agent.acp = AcpClient::spawn(path.to_str().unwrap(), &args, &[], false)
                .await
                .unwrap();
            agent.acp.initialize().await.unwrap();
        }
        agent
            .acp
            .session_prompt_with_idle_timeout(
                &id,
                "/export",
                Duration::from_secs(15),
                Duration::from_secs(30),
            )
            .await
            .unwrap();
        let html =
            std::fs::read_to_string(workspace.join(format!("pi-session-{id}.html"))).unwrap();
        let encoded = html
            .split("id=\"session-data\"")
            .nth(1)
            .unwrap()
            .split_once('>')
            .unwrap()
            .1
            .split("</script>")
            .next()
            .unwrap()
            .trim();
        let data: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();
        let prompt = data["systemPrompt"].as_str().unwrap();
        for marker in [
            "BUZZ_BASE",
            "BUZZ_PERSONA",
            "BUZZ_TEAM",
            "BUZZ_CORE",
            "BUZZ_HUDDLE",
            "BUZZ_CANVAS",
            "BUZZ_SKILL_MARKER",
        ] {
            assert_eq!(
                prompt.matches(marker).count(),
                1,
                "{marker}, restart={restart}"
            );
        }
        assert!(!prompt.contains("OTHER_SESSION"));
        assert!(!prompt.contains("You are an expert coding assistant"));
    }
    agent.acp.shutdown().await;
    std::fs::remove_dir_all(dir).unwrap();
}
