//! App Attest rejections must tell operators which check failed. This is the
//! only test in its binary, so no other thread races tracing's global
//! callsite-interest cache.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_push_gateway::app_attest::AppAttestVerifier;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn attestation_rejections_log_operation_reason_and_failed_check() {
    let root = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/apple-app-attestation-root.pem"
    ))
    .unwrap();
    let verifier = AppAttestVerifier::new("TEAM.test.app".into(), root).unwrap();
    let key_id = STANDARD.encode([7; 32]);
    let cases = [
        ("not base64!", "encoding", "invalid base64 encoding"),
        (
            &*STANDARD.encode(b"not cbor"),
            "structure",
            "invalid CBOR structure",
        ),
    ];
    for (attestation, reason, detail) in cases {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish();
        let result = tracing::subscriber::with_default(subscriber, || {
            verifier.verify_attestation(attestation, &key_id, b"client-data")
        });
        assert!(result.is_err());
        let bytes = capture.0.lock().unwrap().clone();
        let line: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(line["level"], "WARN", "{line}");
        assert_eq!(
            line["fields"]["message"],
            "App Attest verification rejected"
        );
        assert_eq!(line["fields"]["operation"], "attestation");
        assert_eq!(line["fields"]["reason"], reason, "{line}");
        assert_eq!(line["fields"]["detail"], detail, "{line}");
    }
}
