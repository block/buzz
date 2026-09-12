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

use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_media::config::MediaConfig;
use buzz_media::storage::{BlobMeta, MediaStorage};
use buzz_media::{
    process_file_upload_with_hints, FileUploadHints, MediaError, UploadAttribution, UploadRecord,
};
use bytes::Bytes;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};

fn minio_config() -> MediaConfig {
    MediaConfig {
        s3_endpoint: std::env::var("BUZZ_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string()),
        s3_access_key: std::env::var("BUZZ_S3_ACCESS_KEY")
            .unwrap_or_else(|_| "buzz_dev".to_string()),
        s3_secret_key: std::env::var("BUZZ_S3_SECRET_KEY")
            .unwrap_or_else(|_| "buzz_dev_secret".to_string()),
        s3_bucket: std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-media".to_string()),
        s3_region: std::env::var("BUZZ_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        s3_addressing_style: std::env::var("BUZZ_S3_ADDRESSING_STYLE")
            .unwrap_or_else(|_| "path".to_string())
            .parse()
            .expect("BUZZ_S3_ADDRESSING_STYLE must be path or virtual"),
        max_image_bytes: 50 * 1024 * 1024,
        max_gif_bytes: 10 * 1024 * 1024,
        max_video_bytes: 524_288_000,
        max_file_bytes: 104_857_600,
        public_base_url: "http://localhost:3000/media".to_string(),
        calendar_classification_enabled: true,
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    }
}

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn static_creds_round_trip_against_minio() {
    let storage =
        MediaStorage::new(&minio_config()).expect("static creds should build a storage client");

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

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn calendar_reupload_preserves_preexisting_generic_url() {
    let config = minio_config();
    let storage = MediaStorage::new(&config).expect("static creds should build a storage client");
    let body = Bytes::from(format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nSUMMARY:Planning-{}\r\nEND:VCALENDAR\r\n",
        uuid::Uuid::new_v4()
    ));
    let sha256 = hex::encode(Sha256::digest(&body));
    let bin_key = format!("{sha256}.bin");
    let ics_key = format!("{sha256}.ics");
    let tenant = TenantContext::resolved(
        CommunityId::from_uuid(uuid::Uuid::new_v4()),
        "media.example.com",
    );
    let sidecar_key = MediaStorage::ctx_sidecar_key(&tenant, &sha256);

    assert!(!storage.head(&bin_key).await.expect("head fresh bin key"));
    assert!(!storage.head(&ics_key).await.expect("head fresh ics key"));
    assert!(!storage
        .head(&sidecar_key)
        .await
        .expect("head fresh sidecar"));
    storage
        .put(&bin_key, &body, "application/octet-stream")
        .await
        .expect("seed generic blob");
    let original_meta = BlobMeta {
        dim: String::new(),
        blurhash: String::new(),
        thumb_url: String::new(),
        size: body.len() as u64,
        ext: "bin".to_string(),
        mime_type: "application/octet-stream".to_string(),
        uploaded_at: 1_700_000_000,
        duration_secs: None,
    };
    storage
        .put_sidecar(&tenant, &sha256, &original_meta)
        .await
        .expect("seed generic sidecar");

    let keys = Keys::generate();
    let expiration = (Timestamp::now().as_secs() + 300).to_string();
    let auth = EventBuilder::new(Kind::from(24242), "Upload calendar")
        .tags([
            Tag::parse(["t", "upload"]).expect("t tag"),
            Tag::parse(["x", &sha256]).expect("x tag"),
            Tag::parse(["expiration", &expiration]).expect("expiration tag"),
        ])
        .sign_with_keys(&keys)
        .expect("sign upload auth");

    let descriptor = process_file_upload_with_hints(
        &storage,
        &config,
        &tenant,
        &auth,
        body.clone(),
        None,
        FileUploadHints {
            declared_mime: Some("text/calendar".to_string()),
            extension: Some("ics".to_string()),
        },
    )
    .await
    .expect("calendar re-upload");
    let preserved_meta = storage
        .get_sidecar(&tenant, &sha256)
        .await
        .expect("read preserved sidecar");
    let bin_exists = storage.head(&bin_key).await.expect("head bin");
    let ics_exists = storage.head(&ics_key).await.expect("head ics");

    for key in [&bin_key, &ics_key, &sidecar_key] {
        storage.delete(key).await.expect("clean fixture");
    }

    assert!(descriptor.url.ends_with(&format!("/{sha256}.bin")));
    assert_eq!(descriptor.mime_type, "application/octet-stream");
    assert_eq!(preserved_meta.ext, "bin");
    assert_eq!(preserved_meta.mime_type, "application/octet-stream");
    assert!(bin_exists);
    assert!(
        !ics_exists,
        "re-upload must not create a competing .ics blob"
    );
}

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn classification_claim_first_writer_wins_atomically() {
    let storage =
        MediaStorage::new(&minio_config()).expect("static creds should build a storage client");
    let sha256 = hex::encode(Sha256::digest(uuid::Uuid::new_v4().as_bytes()));
    let tenant = TenantContext::resolved(
        CommunityId::from_uuid(uuid::Uuid::new_v4()),
        "media.example.com",
    );
    let claim_key = MediaStorage::classification_claim_key(&tenant, &sha256);
    let generic_meta = BlobMeta {
        ext: "bin".to_string(),
        mime_type: "application/octet-stream".to_string(),
        ..BlobMeta::default()
    };
    let calendar_meta = BlobMeta {
        ext: "ics".to_string(),
        mime_type: "text/calendar".to_string(),
        ..BlobMeta::default()
    };

    assert!(!storage
        .head(&claim_key)
        .await
        .expect("head fresh classification claim"));
    let (generic_created, calendar_created) = tokio::join!(
        storage.put_classification_claim_if_absent(&tenant, &sha256, &generic_meta),
        storage.put_classification_claim_if_absent(&tenant, &sha256, &calendar_meta),
    );
    let generic_created = generic_created.expect("generic conditional put");
    let calendar_created = calendar_created.expect("calendar conditional put");
    let preserved = storage
        .get_classification_claim(&tenant, &sha256)
        .await
        .expect("read winning classification claim");

    storage
        .delete(&claim_key)
        .await
        .expect("clean unique claim fixture");

    assert_ne!(generic_created, calendar_created, "exactly one writer wins");
    if generic_created {
        assert_eq!(preserved.ext, generic_meta.ext);
        assert_eq!(preserved.mime_type, generic_meta.mime_type);
    } else {
        assert_eq!(preserved.ext, calendar_meta.ext);
        assert_eq!(preserved.mime_type, calendar_meta.mime_type);
    }
}

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn upload_record_uses_first_writer_classification_claim() {
    let config = minio_config();
    let storage = MediaStorage::new(&config).expect("static creds should build a storage client");
    let body = Bytes::from(format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nSUMMARY:Claim-{}\r\nEND:VCALENDAR\r\n",
        uuid::Uuid::new_v4()
    ));
    let sha256 = hex::encode(Sha256::digest(&body));
    let bin_key = format!("{sha256}.bin");
    let ics_key = format!("{sha256}.ics");
    let tenant = TenantContext::resolved(
        CommunityId::from_uuid(uuid::Uuid::new_v4()),
        "media.example.com",
    );
    let claim_key = MediaStorage::classification_claim_key(&tenant, &sha256);
    let sidecar_key = MediaStorage::ctx_sidecar_key(&tenant, &sha256);
    let generic_meta = BlobMeta {
        ext: "bin".to_string(),
        mime_type: "application/octet-stream".to_string(),
        size: body.len() as u64,
        uploaded_at: 1_700_000_000,
        ..BlobMeta::default()
    };

    storage
        .put(&bin_key, &body, &generic_meta.mime_type)
        .await
        .expect("seed claimed generic blob");
    assert!(storage
        .put_classification_claim_if_absent(&tenant, &sha256, &generic_meta)
        .await
        .expect("seed classification claim"));

    let keys = Keys::generate();
    let expiration = (Timestamp::now().as_secs() + 300).to_string();
    let event = EventBuilder::new(Kind::from(24242), "Upload calendar")
        .tags([
            Tag::parse(["t", "upload"]).expect("t tag"),
            Tag::parse(["x", &sha256]).expect("x tag"),
            Tag::parse(["expiration", &expiration]).expect("expiration tag"),
        ])
        .sign_with_keys(&keys)
        .expect("sign upload auth");
    let descriptor = process_file_upload_with_hints(
        &storage,
        &config,
        &tenant,
        &event,
        body.clone(),
        Some(UploadAttribution::default()),
        FileUploadHints {
            declared_mime: Some("text/calendar".to_string()),
            extension: Some("ics".to_string()),
        },
    )
    .await
    .expect("upload must use the claimed generic classification");
    let retry_descriptor = process_file_upload_with_hints(
        &storage,
        &config,
        &tenant,
        &event,
        body,
        Some(UploadAttribution::default()),
        FileUploadHints {
            declared_mime: Some("text/calendar".to_string()),
            extension: Some("ics".to_string()),
        },
    )
    .await
    .expect("same signed upload retry must repair in place");

    let sidecar = storage
        .get_sidecar(&tenant, &sha256)
        .await
        .expect("read published sidecar");
    let upload_prefix = format!("_uploads/{}/{sha256}/", tenant.community());
    let upload_page = storage
        .list_prefix_page(&upload_prefix, None, 10)
        .await
        .expect("list upload record");
    assert_eq!(upload_page.objects.len(), 1);
    let record_key = upload_page.objects[0].0.clone();
    let record: UploadRecord =
        serde_json::from_slice(&storage.get(&record_key).await.expect("read upload record"))
            .expect("parse upload record");

    for key in [&record_key, &sidecar_key, &claim_key, &bin_key, &ics_key] {
        storage.delete(key).await.expect("clean unique fixture");
    }

    assert!(descriptor.url.ends_with(&format!("/{sha256}.bin")));
    assert_eq!(retry_descriptor.url, descriptor.url);
    assert_eq!(descriptor.mime_type, generic_meta.mime_type);
    assert_eq!(sidecar.ext, generic_meta.ext);
    assert_eq!(record.ext, generic_meta.ext);
    assert_eq!(record.mime_type, generic_meta.mime_type);
}

#[tokio::test]
#[ignore = "requires a live MinIO (docker compose up -d minio minio-init)"]
async fn calendar_reupload_does_not_restore_blob_behind_existing_serve_gate() {
    let config = minio_config();
    let storage = MediaStorage::new(&config).expect("static creds should build a storage client");
    let body = Bytes::from(format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nSUMMARY:Missing-{}\r\nEND:VCALENDAR\r\n",
        uuid::Uuid::new_v4()
    ));
    let sha256 = hex::encode(Sha256::digest(&body));
    let bin_key = format!("{sha256}.bin");
    let ics_key = format!("{sha256}.ics");
    let tenant = TenantContext::resolved(
        CommunityId::from_uuid(uuid::Uuid::new_v4()),
        "media.example.com",
    );
    let sidecar_key = MediaStorage::ctx_sidecar_key(&tenant, &sha256);
    let original_meta = BlobMeta {
        size: body.len() as u64,
        ext: "bin".to_string(),
        mime_type: "application/octet-stream".to_string(),
        uploaded_at: 1_700_000_000,
        ..BlobMeta::default()
    };
    storage
        .put_sidecar(&tenant, &sha256, &original_meta)
        .await
        .expect("seed sidecar without canonical blob");

    let keys = Keys::generate();
    let expiration = (Timestamp::now().as_secs() + 300).to_string();
    let auth = EventBuilder::new(Kind::from(24242), "Upload calendar")
        .tags([
            Tag::parse(["t", "upload"]).expect("t tag"),
            Tag::parse(["x", &sha256]).expect("x tag"),
            Tag::parse(["expiration", &expiration]).expect("expiration tag"),
        ])
        .sign_with_keys(&keys)
        .expect("sign upload auth");

    let result = process_file_upload_with_hints(
        &storage,
        &config,
        &tenant,
        &auth,
        body,
        None,
        FileUploadHints {
            declared_mime: Some("text/calendar".to_string()),
            extension: Some("ics".to_string()),
        },
    )
    .await;
    let bin_exists = storage.head(&bin_key).await.expect("head bin");
    let ics_exists = storage.head(&ics_key).await.expect("head ics");

    storage
        .delete(&sidecar_key)
        .await
        .expect("clean unique sidecar fixture");

    assert!(matches!(result, Err(MediaError::StorageError(_))));
    assert!(!bin_exists, "must not republish through the existing gate");
    assert!(!ics_exists, "must not create a competing classified blob");
}
