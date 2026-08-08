//! Media storage, validation, and thumbnail generation for Buzz.
//!
//! Library crate — no Axum dependency for handlers. Axum handlers live in `buzz-relay`.

pub mod auth;
pub mod bucket_index;
pub mod config;
pub mod error;
pub mod keys;
pub mod migration;
pub mod storage;
pub mod thumbnail;
pub mod types;
pub mod upload;
pub mod upload_record;
pub mod validation;

pub use bucket_index::{
    classify_key, fold_bucket_listing, BucketAggregate, BucketSnapshot, CommunityStorage, KeyClass,
    Page, SweepError,
};
pub use config::{MediaConfig, MediaMigrationPhase, S3AddressingStyle};
pub use error::MediaError;
pub use keys::{
    legacy_blob_key, legacy_thumb_key, read_candidates, sharded_blob_key, sharded_thumb_key,
    MediaKeyError, MediaReadCandidates,
};
pub use storage::{
    BlobHeadMeta, BlobMeta, BlobStream, ByteStream, MediaStorage, PayloadByteRange,
    PayloadRangeRead,
};
pub use types::BlobDescriptor;
pub use upload::{process_file_upload, process_upload, process_video_upload};
pub use upload_record::{
    parse_port, parse_public_ip, upload_record_key, UploadAttribution, UploadNetworkInfo,
    UploadRecord, UPLOAD_RECORD_VERSION,
};
pub use validation::{looks_like_iso_bmff, serve_inline, validate_video_file, VideoMeta};
