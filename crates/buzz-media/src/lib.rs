//! Media storage, validation, and thumbnail generation for Buzz.
//!
//! Library crate — no Axum dependency for handlers. Axum handlers live in `buzz-relay`.

pub mod auth;
pub mod bucket_index;
pub mod config;
pub mod error;
pub mod publication;
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
pub use config::{MediaConfig, S3AddressingStyle};
pub use error::MediaError;
pub use publication::{
    media_publication_manifest_key, MediaPublicationManifest, StagedMediaPublication,
    VerifiedMediaObject,
};
pub use storage::{BlobHeadMeta, BlobMeta, ByteStream, MediaStorage};
pub use types::BlobDescriptor;
pub use upload::{
    process_file_upload, process_upload, process_video_upload, stage_file_upload, stage_upload,
    stage_video_upload, StagedMediaUpload,
};
pub use upload_record::{
    load_upload_record_plan, parse_port, parse_public_ip, stage_upload_record, upload_record_key,
    upload_record_plan_key, StagedUploadRecord, UploadAttribution, UploadNetworkInfo,
    UploadProjectionResult, UploadRecord, UPLOAD_RECORD_VERSION,
};
pub use validation::{looks_like_iso_bmff, serve_inline, validate_video_file, VideoMeta};
