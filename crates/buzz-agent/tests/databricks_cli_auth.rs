//! CLI compatibility: the real auth subcommand uses the legacy cache layout.
//! No ACP, browser, external network, or user cache. Every child has a fresh root.
#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::time::Duration;

#[tokio::test]
async fn cli_signin_aliases_reuse_legacy_cache_without_runtime_configuration() {
    for provider in ["databricks", "databricks_v2", "databricks-v2"] {
        let root = tempfile::tempdir().unwrap();
        let host = "https://workspace.invalid";
        let key = format!("{host}/oidc/.well-known/oauth-authorization-server|databricks-cli|all-apis,offline_access");
        let dir = root.path().join("buzz-agent/oauth/databricks");
        std::fs::create_dir_all(&dir).unwrap();
        let cache = dir.join(format!("{}.json", hex::encode(Sha256::digest(key))));
        let content = serde_json::json!({"access_token":"synthetic-cli-cache", "refresh_token":null, "expires_at":4_000_000_000u64}).to_string();
        std::fs::write(&cache, &content).unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_buzz-agent"))
                .args(["auth", provider])
                .env_clear()
                .env("BUZZ_AGENT_CONFIG_DIR", root.path())
                .env("HOME", root.path())
                .env("DATABRICKS_HOST", host)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "auth must not emit ACP frames");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("Authenticated. Token cached"));
        assert!(!stderr.contains("Opening browser"));
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), content);
        assert!(!root
            .path()
            .join("buzz-agent/oauth/databricks-strict")
            .exists());
    }
}
