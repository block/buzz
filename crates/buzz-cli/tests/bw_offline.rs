use serde_json::{json, Value};
use std::process::{Command, Output};
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_buzz"))
        .env_clear()
        .args(args)
        .output()
        .expect("offline CLI")
}
#[test]
fn cli_json_dry_run_and_publish_gate_without_any_credentials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("input.json");
    let path = path.to_str().expect("path");
    let d: Value = serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
        .expect("corpus");
    let case = &d["cases"][1];
    let events: Vec<_> = case["input"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(|l| d["events"][l.as_str().expect("label")]["event"].clone())
        .collect();
    std::fs::write(path,json!({"now":case["now"],"trust":case["trust"],"external":case["external"],"events":events}).to_string()).expect("input");
    let out = run(&["bw", "validate", "--input", path]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: Value = serde_json::from_slice(&out.stdout).expect("JSON stdout");
    assert_eq!(rows[0]["event_id"], d["events"]["repo"]["event"]["id"]);
    assert_eq!(rows[1]["outcome"], "accept");
    let out = run(&["bw", "show", "--input", path]);
    assert!(out.status.success());
    let _: Value = serde_json::from_slice(&out.stdout).expect("projection JSON");
    let e = &d["events"]["policy"]["event"];
    std::fs::write(
        path,
        json!({"kind":e["kind"],"tags":e["tags"],"content":e["content"]}).to_string(),
    )
    .expect("draft");
    let out = run(&["bw", "dry-run", "--input", path]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let draft: Value = serde_json::from_slice(&out.stdout).expect("draft JSON");
    assert_eq!(draft["publish_enabled"], false);
    assert_eq!(draft["activation_gate"], "P3C-readback");
    assert_eq!(draft["tags"], e["tags"]);
    let out = run(&["bw", "publish"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let error: Value = serde_json::from_slice(&out.stderr).expect("error JSON");
    assert!(error["message"].as_str().expect("message").contains("P3C"));
}
