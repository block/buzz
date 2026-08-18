use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

#[test]
fn stdio_keeps_protocol_output_to_one_json_line() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-ifc-broker"))
        .args(["--mode", "enforce"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn broker");
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "broker/info",
        "params": {}
    });
    child
        .stdin
        .take()
        .expect("broker stdin")
        .write_all(format!("{request}\n").as_bytes())
        .expect("write request");

    let output = child.wait_with_output().expect("broker output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");
    assert_eq!(stdout.lines().count(), 1);
    let response: Value = serde_json::from_str(stdout.trim()).expect("JSON-RPC response");
    assert_eq!(response["result"]["mode"], "enforce");
    assert_eq!(response["result"]["worker_count"], 0);
}
