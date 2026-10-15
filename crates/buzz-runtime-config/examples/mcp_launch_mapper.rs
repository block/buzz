//! Map a native Hermes MCP config into the versioned stdio MCP launch
//! document (buzz-core `mcp_config` v1).
//!
//! Usage:
//!   cargo run -p buzz-runtime-config --example mcp_launch_mapper [PATH]
//!
//! `PATH` defaults to `~/.hermes/config.yaml`. The output is the versioned
//! launch document (JSON) with environment values redacted so it is safe to
//! paste into a PR comment or log. Pass `--no-redact` to emit full env values
//! (only if you are launching an agent locally and trust the output channel).
use std::path::PathBuf;

use buzz_runtime_config::hermes;
use buzz_runtime_config::launch::{to_launch_json, McpLaunchConfigDocument};
use buzz_runtime_config::McpServerConfig;

fn redacted_json(doc: &McpLaunchConfigDocument) -> serde_json::Value {
    let mut value = serde_json::to_value(doc).expect("doc serializes");
    for server in value["servers"].as_array_mut().expect("servers array") {
        // Arguments may carry secrets too (e.g. an API key passed as a CLI
        // flag), so redact both args and environment values before emitting
        // a shareable sample.
        if let Some(args) = server["args"].as_array_mut() {
            for arg in args.iter_mut() {
                *arg = serde_json::Value::String("***".to_string());
            }
        }
        if let Some(env) = server["env"].as_object_mut() {
            for val in env.values_mut() {
                *val = serde_json::Value::String("***".to_string());
            }
        }
    }
    value
}

fn main() {
    let mut redact = true;
    let mut path: PathBuf = std::env::home_dir()
        .expect("home dir")
        .join(".hermes")
        .join("config.yaml");
    for arg in std::env::args().skip(1) {
        if arg == "--no-redact" {
            redact = false;
        } else {
            path = PathBuf::from(arg);
        }
    }

    let native = hermes::read_mcp_config(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read native Hermes config {}: {err}",
            path.display()
        )
    });
    let doc = McpLaunchConfigDocument::from_runtime_config(&native).unwrap_or_else(|err| {
        panic!("mapped document is invalid (no launch document can be built): {err}")
    });

    let total = native.servers.len();
    let enabled = native
        .servers
        .iter()
        .filter(|s: &&McpServerConfig| s.is_enabled())
        .count();
    let json = if redact {
        redacted_json(&doc)
    } else {
        // Also verify the raw launch bytes encode cleanly (validates env fully).
        let bytes = to_launch_json(&native).expect("launch document encodes");
        serde_json::from_slice(&bytes).expect("parses")
    };

    println!("// source: {}", path.display());
    println!(
        "// runtime shortlist: {enabled}/{total} native servers enabled -> {} launch entries",
        doc.servers().len()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&json).expect("pretty json")
    );
}
