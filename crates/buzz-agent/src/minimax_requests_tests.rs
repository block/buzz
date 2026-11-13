//! Offline request-boundary regressions for MiniMax defaults and compatibility.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::Duration;

use crate::config::{Config, ThinkingEffort};
use crate::llm::Llm;
use crate::types::{HistoryItem, ToolCall, ToolDef, ToolResult, ToolResultContent};
use serde_json::{json, Value};

// Config::from_env runs in a separate process per case, never mutating the
// parallel test runner's environment or inheriting real provider credentials.
#[test]
fn minimax_environment_to_http() {
    let cases = [
        (
            "minimax-cn",
            Some("custom-deployment"),
            None,
            "https://custom.example/v1",
            "custom-deployment",
        ),
        (
            "minimax",
            None,
            None,
            "https://api.minimax.io/v1",
            "MiniMax-M3.1-Flash",
        ),
        (
            "minimax-cn",
            None,
            None,
            "https://api.minimaxi.com/v1",
            "MiniMax-M3.1-Flash",
        ),
        (
            "minimax",
            Some("MiniMax-M2.7"),
            None,
            "https://api.minimax.io/v1",
            "MiniMax-M2.7",
        ),
        (
            "minimax-cn",
            Some("MiniMax-M3"),
            Some("custom-deployment"),
            "https://api.minimaxi.com/v1",
            "custom-deployment",
        ),
        (
            "minimax",
            Some("MiniMax-M3"),
            None,
            "https://api.minimax.io/v1",
            "MiniMax-M3",
        ),
        (
            "anthropic",
            Some("MiniMax-M3.1-Flash"),
            None,
            "https://api.minimax.io/anthropic",
            "MiniMax-M3.1-Flash",
        ),
        (
            "anthropic",
            Some("MiniMax-M3.1-Flash"),
            None,
            "https://api.minimaxi.com/anthropic",
            "MiniMax-M3.1-Flash",
        ),
        (
            "openrouter",
            Some("minimax/minimax-m3.1-flash:free"),
            None,
            "https://openrouter.ai/api/v1",
            "minimax/minimax-m3.1-flash:free",
        ),
    ];
    for (provider, model, override_model, base, expected) in cases {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.env_clear()
            .args([
                "--exact",
                "llm::minimax_requests_tests::minimax_request_child",
                "--nocapture",
            ])
            .env("BUZZ_MINIMAX_TEST_CHILD", "1")
            .env("BUZZ_AGENT_PROVIDER", provider)
            .env("MINIMAX_API_KEY", "offline-test")
            .env("ANTHROPIC_API_KEY", "offline-test")
            .env("OPENROUTER_API_KEY", "offline-test")
            .env("EXPECTED_BASE", base)
            .env("EXPECTED_MODEL", expected)
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "3");
        if let Some(model) = model {
            cmd.env("MINIMAX_MODEL", model)
                .env("ANTHROPIC_MODEL", model)
                .env("OPENROUTER_MODEL", model);
        }
        if provider == "anthropic" {
            cmd.env("ANTHROPIC_BASE_URL", base);
        }
        if base == "https://custom.example/v1" {
            cmd.env("MINIMAX_BASE_URL", base);
        }
        if let Some(model) = override_model {
            cmd.env("BUZZ_AGENT_MODEL", model);
        }
        let output = cmd.output().unwrap();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child test did not pass: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            output.status.success(),
            "{provider}/{expected}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn capture(response: Value) -> (String, std::thread::JoinHandle<(String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        let end = loop {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
        let length: usize = head
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        while bytes.len() < end + length {
            let n = socket.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
        }
        let body = serde_json::from_slice(&bytes[end..end + length]).unwrap();
        let response = response.to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        (head, body)
    });
    (base, task)
}

