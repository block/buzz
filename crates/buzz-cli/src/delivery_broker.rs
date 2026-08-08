use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use buzz_core::delivery_broker::{
    broker_response_digest, BrokerErrorCode, BrokerOperation, BrokerRequest, BrokerResponse,
    BrokerResponseEnvelope, BROKER_CAPABILITY_ENV, BROKER_DIR_ENV, BROKER_PROTOCOL_VERSION,
    BROKER_RESPONSE_ATTESTATION_KIND, BROKER_RESPONSE_PUBKEY_ENV, MAX_BROKER_REQUEST_BYTES,
    MAX_BROKER_RESPONSE_BYTES,
};
use nostr::PublicKey;
use serde_json::Value;
use uuid::Uuid;

use crate::error::CliError;

// The broker admits requests up to 30 seconds old and bounds claimed work at
// 110 seconds. Leave margin for atomic response publication and polling.
const BROKER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(150);
const BROKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_IN_FLIGHT_REQUESTS: usize = 8;

#[derive(Clone)]
pub(crate) struct DeliveryBrokerClient {
    root: PathBuf,
    capability: String,
    response_pubkey: PublicKey,
}

impl DeliveryBrokerClient {
    pub(crate) fn from_env() -> Result<Option<Self>, CliError> {
        let root = std::env::var_os(BROKER_DIR_ENV);
        let capability = std::env::var(BROKER_CAPABILITY_ENV).ok();
        let response_pubkey = std::env::var(BROKER_RESPONSE_PUBKEY_ENV).ok();
        match (root, capability, response_pubkey) {
            (None, None, None) => Ok(None),
            (Some(root), Some(capability), Some(response_pubkey)) if !capability.is_empty() => {
                let response_pubkey = PublicKey::parse(&response_pubkey).map_err(|_| {
                    CliError::Other("delivery broker response pubkey is invalid".into())
                })?;
                Self::new(PathBuf::from(root), capability, response_pubkey).map(Some)
            }
            _ => Err(CliError::Other(format!(
                "delivery broker is partially configured; {BROKER_DIR_ENV}, \
                 {BROKER_CAPABILITY_ENV}, and {BROKER_RESPONSE_PUBKEY_ENV} are required"
            ))),
        }
    }

    pub(crate) fn new(
        root: PathBuf,
        capability: String,
        response_pubkey: PublicKey,
    ) -> Result<Self, CliError> {
        if !root.is_absolute() {
            return Err(CliError::Other(
                "delivery broker directory must be an absolute path".into(),
            ));
        }
        for child in ["requests", "processing", "responses"] {
            let path = root.join(child);
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| {
                CliError::Other(format!(
                    "delivery broker {child} directory is unavailable: {e}"
                ))
            })?;
            if metadata.file_type().is_symlink()
                || metadata_is_reparse_point(&metadata)
                || !metadata.is_dir()
            {
                return Err(CliError::Other(format!(
                    "delivery broker {child} path is not a real directory"
                )));
            }
        }
        Ok(Self {
            root,
            capability,
            response_pubkey,
        })
    }

    pub(crate) async fn query(&self, filters: &[Value]) -> Result<String, CliError> {
        self.call(BrokerOperation::Query {
            filters: filters.to_vec(),
        })
        .await
        .map(|value| value.to_string())
    }

    pub(crate) async fn count(&self, filters: &[Value]) -> Result<String, CliError> {
        self.call(BrokerOperation::Count {
            filters: filters.to_vec(),
        })
        .await
        .map(|value| value.to_string())
    }

    pub(crate) async fn submit_message(&self, event: &nostr::Event) -> Result<String, CliError> {
        self.call(BrokerOperation::SubmitStoredMessage {
            event: Box::new(event.clone()),
        })
        .await
        .map(|value| value.to_string())
    }

    async fn call(&self, operation: BrokerOperation) -> Result<Value, CliError> {
        self.call_with_timeout(operation, BROKER_RESPONSE_TIMEOUT)
            .await
    }

    async fn call_with_timeout(
        &self,
        operation: BrokerOperation,
        timeout: Duration,
    ) -> Result<Value, CliError> {
        let request_id = Uuid::new_v4();
        let request = BrokerRequest {
            version: BROKER_PROTOCOL_VERSION,
            request_id,
            capability: self.capability.clone(),
            created_at_ms: unix_now_ms(),
            operation,
        };
        let bytes = serde_json::to_vec(&request)
            .map_err(|e| CliError::Other(format!("delivery broker request encode failed: {e}")))?;
        if bytes.len() as u64 > MAX_BROKER_REQUEST_BYTES {
            return Err(CliError::Usage(format!(
                "delivery broker request exceeds {} bytes",
                MAX_BROKER_REQUEST_BYTES
            )));
        }

        let request_path = self
            .root
            .join("requests")
            .join(format!("{request_id}.json"));
        let in_flight = ["requests", "processing"]
            .into_iter()
            .map(|child| count_request_files(&self.root.join(child)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum::<usize>();
        if in_flight >= MAX_IN_FLIGHT_REQUESTS {
            return Err(CliError::Relay {
                status: 503,
                body: format!(
                    "delivery broker is at its in-flight limit (max {MAX_IN_FLIGHT_REQUESTS})"
                ),
            });
        }
        write_atomic(&request_path, &bytes)
            .map_err(|e| CliError::Other(format!("delivery broker request publish failed: {e}")))?;

        let response_path = self
            .root
            .join("responses")
            .join(format!("{request_id}.json"));
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match read_bounded_regular_file(&response_path, MAX_BROKER_RESPONSE_BYTES) {
                Ok(Some(bytes)) => {
                    let _ = std::fs::remove_file(&response_path);
                    let envelope: BrokerResponseEnvelope =
                        serde_json::from_slice(&bytes).map_err(|e| {
                            CliError::Other(format!("delivery broker response decode failed: {e}"))
                        })?;
                    verify_envelope(&envelope, &self.response_pubkey)?;
                    return parse_response(request_id, envelope.response);
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(CliError::Other(format!(
                        "delivery broker response read failed: {e}"
                    )))
                }
            }

            if tokio::time::Instant::now() >= deadline {
                let _ = std::fs::remove_file(&request_path);
                let _ = std::fs::remove_file(&response_path);
                return Err(CliError::DeliveryUnknown(format!(
                    "delivery broker timed out waiting for request {request_id}; the operation may have completed"
                )));
            }
            tokio::time::sleep(BROKER_POLL_INTERVAL).await;
        }
    }
}

