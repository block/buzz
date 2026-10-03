//! Subprocess isolation keeps the harness environment immutable in parallel tests.
use super::PREFIX_ENV;
use crate::{
    acp::AcpClient,
    config::{CliArgs, Config},
    usage::StandardAdapterKind,
};
use clap::Parser;
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

const CHILD: &str = "BUZZ_LAUNCH_TEST_CHILD";
const LITERAL: &str = "policy with spaces, ; $(not-a-shell)";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("buzz launch {}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let wrapper = dir.join("wrapper.py");
        fs::write(
            &wrapper,
            r#"import json, os, sys
with open('wrapper.jsonl', 'a') as log:
    log.write(json.dumps(sys.argv[1:]) + '\n')
os.execv(sys.argv[2], sys.argv[2:])
"#,
        )
        .unwrap();
        for name in [
            "goose",
            "codex-acp",
            "claude-agent-acp",
            "hermes-acp",
            "buzz-pi-acp",
        ] {
            let path = dir.join(name);
            fs::write(
                &path,
                r#"#!/usr/bin/env python3
import json, os, sys
with open('worker-started', 'a') as log:
    log.write('started\n')
for line in sys.stdin:
    request = json.loads(line)
    result = {'protocolVersion': 1, 'argv': sys.argv[1:],
              'hermes': os.environ.get('HERMES_ACP_SKIP_CONFIGURED_MCP'),
              'codex': os.environ.get('CODEX_CONFIG'),
              'prefix': os.environ.get('BUZZ_ACP_LAUNCH_PREFIX'),
              'sessionId': 'ses_test', 'meta': request.get('params', {}).get('_meta')}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
"#,
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        // A CLI new enough for `--thinking-display`, so only the prefix can
        // keep summaries off.
        let claude = dir.join("claude");
        fs::write(&claude, "#!/bin/sh\necho '2.1.284 (Claude Code)'\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir)
    }
    fn prefix(&self) -> String {
        json!([
            "/usr/bin/env",
            "python3",
            self.0.join("wrapper.py"),
            LITERAL
        ])
        .to_string()
    }
    fn run(&self, mode: &str, prefix: Option<&str>) {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("BUZZ_") {
                cmd.env_remove(name);
            }
        }
        cmd.args(["--exact", "acp::launch::tests::spawn_child", "--nocapture"])
            .env(CHILD, mode)
            .env_remove("CODEX_CONFIG")
            .env_remove("HERMES_ACP_SKIP_CONFIGURED_MCP")
            .env("CLAUDE_CODE_EXECUTABLE", self.0.join("claude"))
            .current_dir(&self.0);
        if let Some(prefix) = prefix {
            cmd.env(PREFIX_ENV, prefix);
        }
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn wrapper_preserves_worker_identity_and_runs_on_each_spawn() {
    for wrapped in [false, true] {
        let f = Fixture::new();
        let prefix = f.prefix();
        f.run("success", wrapped.then_some(prefix.as_str()));
        assert_eq!(
            fs::read_to_string(f.0.join("worker-started"))
                .unwrap()
                .lines()
                .count(),
            10
        );
        if wrapped {
            let calls: Vec<Value> = fs::read_to_string(f.0.join("wrapper.jsonl"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(calls.len(), 10);
            for call in calls {
                assert_eq!(call[0], LITERAL);
            }
        } else {
            assert!(!f.0.join("wrapper.jsonl").exists());
        }
    }
}

#[test]
fn invalid_or_unavailable_wrapper_never_falls_back_to_worker() {
    let f = Fixture::new();
    for prefix in [
        "",
        "[]",
        "null",
        "{}",
        "[1]",
        "[\"\"]",
        "[\"relative\"]",
        "[\"/tmp/\\u0000\"]",
        "[\"/missing-buzz-launcher\"]",
    ] {
        f.run("failure", Some(prefix));
        assert!(!f.0.join("worker-started").exists());
    }
}

#[tokio::test]
async fn spawn_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    // Bound a broken wrapper/stdio regression; no arbitrary timing gates.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for name in [
            "goose",
            "codex-acp",
            "claude-agent-acp",
            "hermes-acp",
            "buzz-pi-acp",
        ] {
            let worker = std::env::current_dir().unwrap().join(name);
            let args = CliArgs::try_parse_from([
                "buzz-acp",
                "--private-key",
                "0000000000000000000000000000000000000000000000000000000000000001",
                "--agent-command",
                worker.to_str().unwrap(),
                "--agent-args",
                "",
            ])
            .unwrap();
            let config = Config::from_args(args).unwrap();
            // A second spawn uses the same host configuration after shutdown,
            // just as a replacement worker does. No prefix is consumed once.
            for _ in 0..2 {
                let spawned = AcpClient::spawn(
                    &config.agent_command,
                    &config.agent_args,
                    &config.persona_env_vars,
                    config.has_generated_codex_config,
                )
                .await;
                if mode == "failure" {
                    let error = spawned.err().expect("invalid wrapper launched a worker");
                    if std::env::var(PREFIX_ENV)
                        .unwrap()
                        .contains("/missing-buzz-launcher")
                    {
                        assert!(error.to_string().contains("/missing-buzz-launcher"));
                    }
                    return;
                }
                let mut client = spawned.unwrap();
                let probe = client.initialize().await.unwrap();
                let expected = match name {
                    "goose" => json!(["acp"]),
                    "hermes-acp" => json!([]),
                    "buzz-pi-acp" => json!([
                        "--",
                        "--skill",
                        std::env::current_dir().unwrap().join(".agents/skills")
                    ]),
                    _ => json!([]),
                };
                assert_eq!(probe["argv"], expected, "{name}");
                assert!(probe["prefix"].is_null());
                if name == "hermes-acp" {
                    assert_eq!(probe["hermes"], "1");
                }
                let adapter = match name {
                    "codex-acp" => {
                        let config: Value =
                            serde_json::from_str(probe["codex"].as_str().unwrap()).unwrap();
                        assert_eq!(config["sandbox_workspace_write"]["network_access"], true);
                        Some(StandardAdapterKind::Codex)
                    }
                    "claude-agent-acp" => Some(StandardAdapterKind::Claude),
                    _ => None,
                };
                assert_eq!(client.standard_adapter, adapter);
                if name == "claude-agent-acp" {
                    let meta = client
                        .session_new_full("/tmp", vec![], None, None)
                        .await
                        .unwrap()
                        .raw["meta"]
                        .clone();
                    let expected = match std::env::var_os(PREFIX_ENV) {
                        Some(_) => Value::Null,
                        None => json!({"claudeCode": {"options": {"extraArgs": {
                            "thinking-display": "summarized"
                        }}}}),
                    };
                    assert_eq!(meta, expected, "prefix leaves summaries off");
                }
                client.shutdown().await;
            }
        }
    })
    .await
    .expect("wrapped ACP handshake exceeded test deadline");
}
