//! Golden wire fixtures (spec §Provider Protocol).
//!
//! These drive the **built binary** over a real pipe rather than calling an
//! in-process function: the contract the desktop depends on is
//! `stdin → one JSON object on stdout → exit code`, and an in-process test
//! would assert the shape of a value while skipping the three things that
//! actually break — the process writing nothing, writing two objects, or
//! signalling the outcome through the exit code.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/provider-wire")
}

/// Feed one request to the binary; return `(stdout, exit code)`.
fn run(request: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-backend-kubernetes"))
        // A kubeconfig that does not exist, so a fixture that accidentally
        // reaches the cluster fails loudly here instead of depending on
        // whatever cluster the developer is pointed at.
        .env("KUBECONFIG", "/nonexistent/kubeconfig-for-fixture-tests")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("could not run the provider binary");
    child
        .stdin
        .take()
        .expect("no stdin")
        .write_all(request.as_bytes())
        .expect("could not write the request");
    let out = child.wait_with_output().expect("provider did not exit");
    (
        String::from_utf8(out.stdout).expect("stdout was not UTF-8"),
        out.status.code().unwrap_or(-1),
    )
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name))
        .unwrap_or_else(|e| panic!("could not read fixture {name}: {e}"))
}

/// Every response fixture, byte-compared after key-sorted re-serialization so
/// a field rename fails here rather than in a desktop reading `undefined`.
#[test]
fn responses_match_their_fixtures() {
    let cases = [
        "deploy-relay-mesh",
        "deploy-relay-mesh-padded",
        "deploy-tag-image",
        "deploy-no-owner",
    ];
    // The list must cover every response fixture on disk. A literal array is
    // never empty, so `!is_empty()` would assert nothing; what can actually go
    // wrong is a fixture added to the directory and never added here, which
    // reads as a passing suite that exercises one case fewer than it appears to.
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures())
        .expect("could not read the fixture directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some(name.strip_suffix(".response.json")?.to_string()))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = cases.iter().map(|c| c.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "response fixtures and cases disagree");

    for case in cases {
        let (stdout, code) = run(&read(&format!("{case}.request.json")));
        assert_eq!(code, 0, "{case}: a produced response must exit 0");

        // Exactly one object, terminated by exactly one newline. Two responses
        // would leave the desktop's reader holding a second one forever.
        assert_eq!(
            stdout.matches('\n').count(),
            1,
            "{case}: expected exactly one line, got {stdout:?}"
        );

        let actual: serde_json::Value =
            serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{case}: {e}: {stdout:?}"));
        let expected: serde_json::Value =
            serde_json::from_str(&read(&format!("{case}.response.json"))).unwrap();
        assert_eq!(
            actual, expected,
            "{case}: response drifted from its fixture"
        );
    }
}

/// `info` is checked on the fields the desktop reads rather than byte-for-byte:
/// the namespace default is randomly generated per call (§K8s Namespace), so a
/// golden copy of it would be a test that fails every run.
#[test]
fn info_response_carries_the_contract_fields() {
    let (stdout, code) = run(&read("info.request.json"));
    assert_eq!(code, 0);
    let info: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(info["ok"], true);
    assert_eq!(info["protocol_version"], 1);
    assert_eq!(info["name"], "kubernetes");
    let schema = &info["config_schema"];
    assert_eq!(
        schema["required"],
        serde_json::json!(["namespace", "image"])
    );
    let default = schema["properties"]["namespace"]["default"]
        .as_str()
        .expect("no generated namespace default");
    assert!(
        default.starts_with("buzz-agents-"),
        "unexpected namespace default: {default}"
    );
    let image_default = schema["properties"]["image"]["default"]
        .as_str()
        .expect("no image default");
    assert!(
        image_default.starts_with("ghcr.io/block/buzz-sprig:")
            && image_default.contains("@sha256:"),
        "unexpected image default: {image_default}"
    );
}

/// The desktop's richest payload must parse. No response fixture: this one
/// reaches the cluster, so its outcome depends on a kubeconfig. What it
/// guards is that every field the desktop sends is *accepted* — a payload the
/// provider rejects at parse time is a deploy that never starts.
#[test]
fn the_full_desktop_payload_is_accepted() {
    let (stdout, code) = run(&read("deploy-full-launch.request.json"));
    assert_eq!(code, 0);
    let response: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let error = response["error"].as_str().unwrap_or_default();
    // It fails — there is no cluster — but it must fail at the *connection*,
    // having accepted every field above it.
    assert!(
        error.contains("kubeconfig"),
        "the full payload was rejected before reaching the cluster: {error}"
    );
}

/// Sami's pre-registered respond-to matrix, driven through the built binary.
///
/// `build_env` runs at `main.rs:124`, `client::connect` at `:132`, so under a
/// kubeconfig that cannot exist the error string *is* the ordering assertion:
/// "kubeconfig" means the gate passed and we reached the cluster, anything
/// else means we refused before writing a Secret. A test asserting only
/// `ok: false` would pass on the connection error and prove nothing.
///
/// The cases are applied to the real full-launch request so each one differs
/// from a known-good deploy in exactly the field under test.
#[test]
fn the_respond_to_gate_matches_the_harness_acceptance_surface() {
    let key_a = "a".repeat(64);
    let padded_upper = format!("  {}  ", "A".repeat(64));
    // (name, respond_to, allowlist, must reach the cluster)
    let cases: Vec<(&str, &str, Option<Vec<String>>, bool)> = vec![
        ("allowlist + []", "allowlist", Some(vec![]), false),
        ("allowlist + absent", "allowlist", None, false),
        (
            "allowlist + junk",
            "allowlist",
            Some(vec!["beefcafe".into()]),
            false,
        ),
        ("unparseable mode", "npub1abc", None, false),
        ("padded mode", " allowlist ", None, false),
        (
            "allowlist + two valid",
            "allowlist",
            Some(vec![key_a.clone(), "b".repeat(64)]),
            true,
        ),
        (
            "owner-only + junk list",
            "owner-only",
            Some(vec!["beefcafe".into()]),
            true,
        ),
        (
            "allowlist + padded upper",
            "allowlist",
            Some(vec![padded_upper]),
            true,
        ),
        ("nobody", "nobody", None, true),
        ("anyone", "anyone", None, true),
    ];

    let base: serde_json::Value =
        serde_json::from_str(&read("deploy-full-launch.request.json")).unwrap();

    for (name, mode, allowlist, reaches_cluster) in cases {
        let mut request = base.clone();
        let agent = &mut request["agent"];
        agent["respond_to"] = serde_json::json!(mode);
        agent["respond_to_allowlist"] = match &allowlist {
            Some(list) => serde_json::json!(list),
            None => serde_json::Value::Null,
        };

        let (stdout, code) = run(&request.to_string());
        assert_eq!(code, 0, "{name}: provider did not exit cleanly");
        let response: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        let error = response["error"].as_str().unwrap_or_default();
        let reached = error.contains("kubeconfig");

        assert_eq!(
            reached, reaches_cluster,
            "{name}: expected reaches_cluster={reaches_cluster}, got error: {error}"
        );
        if !reaches_cluster {
            assert!(
                error.contains("deploy refused"),
                "{name}: refused, but not by the gate: {error}"
            );
        }
    }
}
