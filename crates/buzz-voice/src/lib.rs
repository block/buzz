//! Reusable local voice primitives for Buzz.

pub mod imported;
pub mod pocket;

pub use pocket::{
    april_model_info, load_text_to_speech, load_voice_style, PocketModelInfo, PocketTts,
    VoiceStyle, DEFAULT_VOICE, SAMPLE_RATE, VOICE_FILE_EXT,
};

/// One immutable artifact required by the April Pocket bundle.
///
/// `filename` is the bundle-relative file name, `sha256` pins its contents,
/// `size_bytes` supports download progress and validation, and `quantized`
/// identifies the INT8 components.
pub type PocketModelArtifact = pocket::PocketModelArtifact;

/// Language bundle selected from the pinned export.
pub const APRIL_BUNDLE_ID: &str = pocket::APRIL_BUNDLE_ID;
/// Pinned upstream export repository.
pub const APRIL_MODEL_ID: &str = pocket::APRIL_MODEL_ID;
/// Pinned revision containing the April bundle.
pub const APRIL_MODEL_REVISION: &str = pocket::APRIL_MODEL_REVISION;