fn count_request_files(directory: &Path) -> Result<usize, CliError> {
    Ok(std::fs::read_dir(directory)
        .map_err(|e| CliError::Other(format!("delivery broker queue read failed: {e}")))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                return false;
            }
            std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && !metadata_is_reparse_point(&metadata)
            })
        })
        .take(MAX_IN_FLIGHT_REQUESTS)
        .count())
}

fn verify_envelope(
    envelope: &BrokerResponseEnvelope,
    expected_pubkey: &PublicKey,
) -> Result<(), CliError> {
    if envelope.attestation.pubkey != *expected_pubkey
        || envelope.attestation.kind.as_u16() != BROKER_RESPONSE_ATTESTATION_KIND
        || !envelope.attestation.tags.is_empty()
    {
        return Err(CliError::Other(
            "delivery broker response attestation identity or shape is invalid".into(),
        ));
    }
    envelope.attestation.verify().map_err(|e| {
        CliError::Other(format!(
            "delivery broker response signature verification failed: {e}"
        ))
    })?;
    let expected_content = broker_response_digest(&envelope.response).map_err(|e| {
        CliError::Other(format!(
            "delivery broker response canonicalization failed: {e}"
        ))
    })?;
    if envelope.attestation.content != expected_content {
        return Err(CliError::Other(
            "delivery broker response attestation did not match its payload".into(),
        ));
    }
    Ok(())
}

fn parse_response(request_id: Uuid, response: BrokerResponse) -> Result<Value, CliError> {
    if response.version != BROKER_PROTOCOL_VERSION || response.request_id != request_id {
        return Err(CliError::Other(
            "delivery broker returned a mismatched protocol response".into(),
        ));
    }
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => {
            let detail = format!("delivery broker {:?}: {}", error.code, error.message);
            match error.code {
                BrokerErrorCode::Busy => Err(CliError::Relay {
                    status: 503,
                    body: detail,
                }),
                BrokerErrorCode::RelayRejected => Err(CliError::Relay {
                    status: 400,
                    body: detail,
                }),
                BrokerErrorCode::DeliveryUnknown => Err(CliError::DeliveryUnknown(detail)),
                _ => Err(CliError::Other(detail)),
            }
        }
        _ => Err(CliError::Other(
            "delivery broker response must contain exactly one of result or error".into(),
        )),
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn read_bounded_regular_file(path: &Path, max_bytes: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut file = match open_read_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker response is not a regular file",
        ));
    }
    if metadata.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker response exceeds size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker response exceeds size limit",
        ));
    }
    Ok(Some(bytes))
}

