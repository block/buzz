//! Opt-in real runtime checks. Uses real ACP, MCP and Git processes, with a
//! deterministic local model for buzz-agent and the installed Goose provider.
use super::*;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn scripted_model(command: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        for round in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 8192];
            let headers_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buf[..n]);
                assert!(request.len() < 2_000_000);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..headers_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            assert!(length < 2_000_000);
            while request.len() < headers_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buf[..n]);
            }
            let request: serde_json::Value =
                serde_json::from_slice(&request[headers_end..]).unwrap();
            let (message, reason) = if round == 0 {
                let name = request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find_map(|tool| {
                        tool["function"]["name"]
                            .as_str()
                            .filter(|name| name.ends_with("__shell"))
                    })
                    .unwrap();
                (
                    serde_json::json!({"role":"assistant", "content":null,"tool_calls":[{"id":"git-probe","type":"function","function":{"name":name,"arguments":serde_json::json!({"command":command}).to_string()}}]}),
                    "tool_calls",
                )
            } else {
                (
                    serde_json::json!({"role":"assistant","content":"done"}),
                    "stop",
                )
            };
            let body = serde_json::json!({"id":"probe","object":"chat.completion","model":"probe","choices":[{"index":0,"message":message,"finish_reason":reason}]}).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (url, task)
}

async fn check_runtime(goose: bool) {
    let binaries = PathBuf::from(
        std::env::var("BUZZ_TEST_BIN_DIR").expect("set BUZZ_TEST_BIN_DIR to built binaries"),
    );
    let temp = tempfile::tempdir().unwrap();
    let mut config = build_mcp_servers_tests::test_config();
    config.relay_url = "https://relay.invalid".into();
    config.mcp_command = if goose {
        String::new()
    } else {
        binaries.join("buzz-dev-mcp").to_str().unwrap().into()
    };
    let git =
        git::GitEnvironment::install(&config.keys, &config.relay_url, &binaries.join("buzz-acp"))
            .unwrap();
    config.persona_env_vars.extend(git.env.iter().cloned());
    let keyfile = git
        .env
        .windows(2)
        .find(|pair| pair[0].1 == "nostr.keyfile")
        .unwrap()[1]
        .1
        .clone();
    let script = temp.path().join("probe.sh");
    std::fs::write(
        &script,
        r#"set -eu
cd "$(dirname "$0")"
test -z "${NOSTR_PRIVATE_KEY:-}"
git config user.name > identity
git config user.email >> identity
test "$(git config --get-urlmatch credential.helper https://relay.invalid/git/test/repo)" = nostr
! git config --get-urlmatch credential.helper https://unrelated.invalid/git/test/repo
git init -q repo
cd repo
git -c core.hooksPath=/dev/null commit -s --allow-empty -qm 'runtime Git probe'
git tag -m 'runtime tag probe' probe
git verify-commit HEAD
git verify-tag probe
git config nostr.keyfile > ../keyfile
printf 'passed' > ../result
"#,
    )
    .unwrap();
    let command = format!("sh '{}'", script.display());
    let (url, model) = scripted_model(command.clone()).await;
    let (runtime, args) = if goose {
        config
            .persona_env_vars
            .push(("GOOSE_MODE".into(), "auto".into()));
        (
            "goose".into(),
            vec!["acp".into(), "--with-builtin".into(), "developer".into()],
        )
    } else {
        config.persona_env_vars.extend([
            ("BUZZ_AGENT_PROVIDER".into(), "openai".into()),
            ("OPENAI_COMPAT_BASE_URL".into(), url),
            ("OPENAI_COMPAT_API_KEY".into(), "test".into()),
            ("OPENAI_COMPAT_MODEL".into(), "probe".into()),
        ]);
        (
            binaries.join("buzz-agent").to_string_lossy().into_owned(),
            vec![],
        )
    };
    // Isolate the probes from operator/repository Git configuration.
    config.persona_env_vars.extend([
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ]);
    let mut client = AcpClient::spawn(&runtime, &args, &config.persona_env_vars, false)
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        client.initialize().await.unwrap();
        let session = client.session_new_full(temp.path().to_str().unwrap(), build_mcp_servers(&config), None, None).await.unwrap();
        client.session_prompt_with_idle_timeout(&session.session_id, &format!("Run exactly this command using your shell tool: {command}. It is a local disposable Git test. Do not edit it, read credentials, or contact any remote. Report the exit code."), Duration::from_secs(60), Duration::from_secs(100)).await.unwrap();
    }).await;
    client.shutdown().await;
    model.abort();
    result.unwrap();
    assert_eq!(
        std::fs::read_to_string(temp.path().join("result")).unwrap(),
        "passed"
    );
    let identity = std::fs::read_to_string(temp.path().join("identity")).unwrap();
    assert!(identity.contains(&format!(
        "{}@relay.invalid",
        config.keys.public_key().to_hex()
    )));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("keyfile"))
            .unwrap()
            .trim(),
        keyfile
    );
    assert!(std::path::Path::new(&keyfile).exists());
    drop(git);
    assert!(!std::path::Path::new(&keyfile).exists());
}

#[tokio::test]
#[ignore = "requires built buzz-acp, buzz-agent and buzz-dev-mcp; BUZZ_TEST_BIN_DIR"]
async fn real_buzz_agent_git_shell() {
    check_runtime(false).await;
}

#[tokio::test]
#[ignore = "requires installed/configured Goose and built buzz-acp; BUZZ_TEST_BIN_DIR"]
async fn real_goose_native_git_shell() {
    check_runtime(true).await;
}
