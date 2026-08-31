//! Live round-trip test for the **static-credentials** S3 path against an
//! S3-compatible service. It is guarded by `#[ignore]`.
//!
//! This is the path local/dev and any static-key deployment uses
//! (`s3_access_key`/`s3_secret_key` both non-empty -> `Credentials::new`). It
//! exists to prove that adding the IRSA/credential-chain fallback did **not**
//! regress hardcoded credentials.
//!
//! Run it against the docker-compose MinIO (creds `buzz_dev`/`buzz_dev_secret`,
//! bucket `buzz-media`, endpoint `http://localhost:9000`):
//!
//! ```bash
//! docker compose up -d minio minio-init
//! cargo test -p buzz-media --test static_creds_minio -- --ignored
//! ```
//!
//! Overridable via `BUZZ_S3_ENDPOINT` / `BUZZ_S3_ACCESS_KEY` /
//! `BUZZ_S3_SECRET_KEY` / `BUZZ_S3_BUCKET` / `BUZZ_S3_REGION` /
//! `BUZZ_S3_ADDRESSING_STYLE`. The default remains `path` for MinIO.

use buzz_media::storage::MediaStorage;
use buzz_object_store::{S3ObjectStore, S3StoreConfig};

fn minio_config() -> S3StoreConfig {
    S3StoreConfig {
        endpoint: std::env::var("BUZZ_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string()),
        access_key: std::env::var("BUZZ_S3_ACCESS_KEY").unwrap_or_else(|_| "buzz_dev".to_string()),
        secret_key: std::env::var("BUZZ_S3_SECRET_KEY")
            .unwrap_or_else(|_| "buzz_dev_secret".to_string()),
        bucket: std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-media".to_string()),
        region: std::env::var("BUZZ_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        addressing_style: std::env::var("BUZZ_S3_ADDRESSING_STYLE")
            .unwrap_or_else(|_| "path".to_string())
            .parse()
            .expect("BUZZ_S3_ADDRESSING_STYLE must be path or virtual"),
    }
}

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn static_creds_round_trip_against_minio() {
    let store = S3ObjectStore::new(&minio_config())
        .expect("static creds should build an object-store client");
    let storage = MediaStorage::with_store(std::sync::Arc::new(store));

    let key = format!("_test/static-creds-{}.bin", std::process::id());
    let body = b"hardcoded-creds-still-work";

    // PUT
    storage
        .put(&key, body, "application/octet-stream")
        .await
        .expect("put with static creds should succeed");

    // HEAD -> exists with correct size
    assert!(storage.head(&key).await.expect("head should succeed"));
    let meta = storage
        .head_with_metadata(&key)
        .await
        .expect("head_with_metadata should succeed")
        .expect("object should exist");
    assert_eq!(meta.size, body.len() as u64);

    // GET round-trips the bytes
    let got = storage.get(&key).await.expect("get should succeed");
    assert_eq!(got, body);

    // DELETE, then HEAD reports absence
    storage.delete(&key).await.expect("delete should succeed");
    assert!(!storage.head(&key).await.expect("head after delete"));
}
