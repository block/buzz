//! Exercise the actual executable's parsed configuration and verifier constructor.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn parsed_configuration_reaches_executable_verifier() {
    for setting in [None, Some("production"), Some("development")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-push-gateway"));
        command.env_clear().envs([
            (
                "BUZZ_PUSH_GRANT_KEYS",
                format!("grant:{}", STANDARD.encode([1; 32])),
            ),
            (
                "BUZZ_PUSH_TOKEN_KEYS",
                format!("token:{}", STANDARD.encode([2; 32])),
            ),
            ("BUZZ_PUSH_GATEWAY_ORIGIN", "https://push.example".into()),
            ("BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS", "2592000".into()),
            ("DATABASE_URL", "unused".into()),
            (
                "BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID",
                "TEAM.test.app".into(),
            ),
            (
                "BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH",
                format!(
                    "{}/tests/fixtures/apple-app-attestation-root.pem",
                    env!("CARGO_MANIFEST_DIR")
                ),
            ),
            // Startup stops immediately after configuring the real verifier.
            ("BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH", "".into()),
            ("BUZZ_PUSH_DOGFOOD_APNS_TOPIC", "test.app".into()),
            ("RUST_LOG", "info".into()),
        ]);
        // A directory cannot be read as an APNs certificate, on every platform.
        command.env(
            "BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH",
            env!("CARGO_MANIFEST_DIR"),
        );
        if let Some(setting) = setting {
            command.env("BUZZ_PUSH_APP_ATTEST_ENVIRONMENT", setting);
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("gateway startup did not terminate");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        if setting == Some("development") && !cfg!(feature = "personal-dev-app-attest") {
            assert!(
                stderr.contains("BUZZ_PUSH_APP_ATTEST_ENVIRONMENT"),
                "{stderr}"
            );
            assert!(!stdout.contains("App Attest verifier configured"));
        } else {
            let event = stdout
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find(|event| event["fields"]["message"] == "App Attest verifier configured")
                .unwrap_or_else(|| panic!("verifier was not constructed: {stdout} {stderr}"));
            let expected = if setting == Some("development") {
                "Development"
            } else {
                "Production"
            };
            assert_eq!(event["fields"]["environment"], expected);
        }
    }
}