#[tokio::test]
async fn minimax_request_child() {
    if std::env::var_os("BUZZ_MINIMAX_TEST_CHILD").is_none() {
        return;
    }
    let mut cfg = Config::from_env().unwrap();
    let model = std::env::var("EXPECTED_MODEL").unwrap();
    assert_eq!(cfg.model, model);
    assert_eq!(cfg.base_url, std::env::var("EXPECTED_BASE").unwrap());
    let anthropic = std::env::var("BUZZ_AGENT_PROVIDER").unwrap() == "anthropic";
    let gateway = std::env::var("BUZZ_AGENT_PROVIDER").unwrap() == "openrouter";
    if !anthropic && !gateway && model == "MiniMax-M3.1-Flash" {
        assert_eq!(cfg.max_context_tokens, 1_000_000);
    }
    let flash = model.contains("3.1-flash") || model.contains("3.1-Flash");
    if flash {
        // Session model switches pass the effective model independently of Config.
        cfg.model = "saved-prior-model".into();
    }
    let efforts = if flash {
        vec![
            None,
            Some(ThinkingEffort::Low),
            Some(ThinkingEffort::Medium),
            Some(ThinkingEffort::High),
            Some(ThinkingEffort::XHigh),
            Some(ThinkingEffort::Max),
        ]
    } else {
        vec![None]
    };
    let history = [
        HistoryItem::User("inspect image".into()),
        HistoryItem::Assistant {
            text: "opening image".into(),
            tool_calls: vec![ToolCall {
                provider_id: "call-1".into(),
                name: "view_image".into(),
                arguments: json!({}),
                provider_extra: Default::default(),
            }],
            reasoning_details: None,
        },
        HistoryItem::ToolResult(ToolResult {
            provider_id: "call-1".into(),
            is_error: false,
            content: vec![ToolResultContent::Image {
                data: "aW1hZ2U=".into(),
                mime_type: "image/png".into(),
            }],
        }),
    ];
    let tools = [ToolDef {
        name: "view_image".into(),
        description: "View an image".into(),
        input_schema: json!({"type": "object", "properties": {}}),
    }];
    for effort in efforts {
        let response = if anthropic {
            json!({"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"})
        } else {
            json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]})
        };
        let (base, task) = capture(response);
        // Preserve the protocol prefix while redirecting only the host to a fixture.
        cfg.base_url = format!("{base}{}", if anthropic { "/anthropic" } else { "/v1" });
        cfg.thinking_effort = effort;
        let llm = Llm::new(&cfg).unwrap();
        let result = llm
            .complete(&cfg, "system", &history, &tools, &model)
            .await
            .unwrap();
        assert_eq!(result.text, "ok");
        let (head, body) = task.join().unwrap();
        assert!(
            head.starts_with(if anthropic {
                "POST /anthropic/v1/messages "
            } else {
                "POST /v1/chat/completions "
            }),
            "{head}"
        );
        assert!(head.to_lowercase().contains(if anthropic {
            "x-api-key: offline-test"
        } else {
            "authorization: bearer offline-test"
        }));
        assert_eq!(body["model"], model);
        let tool = if anthropic {
            &body["tools"][0]
        } else {
            &body["tools"][0]["function"]
        };
        assert_eq!(tool["name"], "view_image");
        let field = if anthropic {
            &body["output_config"]["effort"]
        } else if gateway {
            &body["reasoning"]["effort"]
        } else {
            &body["reasoning_effort"]
        };
        match effort {
            Some(e) => assert_eq!(field, e.openai_effort_str()),
            None => assert!(field.is_null()),
        }
        assert_ne!(body["thinking"]["type"], "disabled");
        assert!(body.get("service_tier").is_none());
        assert!(body.get("reasoning_split").is_none());
        assert_eq!(
            body["messages"][if anthropic { 1 } else { 2 }]["role"],
            "assistant"
        );
        let serialized = body.to_string();
        assert!(serialized.contains("aW1hZ2U="));
        assert!(serialized.contains("image/png"));
        assert!(serialized.contains("call-1"));
    }
    if flash {
        // Invalid efforts fail before attempting the (unreachable) transport.
        cfg.base_url = "http://127.0.0.1:1".into();
        for effort in [ThinkingEffort::None, ThinkingEffort::Minimal] {
            cfg.thinking_effort = Some(effort);
            let error = Llm::new(&cfg)
                .unwrap()
                .complete(&cfg, "", &[], &[], &model)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("reasoning effort must be"));
        }
    }
}
