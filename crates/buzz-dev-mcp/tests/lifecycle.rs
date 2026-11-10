//! Exercise both MCP lifecycles against the shipped stdio server, without an LLM.
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

struct Server {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    work: tempfile::TempDir,
    meta: Option<Value>,
}

impl Server {
    fn start(meta: Option<Value>) -> Self {
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("fixture.txt"), "sunflower\n").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-dev-mcp"))
            .current_dir(work.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            work,
            meta,
        }
    }

    async fn request(&mut self, id: u32, method: &str, mut params: Value) -> Value {
        if let Some(meta) = &self.meta {
            params["_meta"] = meta.clone();
        }
        let frame = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
        self.input
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .unwrap();
        let line = timeout(Duration::from_secs(10), self.output.next_line())
            .await
            .expect("server response deadline")
            .unwrap()
            .expect("server closed stdout");
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    async fn check_tools_and_shutdown(mut self) {
        let listed = self.request(2, "tools/list", json!({})).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        for name in ["shell", "read_file", "str_replace"] {
            assert!(tools.iter().any(|tool| tool["name"] == name), "{listed}");
        }
        let read = self
            .request(
                3,
                "tools/call",
                json!({
                    "name":"read_file", "arguments":{"path":"fixture.txt"}
                }),
            )
            .await;
        assert!(read["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("sunflower"));
        let edit = self.request(4, "tools/call", json!({
            "name":"str_replace", "arguments":{"path":"fixture.txt", "old_str":"sunflower", "new_str":"orchid"}
        })).await;
        assert!(edit.get("error").is_none(), "{edit}");
        assert_eq!(
            std::fs::read_to_string(self.work.path().join("fixture.txt")).unwrap(),
            "orchid\n"
        );
        let bad = self
            .request(
                5,
                "tools/call",
                json!({
                    "name":"read_file", "arguments":{}
                }),
            )
            .await;
        assert_eq!(bad["result"]["isError"], true, "{bad}");
        // A rejected tool call must leave the connection usable.
        let listed = self.request(6, "tools/list", json!({})).await;
        assert!(listed["result"]["tools"].is_array(), "{listed}");
        drop(self.input);
        let status = timeout(Duration::from_secs(10), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{status}");
    }
}

#[tokio::test]
async fn legacy_initialize_preserves_tools() {
    for version in ["2024-11-05", "2025-11-25"] {
        let mut server = Server::start(None);
        let reply = server
            .request(
                1,
                "initialize",
                json!({
                    "protocolVersion":version, "capabilities":{},
                    "clientInfo":{"name":"legacy-client", "version":"1"}
                }),
            )
            .await;
        assert_eq!(reply["result"]["protocolVersion"], version, "{reply}");
        server
            .input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await
            .unwrap();
        server.check_tools_and_shutdown().await;
    }
}

#[tokio::test]
async fn discover_preserves_tools() {
    let mut server = Server::start(Some(json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"discovery-client", "version":"1"},
        "io.modelcontextprotocol/clientCapabilities":{}
    })));
    let reply = server.request(1, "server/discover", json!({})).await;
    assert!(
        reply["result"]["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28")),
        "{reply}"
    );
    assert_eq!(
        reply["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "buzz-dev-mcp"
    );
    server.check_tools_and_shutdown().await;
}
