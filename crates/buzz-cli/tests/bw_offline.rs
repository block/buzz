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
    assert_eq!(draft["activation_gate"], "activation-readback");
    assert_eq!(draft["tags"], e["tags"]);
    // A bare publish carries no activation evidence and stays refused.
    let out = run(&["bw", "publish"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let _: Value = serde_json::from_slice(&out.stderr).expect("error JSON");
}

/// The whole producer boundary through the real binary, with a cleared
/// environment: no relay URL, no private key and no network are ever consulted.
#[test]
fn cli_publish_requires_activation_and_an_exact_readback_offline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let d: Value = serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
        .expect("corpus");
    let case = d["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["name"] == "issue-update-positive")
        .expect("case");
    let operation = &d["events"]["update_a"]["event"];
    let write = |name: &str, v: &Value| {
        let p = dir.path().join(name);
        std::fs::write(&p, v.to_string()).expect("write");
        p.to_str().expect("path").to_owned()
    };
    let bundle = |name: &str, labels: &[&str]| {
        write(
            name,
            &json!({
                "now": case["now"], "trust": case["trust"], "external": case["external"],
                "events": labels.iter().map(|l| d["events"][*l]["event"].clone()).collect::<Vec<_>>(),
                "operation": {"pubkey":operation["pubkey"],"created_at":operation["created_at"],
                    "kind":operation["kind"],"tags":operation["tags"],"content":operation["content"]},
            }),
        )
    };
    let activated = bundle("activated.json", &["repo", "policy", "root_a", "enroll_a"]);
    let unactivated = bundle("unactivated.json", &["policy"]);
    let signed = write("signed.json", operation);
    let foreign = write("foreign.json", &d["events"]["enroll_a"]["event"]);

    // Without an owner-signed genesis there is nothing to activate.
    let out = run(&["bw", "publish", "--input", &unactivated]);
    assert_eq!(out.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&out.stderr).expect("error JSON");
    assert_eq!(error["message"], "bw:pending:references:missing-reference");

    // Preparation pins the ID the signature will carry and publishes nothing.
    let out = run(&["bw", "publish", "--input", &activated]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let prepared: Value = serde_json::from_slice(&out.stdout).expect("prepared JSON");
    assert_eq!(prepared["stage"], "prepared");
    assert_eq!(prepared["published"], false);
    assert_eq!(prepared["event_id"], operation["id"]);
    assert_eq!(
        prepared["activation"]["genesis"],
        d["events"]["repo"]["event"]["id"]
    );

    // A signed event alone, and a readback of some other event, are not proof.
    for args in [
        vec!["bw", "publish", "--input", &activated, "--signed", &signed],
        vec![
            "bw",
            "publish",
            "--input",
            &activated,
            "--signed",
            &signed,
            "--readback",
            &foreign,
        ],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let error: Value = serde_json::from_slice(&out.stderr).expect("error JSON");
        let message = error["message"].as_str().expect("message");
        assert!(message.starts_with("bw:"), "{message}");
    }

    // The exact readback of the pinned event is the only accepted proof, and the
    // reported ID is still the one pinned before signing.
    let out = run(&[
        "bw",
        "publish",
        "--input",
        &activated,
        "--signed",
        &signed,
        "--readback",
        &signed,
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let published: Value = serde_json::from_slice(&out.stdout).expect("published JSON");
    assert_eq!(published["stage"], "published");
    assert_eq!(published["published"], true);
    assert_eq!(published["event_id"], prepared["event_id"]);
}