fn open_read_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        // Open the reparse point itself instead of traversing it. The opened
        // handle's metadata is then checked for FILE_ATTRIBUTE_REPARSE_POINT.
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("broker"),
        Uuid::new_v4()
    ));

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp_path)?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;

    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    #[tokio::test]
    async fn request_and_response_round_trip_is_correlated() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        std::fs::create_dir(temp.path().join("responses")).expect("responses");
        let response_keys = Keys::generate();
        let client = DeliveryBrokerClient::new(
            temp.path().to_path_buf(),
            "secret".into(),
            response_keys.public_key(),
        )
        .expect("client");
        let request_dir = temp.path().join("requests");
        let response_dir = temp.path().join("responses");

        let server = tokio::spawn(async move {
            loop {
                let entry = std::fs::read_dir(&request_dir)
                    .expect("read requests")
                    .flatten()
                    .next();
                if let Some(entry) = entry {
                    let request: BrokerRequest =
                        serde_json::from_slice(&std::fs::read(entry.path()).expect("read request"))
                            .expect("decode request");
                    let response = BrokerResponse::success(
                        request.request_id,
                        serde_json::json!({"count": 3}),
                    );
                    let attestation = EventBuilder::new(
                        Kind::Custom(BROKER_RESPONSE_ATTESTATION_KIND),
                        broker_response_digest(&response).expect("response digest"),
                    )
                    .tags([])
                    .sign_with_keys(&response_keys)
                    .expect("sign response");
                    let envelope = BrokerResponseEnvelope {
                        response,
                        attestation,
                    };
                    let path = response_dir.join(format!("{}.json", request.request_id));
                    write_atomic(
                        &path,
                        &serde_json::to_vec(&envelope).expect("encode response"),
                    )
                    .expect("write response");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let value = client
            .call_with_timeout(
                BrokerOperation::Count {
                    filters: vec![serde_json::json!({"kinds": [9]})],
                },
                Duration::from_secs(2),
            )
            .await
            .expect("broker call");
        server.await.expect("server task");
        assert_eq!(value, serde_json::json!({"count": 3}));
    }

    #[tokio::test]
    async fn submitted_message_preserves_the_exact_signed_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        std::fs::create_dir(temp.path().join("responses")).expect("responses");
        let response_keys = Keys::generate();
        let client = DeliveryBrokerClient::new(
            temp.path().to_path_buf(),
            "secret".into(),
            response_keys.public_key(),
        )
        .expect("client");
        let event = EventBuilder::new(Kind::Custom(9), "exact\nstructured reply")
            .tags([])
            .sign_with_keys(&Keys::generate())
            .expect("event");
        let expected = event.clone();
        let request_dir = temp.path().join("requests");
        let response_dir = temp.path().join("responses");

        let server = tokio::spawn(async move {
            loop {
                if let Some(entry) = std::fs::read_dir(&request_dir)
                    .expect("read requests")
                    .flatten()
                    .next()
                {
                    let request: BrokerRequest =
                        serde_json::from_slice(&std::fs::read(entry.path()).expect("read request"))
                            .expect("decode request");
                    let BrokerOperation::SubmitStoredMessage { event } = request.operation else {
                        panic!("expected stored message operation");
                    };
                    assert_eq!(*event, expected);
                    let response = BrokerResponse::success(
                        request.request_id,
                        serde_json::json!({
                            "event_id": expected.id.to_hex(),
                            "accepted": true,
                            "delivery_path": "harness_broker",
                            "readback_verified": true,
                            "reconciled": false
                        }),
                    );
                    let attestation = EventBuilder::new(
                        Kind::Custom(BROKER_RESPONSE_ATTESTATION_KIND),
                        broker_response_digest(&response).expect("response digest"),
                    )
                    .tags([])
                    .sign_with_keys(&response_keys)
                    .expect("sign response");
                    let envelope = BrokerResponseEnvelope {
                        response,
                        attestation,
                    };
                    write_atomic(
                        &response_dir.join(format!("{}.json", request.request_id)),
                        &serde_json::to_vec(&envelope).expect("encode response"),
                    )
                    .expect("write response");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let raw = client
            .submit_message(&event)
            .await
            .expect("broker delivery");
        server.await.expect("server task");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("response json");
        assert_eq!(value["event_id"], event.id.to_hex());
        assert_eq!(value["delivery_path"], "harness_broker");
        assert_eq!(value["readback_verified"], true);
    }

    #[test]
    fn mismatched_response_id_fails_closed() {
        let expected = Uuid::new_v4();
        let response = BrokerResponse::success(Uuid::new_v4(), serde_json::json!([]));
        assert!(parse_response(expected, response).is_err());
    }

    #[test]
    fn forged_response_attestation_is_rejected() {
        let expected_keys = Keys::generate();
        let attacker_keys = Keys::generate();
        let response = BrokerResponse::success(Uuid::new_v4(), serde_json::json!([]));
        let attestation = EventBuilder::new(
            Kind::Custom(BROKER_RESPONSE_ATTESTATION_KIND),
            broker_response_digest(&response).expect("response digest"),
        )
        .tags([])
        .sign_with_keys(&attacker_keys)
        .expect("sign response");
        let envelope = BrokerResponseEnvelope {
            response,
            attestation,
        };

        assert!(verify_envelope(&envelope, &expected_keys.public_key()).is_err());
    }

    #[test]
    fn response_attestation_binds_result_request_and_protocol() {
        let response_keys = Keys::generate();
        let response = BrokerResponse::success(Uuid::new_v4(), serde_json::json!({"count": 1}));
        let attestation = EventBuilder::new(
            Kind::Custom(BROKER_RESPONSE_ATTESTATION_KIND),
            broker_response_digest(&response).expect("response digest"),
        )
        .tags([])
        .sign_with_keys(&response_keys)
        .expect("sign response");
        let envelope = BrokerResponseEnvelope {
            response,
            attestation,
        };
        verify_envelope(&envelope, &response_keys.public_key()).expect("original envelope");

        let mut mutated_result = envelope.clone();
        mutated_result.response.result = Some(serde_json::json!({"count": 2}));
        assert!(verify_envelope(&mutated_result, &response_keys.public_key()).is_err());

        let mut mutated_request = envelope.clone();
        mutated_request.response.request_id = Uuid::new_v4();
        assert!(verify_envelope(&mutated_request, &response_keys.public_key()).is_err());

        let mut mutated_protocol = envelope;
        mutated_protocol.response.version = BROKER_PROTOCOL_VERSION.saturating_add(1);
        assert!(verify_envelope(&mutated_protocol, &response_keys.public_key()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn response_reader_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let link = temp.path().join("response.json");
        std::fs::write(&target, b"forged").expect("target");
        symlink(&target, &link).expect("symlink");
        assert!(read_bounded_regular_file(&link, 1024).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn response_reader_does_not_follow_reparse_point_symlinks() {
        use std::os::windows::fs::symlink_file;

        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let link = temp.path().join("response.json");
        std::fs::write(&target, b"forged").expect("target");
        if symlink_file(&target, &link).is_err() {
            // Windows requires Developer Mode or SeCreateSymbolicLinkPrivilege.
            return;
        }
        assert!(read_bounded_regular_file(&link, 1024).is_err());
    }

    #[tokio::test]
    async fn queue_cap_rejects_before_publishing_another_request() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        std::fs::create_dir(temp.path().join("responses")).expect("responses");
        for index in 0..MAX_IN_FLIGHT_REQUESTS {
            let child = if index % 2 == 0 {
                "requests"
            } else {
                "processing"
            };
            std::fs::write(temp.path().join(child).join(format!("{index}.json")), b"x")
                .expect("in-flight file");
        }
        let client = DeliveryBrokerClient::new(
            temp.path().to_path_buf(),
            "secret".into(),
            Keys::generate().public_key(),
        )
        .expect("client");
        let error = client
            .call_with_timeout(
                BrokerOperation::Count {
                    filters: vec![serde_json::json!({"kinds": [9]})],
                },
                Duration::from_millis(1),
            )
            .await
            .expect_err("queue must be full");
        assert!(matches!(error, CliError::Relay { status: 503, .. }));
        assert!(crate::error::is_retryable_error(&error));
        assert!(error.to_string().contains("in-flight limit"));
    }

    #[test]
    fn signed_busy_response_is_a_retryable_503() {
        let request_id = Uuid::new_v4();
        let response = BrokerResponse::failure(
            request_id,
            BrokerErrorCode::Busy,
            "delivery broker is at its in-flight limit",
        );

        let error = parse_response(request_id, response).expect_err("busy must fail");
        assert!(matches!(error, CliError::Relay { status: 503, .. }));
        assert!(crate::error::is_retryable_error(&error));
    }
}
