//! Immutable capabilities for Buzz Desktop's April Pocket TTS bundle.

/// Pinned upstream export repository.
pub const APRIL_MODEL_ID: &str = "KevinAHM/pocket-tts-onnx";

/// Pinned revision containing the `english_2026-04` bundle.
pub const APRIL_MODEL_REVISION: &str = "58a6d00cf13d239b6748cb0769f35c580a8f606c";

/// Language bundle selected from the pinned export.
pub const APRIL_BUNDLE_ID: &str = "english_2026-04";

/// Maximum input size declared by the April bundle.
pub const APRIL_MAX_TOKEN_PER_CHUNK: usize = 50;

/// One immutable artifact required by the April INT8 runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PocketModelArtifact {
    pub filename: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub quantized: bool,
}

/// Capabilities of Buzz Desktop's sole Pocket model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PocketModelInfo {
    /// Language bundle selected from the pinned export.
    pub bundle_id: &'static str,
    /// Upstream model repository.
    pub source_model_id: &'static str,
    /// Pinned upstream model revision.
    pub revision: &'static str,
    /// PCM output sample rate.
    pub sample_rate: u32,
    /// Maximum input size declared by the bundle.
    pub max_token_per_chunk: usize,
    /// Immutable files required by the runtime.
    pub artifacts: &'static [PocketModelArtifact],
    /// Components quantized in the selected bundle.
    pub quantized_components: &'static [&'static str],
}

const INT8_ARTIFACTS: [PocketModelArtifact; 8] = [
    PocketModelArtifact {
        filename: "bundle.json",
        sha256: "bab643150f437f37df080a710520ff39ed9ebd9a339f8ebdc739f7eddfc28b3f",
        size_bytes: 24_381,
        quantized: false,
    },
    PocketModelArtifact {
        filename: "bos_before_voice.npy",
        sha256: "f46edf4f7007b7ba4ea58831f49d003e59e167b4641c44bb3addfe9231a780b1",
        size_bytes: 4_224,
        quantized: false,
    },
    PocketModelArtifact {
        filename: "tokenizer.model",
        sha256: "d461765ae179566678c93091c5fa6f2984c31bbe990bf1aa62d92c64d91bc3f6",
        size_bytes: 59_339,
        quantized: false,
    },
    PocketModelArtifact {
        filename: "flow_lm_main_int8.onnx",
        sha256: "f9bd8106b79a0192c1c43399ab938fb24900a95c1c599870d75a884e99000116",
        size_bytes: 76_341_079,
        quantized: true,
    },
    PocketModelArtifact {
        filename: "flow_lm_flow_int8.onnx",
        sha256: "3dd781ee5abee9e195320bf0106bebd6372a852b3b36352524ee78b40554635d",
        size_bytes: 9_962_530,
        quantized: true,
    },
    PocketModelArtifact {
        filename: "mimi_decoder_int8.onnx",
        sha256: "3630450a3297a101792a6ac66619ebc70ab916b265e6220c2afaef8b1673f925",
        size_bytes: 22_684_077,
        quantized: true,
    },
    PocketModelArtifact {
        filename: "mimi_encoder.onnx",
        sha256: "853e2ca623b8782d94c3745ec6133bfdff7ce33d9b11128bd29ea03f28d76e3d",
        size_bytes: 39_768_446,
        quantized: false,
    },
    PocketModelArtifact {
        filename: "text_conditioner.onnx",
        sha256: "4ecee995fb69f85c7a7493d11f7b5ee15d9950facc7ab3f5c9c49ef1e03847bb",
        size_bytes: 16_388_344,
        quantized: false,
    },
];

const INT8_COMPONENTS: [&str; 3] = ["flow_lm_main", "flow_lm_flow", "mimi_decoder"];

/// Return immutable metadata for Buzz Desktop's April INT8 model.
pub const fn april_model_info() -> PocketModelInfo {
    PocketModelInfo {
        bundle_id: APRIL_BUNDLE_ID,
        source_model_id: APRIL_MODEL_ID,
        revision: APRIL_MODEL_REVISION,
        sample_rate: 24_000,
        max_token_per_chunk: APRIL_MAX_TOKEN_PER_CHUNK,
        artifacts: &INT8_ARTIFACTS,
        quantized_components: &INT8_COMPONENTS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_matches_pinned_int8_layout() {
        let info = april_model_info();
        assert_eq!(info.artifacts.len(), 8);
        assert_eq!(
            info.quantized_components,
            ["flow_lm_main", "flow_lm_flow", "mimi_decoder"]
        );
        assert_eq!(
            info.artifacts
                .iter()
                .map(|artifact| artifact.size_bytes)
                .sum::<u64>(),
            165_232_420
        );
        assert!(info
            .artifacts
            .iter()
            .any(|artifact| { artifact.filename == "mimi_encoder.onnx" && !artifact.quantized }));
        assert!(!info
            .artifacts
            .iter()
            .any(|artifact| artifact.filename == "mimi_encoder_int8.onnx"));
    }
}
