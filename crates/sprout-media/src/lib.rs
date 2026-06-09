//! Media storage, validation, and thumbnail generation for Sprout.
//!
//! Library crate — no Axum dependency for handlers. Axum handlers live in `sprout-relay`.

pub mod auth;
pub mod config;
pub mod error;
pub mod storage;
pub mod thumbnail;
pub mod types;
pub mod upload;
pub mod validation;

pub use config::MediaConfig;
pub use error::MediaError;
pub use storage::{BlobHeadMeta, BlobMeta, ByteStream, MediaStorage};
pub use types::BlobDescriptor;
pub use upload::{process_file_upload, process_upload, process_video_upload};
pub use validation::{serve_inline, validate_video_file, VideoMeta};
