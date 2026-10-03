//! Model download manager for STT (Parakeet family) and TTS (Pocket TTS) models.
//!
//! The STT model is selectable via the `STT_MODELS` registry (issue #2478):
//! the English Parakeet TDT-CTC 110M default, multilingual Parakeet TDT 0.6B
//! v3, or SenseVoiceSmall, chosen by `BUZZ_STT_MODEL` / system locale in
//! `selected_stt_model`.
//!
//! Mental model:
//!   app launch → start_stt_download (background) → ~/.buzz/models/<selected>/
//!   app launch → start_tts_download (background) → ~/.buzz/models/pocket-tts/
//!   STT pipeline → is_stt_ready() → stt_model_dir() → run inference
//!   TTS pipeline → is_tts_ready() → tts_model_dir() → run synthesis
//!
//! Models are downloaded once and cached. A version manifest (`.buzz-model-manifest`)
//! is written alongside model files — if the on-disk version doesn't match the
//! compiled-in version, the model is re-downloaded.
//!
//! Upgrade note: an older Moonshine STT model directory at
//! `~/.buzz/models/moonshine-tiny/` is removed best-effort once the new STT
//! model finishes installing successfully. Cleanup is gated on the new model
//! being Ready, so a failed download never removes the previous on-disk model
//! during migration. If removal fails (permissions, etc.) the leftover is
//! harmless and can be removed by hand.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::pocket::{
    april_model_info, PocketModelArtifact, APRIL_BUNDLE_ID, APRIL_MODEL_ID, APRIL_MODEL_REVISION,
};
use super::tts_voice_registry::POCKET_VOICES;

#[path = "models_voice_upgrade.rs"]
mod voice_upgrade;

// ── Integrity verification ────────────────────────────────────────────────────
//
// Model artifacts are verified against pinned SHA-256 hashes before
// installation. This is defense-in-depth: HTTPS protects the transport,
// hashes protect the content.
//
// To recompute hashes: download each file, run `shasum -a 256 <file>`, and
// update the corresponding constant.
//
// STT archive hashes are pinned per model in the `STT_MODELS` registry below
// (`SttModel::archive_sha256`). Both shipped models are pinned; the field is
// `Option` so a model may temporarily ship `None` (size cap + safe extraction
// + expected-files verification still apply) before its hash is computed.

fn pocket_artifact_url(filename: &str) -> String {
    format!(
        "https://huggingface.co/{APRIL_MODEL_ID}/resolve/{APRIL_MODEL_REVISION}/onnx/{APRIL_BUNDLE_ID}/{filename}"
    )
}

fn pocket_license_url() -> String {
    format!("https://huggingface.co/{APRIL_MODEL_ID}/resolve/{APRIL_MODEL_REVISION}/onnx/LICENSE")
}

/// Reference voice WAV: "Mary (f, conversation)" from the Kyutai TTS demo
/// voice set — VCTK speaker p333, ai-coustics-enhanced. Pinned to
/// kyutai/tts-voices commit 323332d33f997de8394f24a193e1a76df720e01a.
///
/// Mapping comes from the speaker dropdown on <https://kyutai.org/tts>:
/// the Pocket TTS preset "Mary (f, conversation)" maps to
/// `vctk/p333_023_enhanced.wav`. We rename to `reference_sample.wav` on disk
/// so the rest of the engine code stays voice-agnostic; the friendly label
/// only matters for attribution and PR-body docs.
const POCKET_REFERENCE_WAV_URL: &str =
    "https://huggingface.co/kyutai/tts-voices/resolve/323332d33f997de8394f24a193e1a76df720e01a/vctk/p333_023_enhanced.wav";

const TTS_LICENSE_ARTIFACT: PocketModelArtifact = PocketModelArtifact {
    filename: "LICENSE",
    sha256: "fe7b4ce83b8381cc5b216bbb4af73c570688d1b819c73bbaed8ca401f4677cd6",
    size_bytes: 18_655,
    quantized: false,
};

const TTS_REFERENCE_ARTIFACT: PocketModelArtifact = PocketModelArtifact {
    filename: "reference_sample.wav",
    sha256: "a35b0468382218e9f37a9a7494d1e4b74deaf18d7ced22265b4e325bb55c183f",
    size_bytes: 639_084,
    quantized: false,
};

// ── Model versioning ──────────────────────────────────────────────────────────
//
// A version manifest is written alongside model files after successful download.
// If the on-disk manifest doesn't match the compiled-in version, the model is
// considered stale and re-downloaded. Increment when upgrading model files.

// STT manifest versions are per model — see `SttModel::version` in the
// `STT_MODELS` registry below.

/// Identifies the April INT8 asset set plus the official VCTK presets.
const TTS_MODEL_VERSION: &str = "5";

/// Filename for the version manifest written alongside model files.
const MANIFEST_FILENAME: &str = ".buzz-model-manifest";

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum expected Pocket TTS file size. The largest pinned INT8 artifact is
/// `flow_lm_main_int8.onnx` at 76,341,079 bytes.
const MAX_TTS_FILE_BYTES: u64 = 100 * 1024 * 1024;

// ── STT model registry (multilingual — issue #2478) ───────────────────────────
//
// Historically the huddle STT model was hard-pinned to an English-only build,
// so non-English speech transcribed as garbage (issue #2478). The registry
// makes the model selectable: the English default stays the default, and each
// supported system locale maps to a model that actually covers its language
// (or is forced with the `BUZZ_STT_MODEL` env override). Adding a model is data
// here plus, for a new sherpa-onnx model family, one match arm in `stt.rs`.

/// Attribution sidecar filename written next to every STT model's files.
const STT_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";

/// sherpa-onnx model family — decides how the offline recognizer is configured
/// (`stt.rs`) and which ONNX files must be present on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttFamily {
    /// Single-file NeMo CTC head (e.g. Parakeet TDT-CTC 110M English).
    NemoCtc,
    /// NeMo transducer: encoder + decoder + joiner (e.g. Parakeet TDT 0.6B v3).
    Transducer,
    /// Single-file SenseVoice model with language auto-detection and ITN.
    SenseVoice,
}

/// One selectable speech-to-text model. `STT_MODELS` is the single source of
/// truth; `select_stt_model` picks one at startup.
pub struct SttModel {
    /// Stable id used by the `BUZZ_STT_MODEL` override and in logs.
    pub id: &'static str,
    /// Directory name under `~/.buzz/models/`.
    pub dir_name: &'static str,
    /// Download URL for the `.tar.bz2` archive.
    pub download_url: &'static str,
    /// Directory name produced by `tar xjf` on the archive.
    pub archive_subdir: &'static str,
    /// SHA-256 of the archive, or `None` if not yet pinned. `None` still
    /// enforces the size cap, safe extraction, and expected-files check — it
    /// only skips the content hash. The English default is always `Some`.
    pub archive_sha256: Option<&'static str>,
    /// Hard cap on the downloaded archive size, in bytes.
    pub max_download_bytes: u64,
    /// Model files (excluding the license sidecar) required for "ready".
    pub model_files: &'static [&'static str],
    /// sherpa-onnx model family.
    pub family: SttFamily,
    /// Manifest version — bump to force re-download of this model.
    pub version: &'static str,
    /// Model-specific license and attribution notice written next to the bytes.
    pub license_text: &'static str,
    /// Human-readable language coverage (About dialog / logs).
    pub languages: &'static str,
    /// Primary locale tags that may automatically select this model.
    pub auto_select_languages: &'static [&'static str],
}

/// Registry of selectable STT models. Index 0 is the default (English).
static STT_MODELS: &[SttModel] = &[
    // NVIDIA Parakeet TDT-CTC 110M (English, int8) — packaged for sherpa-onnx
    // by k2-fsa. Single ONNX file (CTC head) + tokens.txt. Avg WER ~7.5% across
    // the OpenASR-style benchmarks; CTC blank-token decoding eliminates the
    // silence/cut-audio hallucination class that hurts encoder-decoder models
    // on noisy huddle audio. This remains the default for English.
    SttModel {
        id: "parakeet-en",
        dir_name: "parakeet-tdt-ctc-110m-en",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
             sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8.tar.bz2",
        archive_subdir: "sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8",
        archive_sha256: Some("17f945007b52ccd8b7200ffc7c5652e9e8e961dfdf479cefcabd06cf5703630b"),
        max_download_bytes: 200 * 1024 * 1024,
        model_files: &["model.int8.onnx", "tokens.txt"],
        family: SttFamily::NemoCtc,
        version: "2",
        license_text: STT_EN_LICENSE_TEXT,
        languages: "English",
        auto_select_languages: &["en"],
    },
    // NVIDIA Parakeet TDT 0.6B v3 (multilingual, int8) — packaged for
    // sherpa-onnx by k2-fsa. Transducer family (encoder/decoder/joiner). Auto
    // language-ID + punctuation across 25 European languages. Locale selection
    // is restricted to those languages instead of treating it as a universal
    // non-English fallback (issue #2478).
    //
    // Checksum: upstream publishes no SHA-256, so this hash was computed from
    // the k2-fsa `asr-models` release archive
    // (sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2, ~465 MB) with
    // `shasum -a 256`. Recompute and re-pin if k2-fsa republishes the asset.
    SttModel {
        id: "parakeet-v3",
        dir_name: "parakeet-tdt-0.6b-v3",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
             sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
        archive_subdir: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        archive_sha256: Some("5793d0fd397c5778d2cf2126994d58e9d56b1be7c04d13c7a15bb1b4eafb16bf"),
        max_download_bytes: 800 * 1024 * 1024,
        model_files: &[
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
        ],
        family: SttFamily::Transducer,
        version: "1",
        license_text: STT_V3_LICENSE_TEXT,
        languages: "25 European languages (auto-detected): Bulgarian, Croatian, \
                    Czech, Danish, Dutch, English, Estonian, Finnish, French, \
                    German, Greek, Hungarian, Italian, Latvian, Lithuanian, \
                    Maltese, Polish, Portuguese, Romanian, Russian, Slovak, \
                    Slovenian, Spanish, Swedish, Ukrainian",
        auto_select_languages: &[
            "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt",
            "mt", "pl", "pt", "ro", "ru", "sk", "sl", "es", "sv", "uk",
        ],
    },
    // SenseVoiceSmall int8 — packaged for sherpa-onnx by k2-fsa. This covers
    // the Korean case reported in #2478 as well as Chinese, Cantonese, and
    // Japanese. The English locale intentionally keeps the smaller Parakeet
    // English default; SenseVoice remains available through BUZZ_STT_MODEL.
    SttModel {
        id: "sensevoice",
        dir_name: "sensevoice-small",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
             sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2",
        archive_subdir: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17",
        archive_sha256: Some("7d1efa2138a65b0b488df37f8b89e3d91a60676e416f515b952358d83dfd347e"),
        max_download_bytes: 250 * 1024 * 1024,
        model_files: &["model.int8.onnx", "tokens.txt"],
        family: SttFamily::SenseVoice,
        version: "1",
        license_text: STT_SENSEVOICE_LICENSE_TEXT,
        languages: "Chinese, Cantonese, English, Japanese, Korean (auto-detected)",
        auto_select_languages: &["zh", "yue", "ja", "ko"],
    },
];

/// The default STT model (English). Used when no override/locale applies.
fn default_stt_model() -> &'static SttModel {
    &STT_MODELS[0]
}

/// Look up a model by id (case-insensitive). `None` if unknown.
fn stt_model_by_id(id: &str) -> Option<&'static SttModel> {
    STT_MODELS.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Files that must be present for a model to be "ready": its model files plus
/// the Buzz-written model-specific license sidecar. This keeps the applicable
/// notice next to the bytes even when an upstream archive also ships LICENSE.
fn stt_expected_files(model: &SttModel) -> Vec<&'static str> {
    let mut files = model.model_files.to_vec();
    files.push(STT_LICENSE_FILE_NAME);
    files
}

/// Pick the STT model from an explicit override id and a best-effort locale.
///
/// Precedence (issue #2478 options 2 + 3):
///   1. `override_id` (from `BUZZ_STT_MODEL`) when it names a known model.
///   2. a locale covered by a registered model → that model.
///   3. otherwise the English default.
///
/// Pure and dependency-free so it is unit-testable without touching disk/env.
pub fn select_stt_model(override_id: Option<&str>, locale: Option<&str>) -> &'static SttModel {
    if let Some(id) = override_id {
        let id = id.trim();
        if !id.is_empty() {
            if let Some(model) = stt_model_by_id(id) {
                return model;
            }
            eprintln!(
                "buzz-desktop: BUZZ_STT_MODEL='{id}' is not a known STT model id — ignoring \
                 (valid ids: {})",
                STT_MODELS
                    .iter()
                    .map(|m| m.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if let Some(locale) = locale {
        // Take the primary language subtag: "de-DE"/"uk_UA" → "de"/"uk".
        let lang = locale
            .split(['-', '_', '.'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(model) = STT_MODELS
            .iter()
            .find(|model| model.auto_select_languages.contains(&lang.as_str()))
        {
            return model;
        }
    }
    default_stt_model()
}

/// Best-effort system locale from the environment (dependency-free).
///
/// Reads the standard POSIX locale variables in precedence order. Returns
/// `None` when unset or set to the neutral `C`/`POSIX` locale, in which case
/// selection falls back to the English default.
fn detect_locale() -> Option<String> {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() && value != "C" && value != "POSIX" {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Resolve the STT model to use for this process: `BUZZ_STT_MODEL` override,
/// else system-locale auto-select, else English default.
fn selected_stt_model() -> &'static SttModel {
    let override_id = std::env::var("BUZZ_STT_MODEL").ok();
    select_stt_model(override_id.as_deref(), detect_locale().as_deref())
}

/// CC-BY-4.0 §3(a)(1) attribution for Parakeet TDT-CTC 110M (English).
/// Covers all five §3(a)(1) bullets: creator, copyright notice, license
/// notice, warranty disclaimer reference, and URI to the source material.
const STT_EN_LICENSE_TEXT: &str = "\
NVIDIA Parakeet TDT-CTC 110M (English)
© NVIDIA Corporation.

Licensed under the Creative Commons Attribution 4.0 International License
(CC-BY-4.0). License text: https://creativecommons.org/licenses/by/4.0/

Original model: https://huggingface.co/nvidia/parakeet-tdt_ctc-110m
Converted to ONNX with int8 quantization by the sherpa-onnx project
(https://github.com/k2-fsa/sherpa-onnx); Buzz ships this conversion
unmodified.

Provided \"AS IS\", without warranty of any kind, express or implied. See the
license text for full warranty disclaimer.
";

/// CC-BY-4.0 §3(a)(1) attribution for Parakeet TDT 0.6B v3 (multilingual).
const STT_V3_LICENSE_TEXT: &str = "\
NVIDIA Parakeet TDT 0.6B v3 (multilingual)
© NVIDIA Corporation.

Licensed under the Creative Commons Attribution 4.0 International License
(CC-BY-4.0). License text: https://creativecommons.org/licenses/by/4.0/

Original model: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
Converted to ONNX with int8 quantization by the sherpa-onnx project
(https://github.com/k2-fsa/sherpa-onnx); Buzz ships this conversion
unmodified.

Provided \"AS IS\", without warranty of any kind, express or implied. See the
license text for full warranty disclaimer.
";

/// FunASR Model Open Source License 1.1 notice for SenseVoiceSmall.
const STT_SENSEVOICE_LICENSE_TEXT: &str = "\
SenseVoiceSmall
Copyright (C) 2023-2028 Alibaba Group. All rights reserved.

Licensed under the FunASR Model Open Source License Agreement, Version 1.1.
Full license: https://github.com/modelscope/FunASR/blob/main/MODEL_LICENSE

Original model: https://github.com/FunAudioLLM/SenseVoice
Converted to ONNX with int8 quantization by the sherpa-onnx project
(https://github.com/k2-fsa/sherpa-onnx); Buzz ships this conversion unmodified.
The upstream archive also includes its own LICENSE pointer.
";

// ── Pocket TTS model ──────────────────────────────────────────────────────────

/// Final directory name under `~/.buzz/models/`.
const TTS_MODEL_DIR_NAME: &str = "pocket-tts";

/// Attribution sidecar written next to the Pocket TTS model files.
const TTS_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";

/// All files that must be present for Pocket TTS to be considered ready.
const TTS_EXPECTED_FILES: &[&str] = &[
    "bundle.json",
    "bos_before_voice.npy",
    "flow_lm_main_int8.onnx",
    "flow_lm_flow_int8.onnx",
    "mimi_decoder_int8.onnx",
    "mimi_encoder.onnx",
    "text_conditioner.onnx",
    "tokenizer.model",
    "LICENSE",
    "reference_sample.wav",
    "anna.wav",
    "vera.wav",
    "fantine.wav",
    "charles.wav",
    "paul.wav",
    "eponine.wav",
    "azelma.wav",
    "george.wav",
    "jane.wav",
    "michael.wav",
    "eve.wav",
    TTS_LICENSE_FILE_NAME,
];

// ── Status types ──────────────────────────────────────────────────────────────

/// Download/readiness status for a single model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    NotDownloaded,
    Downloading { progress_percent: u8 },
    Ready,
    Error(String),
}

/// Combined status for all voice models (returned to the frontend).
///
/// `stt` is the speech-to-text model status (currently Parakeet TDT-CTC 110M;
/// historically Moonshine Tiny). The field name describes the role, not the
/// specific model, so future model swaps don't ripple into the API surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceModelStatus {
    pub stt: ModelStatus,
    pub tts: ModelStatus,
}

// ── Safe archive extraction ───────────────────────────────────────────────────

/// Extract a .tar.bz2 archive safely using Rust-native crates.
///
/// The `tar` crate rejects path traversal (absolute paths, `..` components)
/// by default in `unpack()`. We add an explicit pre-check as defense-in-depth.
fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<(), String> {
    use bzip2::read::BzDecoder;
    use std::fs::File;
    use tar::Archive;

    let file = File::open(archive_path).map_err(|e| format!("open archive: {e}"))?;
    let decoder = BzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    // Pre-validate: check all entries for path safety before extracting anything.
    // This is defense-in-depth — the tar crate also rejects traversal in unpack().
    {
        let file2 =
            File::open(archive_path).map_err(|e| format!("open archive for validation: {e}"))?;
        let decoder2 = BzDecoder::new(file2);
        let mut check_archive = Archive::new(decoder2);
        for entry in check_archive
            .entries()
            .map_err(|e| format!("read archive entries: {e}"))?
        {
            let entry = entry.map_err(|e| format!("archive entry: {e}"))?;
            let path = entry.path().map_err(|e| format!("entry path: {e}"))?;
            let path_str = path.to_string_lossy();

            // Reject absolute paths.
            if path.is_absolute() {
                return Err(format!("archive contains absolute path: {path_str}"));
            }
            // Reject path traversal.
            for component in path.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(format!("archive contains path traversal: {path_str}"));
                }
            }
            // Reject symlinks.
            if entry.header().entry_type().is_symlink()
                || entry.header().entry_type().is_hard_link()
            {
                return Err(format!("archive contains symlink/hardlink: {path_str}"));
            }
        }
    }

    // Safe to extract — all entries validated.
    archive
        .unpack(dest_dir)
        .map_err(|e| format!("extract archive: {e}"))?;

    Ok(())
}

// ── Hash verification ─────────────────────────────────────────────────────────

/// Compute SHA-256 hash of a file. Returns lowercase hex string.
async fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("read file for hash: {e}"))?;
    let hash = Sha256::digest(&bytes);
    Ok(hex::encode(hash))
}

// ── Shared HTTP helpers ───────────────────────────────────────────────────────

/// Send a GET request and return the response, or a descriptive error.
async fn fetch_url(
    client: &reqwest::Client,
    url: &str,
    label: &str,
) -> Result<reqwest::Response, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download {label} request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "download {label} HTTP {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("unknown"),
        ));
    }
    Ok(response)
}

/// Create (or recreate) a temp directory, removing any stale one first.
async fn fresh_temp_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|e| format!("remove stale temp dir: {e}"))?;
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| format!("create temp dir: {e}"))
}

/// Stream an HTTP response to a file with progress reporting and size limits.
///
/// Calls `progress_fn(bytes_downloaded, content_length)` after each chunk.
/// Returns the total number of bytes written.
async fn download_file<F>(
    response: reqwest::Response,
    dest: &Path,
    max_bytes: u64,
    label: &str,
    progress_fn: F,
) -> Result<u64, String>
where
    F: Fn(u64, Option<u64>),
{
    use tokio::io::AsyncWriteExt;

    let content_length = response.content_length();
    if let Some(total) = content_length {
        if total > max_bytes {
            return Err(format!(
                "download {label} too large: {total} bytes (max {max_bytes})"
            ));
        }
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("create {label}: {e}"))?;
    let mut downloaded: u64 = 0;
    let mut response = response;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download {label} stream error: {e}"))?
    {
        downloaded += chunk.len() as u64;
        if downloaded > max_bytes {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(format!(
                "download {label} exceeded max size: {downloaded} bytes (max {max_bytes})"
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write {label}: {e}"))?;
        progress_fn(downloaded, content_length);
    }

    file.flush()
        .await
        .map_err(|e| format!("flush {label}: {e}"))?;
    Ok(downloaded)
}

// ── ModelSlot ─────────────────────────────────────────────────────────────────

/// Per-model state + config. `ModelManager` owns two of these (stt, tts).
#[derive(Clone)]
struct ModelSlot {
    dir_name: &'static str,            // subdir under ~/.buzz/models/
    expected_files: Vec<&'static str>, // files required for "ready"
    version: &'static str,             // manifest version; increment to force re-download
    expected_size: fn(&str) -> Option<u64>,
    status: Arc<Mutex<ModelStatus>>,
    just_ready: Arc<AtomicBool>, // fires once when download completes
}

impl ModelSlot {
    fn new(
        dir_name: &'static str,
        expected_files: Vec<&'static str>,
        version: &'static str,
    ) -> Self {
        Self {
            dir_name,
            expected_files,
            version,
            expected_size: |_| None,
            status: Arc::new(Mutex::new(ModelStatus::NotDownloaded)),
            just_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with_expected_sizes(mut self, expected_size: fn(&str) -> Option<u64>) -> Self {
        self.expected_size = expected_size;
        self
    }

    fn model_dir(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.dir_name)
    }

    fn is_ready(&self, models_dir: &Path) -> bool {
        let dir = self.model_dir(models_dir);
        std::fs::read_to_string(dir.join(MANIFEST_FILENAME))
            .map(|v| v.trim() == self.version)
            .unwrap_or(false)
            && self.expected_files.iter().all(|filename| {
                let path = dir.join(filename);
                path.is_file()
                    && (self.expected_size)(filename)
                        .map(|expected| {
                            path.metadata()
                                .map(|metadata| metadata.len() == expected)
                                .unwrap_or(false)
                        })
                        .unwrap_or(true)
            })
    }

    fn dir_if_ready(&self, models_dir: &Path) -> Option<PathBuf> {
        self.is_ready(models_dir)
            .then(|| self.model_dir(models_dir))
    }

    fn status(&self) -> ModelStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn set_status(&self, s: ModelStatus) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
    fn take_ready(&self) -> bool {
        self.just_ready.swap(false, Ordering::AcqRel)
    }

    /// Recover or clean up the backup left by an interrupted atomic install.
    fn recover_interrupted_install(&self, models_dir: &Path) {
        let final_dir = self.model_dir(models_dir);
        let backup_dir = final_dir.with_extension("old");
        if !backup_dir.exists() {
            return;
        }
        if self.is_ready(models_dir) {
            if let Err(error) = std::fs::remove_dir_all(&backup_dir) {
                eprintln!(
                    "buzz-desktop: could not remove stale {} backup: {error}",
                    self.dir_name
                );
            }
            return;
        }
        if final_dir.exists() {
            if let Err(error) = std::fs::remove_dir_all(&final_dir) {
                eprintln!(
                    "buzz-desktop: could not remove incomplete {} install: {error}",
                    self.dir_name
                );
                return;
            }
        }
        if let Err(error) = std::fs::rename(&backup_dir, &final_dir) {
            eprintln!(
                "buzz-desktop: could not restore interrupted {} install: {error}",
                self.dir_name
            );
        }
    }

    /// Spawn a background download task if not already ready or downloading.
    fn start_download<F, Fut>(
        &self,
        models_dir: &Path,
        http_client: reqwest::Client,
        name: &'static str,
        download_fn: F,
    ) where
        F: FnOnce(reqwest::Client) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send,
    {
        if self.is_ready(models_dir) {
            self.set_status(ModelStatus::Ready);
            return;
        }
        {
            let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
            match *st {
                ModelStatus::Downloading { .. } | ModelStatus::Ready => return,
                _ => {}
            }
            *st = ModelStatus::Downloading {
                progress_percent: 0,
            };
        }
        let slot = self.clone();
        // Use tauri::async_runtime::spawn (not tokio::spawn) because this may
        // be called from the Tauri setup callback before the main Tokio runtime
        // is accessible on the current thread. Tauri's runtime is always available.
        tauri::async_runtime::spawn(async move {
            if let Err(e) = download_fn(http_client).await {
                eprintln!("buzz-desktop: {name} download failed: {e}");
                slot.set_status(ModelStatus::Error(e));
            }
        });
    }

    /// Verify files in `source_dir`, atomic-swap into final location, write manifest, signal ready.
    /// `temp_cleanup`: optional extra dir to remove (e.g. outer extraction dir for STT archive).
    async fn verify_and_install(
        &self,
        models_dir: &Path,
        source_dir: &Path,
        temp_cleanup: Option<&Path>,
    ) -> Result<(), String> {
        let missing: Vec<&str> = self
            .expected_files
            .iter()
            .filter(|&&f| !source_dir.join(f).is_file())
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "model verification failed — missing: {}",
                missing.join(", ")
            ));
        }

        std::fs::write(source_dir.join(MANIFEST_FILENAME), self.version)
            .map_err(|e| format!("write model manifest: {e}"))?;

        let final_dir = self.model_dir(models_dir);
        let backup_dir = final_dir.with_extension("old");

        if final_dir.exists() {
            if backup_dir.exists() {
                let _ = tokio::fs::remove_dir_all(&backup_dir).await;
            }
            tokio::fs::rename(&final_dir, &backup_dir)
                .await
                .map_err(|e| format!("backup old model: {e}"))?;
        }
        if let Err(e) = tokio::fs::rename(source_dir, &final_dir).await {
            if backup_dir.exists() {
                let _ = tokio::fs::rename(&backup_dir, &final_dir).await;
            }
            return Err(format!("install new model: {e}"));
        }

        let _ = tokio::fs::remove_dir_all(&backup_dir).await;
        if let Some(extra) = temp_cleanup {
            let _ = tokio::fs::remove_dir_all(extra).await;
        }

        self.set_status(ModelStatus::Ready);
        self.just_ready.store(true, Ordering::Release);
        Ok(())
    }
}

fn tts_expected_size(filename: &str) -> Option<u64> {
    april_model_info()
        .artifacts
        .iter()
        .find(|artifact| artifact.filename == filename)
        .map(|artifact| artifact.size_bytes)
        .or_else(|| {
            [TTS_LICENSE_ARTIFACT, TTS_REFERENCE_ARTIFACT]
                .iter()
                .find(|artifact| artifact.filename == filename)
                .map(|artifact| artifact.size_bytes)
        })
}

fn tts_model_slot() -> ModelSlot {
    ModelSlot::new(
        TTS_MODEL_DIR_NAME,
        TTS_EXPECTED_FILES.to_vec(),
        TTS_MODEL_VERSION,
    )
    .with_expected_sizes(tts_expected_size)
}

fn models_dir(nest_dir: PathBuf) -> PathBuf {
    nest_dir.join("models")
}

// ── ModelManager ──────────────────────────────────────────────────────────────

/// Manages download and location of STT/TTS model files.
///
/// Cheap to clone — all inner state is behind `Arc`.
#[derive(Clone)]
pub struct ModelManager {
    /// Model storage under the selected build's nest.
    models_dir: PathBuf,
    /// The STT model selected for this process (override / locale / default).
    stt_model: &'static SttModel,
    stt: ModelSlot,
    tts: ModelSlot,
}

impl ModelManager {
    /// Create a new `ModelManager` rooted in the selected build's nest.
    ///
    /// The STT model is resolved once here from `BUZZ_STT_MODEL`, then the
    /// system locale, then the English default (issue #2478).
    ///
    /// Returns `None` if the selected build's nest cannot be resolved.
    pub fn new() -> Option<Self> {
        let models_dir = models_dir(crate::managed_agents::nest_dir()?);
        let stt_model = selected_stt_model();
        eprintln!(
            "buzz-desktop: STT model '{}' selected ({})",
            stt_model.id, stt_model.languages
        );
        let manager = Self {
            models_dir,
            stt_model,
            stt: ModelSlot::new(
                stt_model.dir_name,
                stt_expected_files(stt_model),
                stt_model.version,
            ),
            tts: tts_model_slot(),
        };
        manager.stt.recover_interrupted_install(&manager.models_dir);
        manager.tts.recover_interrupted_install(&manager.models_dir);
        Some(manager)
    }

    // ── STT accessors ────────────────────────────────────────────────────────

    /// The sherpa-onnx model family of the selected STT model. The huddle STT
    /// pipeline uses this to configure the offline recognizer.
    pub fn stt_family(&self) -> SttFamily {
        self.stt_model.family
    }

    /// Path to the STT model directory, or `None` if not ready.
    pub fn stt_model_dir(&self) -> Option<PathBuf> {
        self.stt.dir_if_ready(&self.models_dir)
    }
    /// `true` if all STT files are present and the manifest version matches.
    pub fn is_stt_ready(&self) -> bool {
        self.stt.is_ready(&self.models_dir)
    }
    /// Current STT download status.
    pub fn stt_status(&self) -> ModelStatus {
        self.stt.status()
    }
    /// Returns `true` once when the STT model just became ready. Resets the flag.
    pub fn take_stt_ready(&self) -> bool {
        self.stt.take_ready()
    }

    // ── TTS accessors ─────────────────────────────────────────────────────────

    /// Path to the TTS model directory, or `None` if not ready.
    pub fn tts_model_dir(&self) -> Option<PathBuf> {
        self.tts.dir_if_ready(&self.models_dir)
    }
    /// `true` if all TTS files are present and the manifest version matches.
    pub fn is_tts_ready(&self) -> bool {
        self.tts.is_ready(&self.models_dir)
    }
    /// Current TTS download status.
    pub fn tts_status(&self) -> ModelStatus {
        self.tts.status()
    }
    /// Returns `true` once when TTS just became ready. Resets the flag.
    pub fn take_tts_ready(&self) -> bool {
        self.tts.take_ready()
    }

    // ── Download triggers ─────────────────────────────────────────────────────

    /// Start a background STT model download. No-op if already ready or downloading.
    ///
    /// Also schedules a best-effort cleanup of the legacy Moonshine model
    /// directory — but **only when the new STT model is already on disk and
    /// Ready**. This covers the "fast-path" upgrade scenario (new model
    /// installed by a previous build, `download_stt_model` short-circuits, the
    /// post-install cleanup never runs). For users mid-migration (old model
    /// present, new model still downloading) we keep the old files until the
    /// Parakeet install finishes, avoiding unnecessary data loss if the
    /// ~100 MB download fails. The post-install path inside
    /// `download_stt_model` handles cleanup once the new install reaches Ready.
    pub fn start_stt_download(&self, http_client: reqwest::Client) {
        let manager = self.clone();
        self.stt.start_download(
            &self.models_dir,
            http_client,
            "stt",
            move |client| async move { manager.download_stt_model(client).await },
        );
        if self.stt.is_ready(&self.models_dir) {
            // Detached cleanup task — must not block startup. Gated above on
            // the new model being Ready, so a mid-migration user keeps their
            // existing moonshine-tiny files until Parakeet install completes.
            let models_dir = self.models_dir.clone();
            tauri::async_runtime::spawn(async move {
                cleanup_legacy_moonshine_dir(&models_dir).await;
            });
        }
    }

    /// Start a background Pocket TTS download. No-op if already ready or downloading.
    pub fn start_tts_download(&self, http_client: reqwest::Client) {
        if let Err(error) = voice_upgrade::install_vctk_presets_into_v4_model(&self.models_dir) {
            eprintln!("buzz-desktop: could not upgrade existing Pocket voices in place: {error}");
        }
        let manager = self.clone();
        self.tts.start_download(
            &self.models_dir,
            http_client,
            "tts",
            move |client| async move { manager.download_tts_model(client).await },
        );
    }

    // ── Private download implementations ─────────────────────────────────────

    /// Download, extract, and verify the STT model archive.
    async fn download_stt_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;

        // Temp filenames derive from the final directory name to avoid colliding
        // with leftovers from any previous STT model (e.g. moonshine-tiny.*).
        let model = self.stt_model;
        let archive_path = self.models_dir.join(format!("{}.tar.bz2", model.dir_name));
        let temp_dir = self.models_dir.join(format!("{}.tmp", model.dir_name));

        eprintln!(
            "buzz-desktop: downloading STT model '{}' from {}",
            model.id, model.download_url
        );
        let response = fetch_url(&http_client, model.download_url, "stt archive").await?;

        let slot = self.stt.clone();
        let bytes = download_file(
            response,
            &archive_path,
            model.max_download_bytes,
            "stt archive",
            |downloaded, content_length| {
                if let Some(pct) =
                    content_length.and_then(|total| (downloaded * 89).checked_div(total))
                {
                    slot.set_status(ModelStatus::Downloading {
                        progress_percent: pct.min(89) as u8,
                    });
                }
            },
        )
        .await?;
        eprintln!("buzz-desktop: downloaded {bytes} bytes, wrote to disk");

        // Verify archive integrity before extraction. Models with a pinned
        // hash are content-verified; a `None` hash relies on the size cap
        // (already enforced above), safe extraction, and the expected-files
        // check in `verify_and_install`.
        match model.archive_sha256 {
            Some(expected) => {
                let hash = sha256_file(&archive_path).await?;
                if hash != expected {
                    let _ = tokio::fs::remove_file(&archive_path).await;
                    return Err(format!(
                        "STT archive integrity check failed: expected {expected}, got {hash}"
                    ));
                }
            }
            None => {
                eprintln!(
                    "buzz-desktop: STT model '{}' has no pinned SHA-256 — \
                     skipping content hash (size cap + safe extraction still enforced)",
                    model.id
                );
            }
        }

        self.stt.set_status(ModelStatus::Downloading {
            progress_percent: 90,
        });
        fresh_temp_dir(&temp_dir).await?;

        eprintln!("buzz-desktop: extracting STT archive…");
        let (ap, td) = (archive_path.clone(), temp_dir.clone());
        tokio::task::spawn_blocking(move || extract_archive(&ap, &td))
            .await
            .map_err(|e| format!("tar task panicked: {e}"))??;

        let extracted_subdir = temp_dir.join(model.archive_subdir);
        if !extracted_subdir.is_dir() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!(
                "expected subdir '{}' not found after extraction",
                model.archive_subdir
            ));
        }

        // Write the model-specific license sidecar before the atomic install,
        // so it lands in the final directory as part of the same rename. Some
        // upstream archives also include their own LICENSE.
        let license_path = extracted_subdir.join(STT_LICENSE_FILE_NAME);
        if let Err(e) = tokio::fs::write(&license_path, model.license_text).await {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(format!("write model license sidecar: {e}"));
        }

        // verify_and_install takes the subdir (actual model files); temp_cleanup removes outer dir.
        if let Err(e) = self
            .stt
            .verify_and_install(&self.models_dir, &extracted_subdir, Some(&temp_dir))
            .await
        {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(e);
        }
        let _ = tokio::fs::remove_file(&archive_path).await;

        // Best-effort cleanup of the previous default STT model dir (Moonshine
        // Tiny, ~70 MB). Runs only after the new install reaches Ready, so a
        // failed download never removes the previous on-disk model during
        // migration. The same cleanup also runs from `start_stt_download` to
        // cover users who already have the new model installed.
        cleanup_legacy_moonshine_dir(&self.models_dir).await;

        eprintln!(
            "buzz-desktop: STT model ready at {}",
            self.stt.model_dir(&self.models_dir).display()
        );
        Ok(())
    }

    /// Download and verify the Pocket TTS model files from HuggingFace.
    ///
    /// Downloads files into `~/.buzz/models/pocket-tts/`:
    ///   - five ONNX sessions selected by the April INT8 bundle
    ///   - bundle metadata, SentencePiece tokenizer, and learned voice BOS
    ///   - upstream `LICENSE` plus Buzz's `MODEL_LICENSE.txt` attribution sidecar
    ///   - `reference_sample.wav` plus the embedded official VCTK presets
    ///
    /// Files are written to a temp directory first, then moved atomically.
    async fn download_tts_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;

        let temp_dir = self.models_dir.join("pocket-tts.tmp");
        fresh_temp_dir(&temp_dir).await?;

        let mut downloads: Vec<(String, PocketModelArtifact)> = april_model_info()
            .artifacts
            .iter()
            .copied()
            .map(|artifact| (pocket_artifact_url(artifact.filename), artifact))
            .collect();
        downloads.push((pocket_license_url(), TTS_LICENSE_ARTIFACT));
        downloads.push((POCKET_REFERENCE_WAV_URL.to_string(), TTS_REFERENCE_ARTIFACT));
        let total_files = downloads.len() as u32;

        for (i, (url, artifact)) in downloads.iter().enumerate() {
            let filename = artifact.filename;
            eprintln!("buzz-desktop: downloading Pocket TTS {filename} from {url}");

            let response = fetch_url(&http_client, url, filename)
                .await
                .inspect_err(|_| {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                })?;

            let dest = temp_dir.join(filename);
            let slot = self.tts.clone();
            let file_index = i as u32;
            let bytes = download_file(
                response,
                &dest,
                MAX_TTS_FILE_BYTES,
                filename,
                |downloaded, content_length| {
                    if let Some(total) = content_length {
                        if total > 0 {
                            let file_frac = downloaded as f64 / total as f64;
                            let base = (file_index as f64 / total_files as f64) * 89.0;
                            let span = 89.0 / total_files as f64;
                            let pct = (base + span * file_frac).min(89.0) as u8;
                            slot.set_status(ModelStatus::Downloading {
                                progress_percent: pct,
                            });
                        }
                    }
                },
            )
            .await
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&temp_dir);
            })?;
            eprintln!("buzz-desktop: downloaded {bytes} bytes ({filename}), wrote to disk");

            if bytes != artifact.size_bytes {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!(
                    "Pocket TTS {filename} size check failed: expected {} bytes, got {bytes}",
                    artifact.size_bytes
                ));
            }
            let actual = sha256_file(&dest).await?;
            if actual != artifact.sha256 {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!(
                    "Pocket TTS {filename} integrity check failed: expected {}, got {actual}",
                    artifact.sha256
                ));
            }

            // Ensure progress reflects file completion even without content-length.
            let pct = (((i as u32 + 1) * 89) / total_files).min(89) as u8;
            self.tts.set_status(ModelStatus::Downloading {
                progress_percent: pct,
            });
        }

        tokio::fs::write(
            temp_dir.join(TTS_LICENSE_FILE_NAME),
            voice_upgrade::TTS_LICENSE_TEXT,
        )
        .await
        .map_err(|e| format!("write TTS model license sidecar: {e}"))?;
        for voice in POCKET_VOICES {
            let Some(bytes) = voice.bytes else {
                continue;
            };
            tokio::fs::write(temp_dir.join(voice.reference_file), bytes)
                .await
                .map_err(|e| format!("install bundled {} voice: {e}", voice.display_name))?;
        }

        self.tts.set_status(ModelStatus::Downloading {
            progress_percent: 90,
        });

        if let Err(e) = self
            .tts
            .verify_and_install(&self.models_dir, &temp_dir, None)
            .await
        {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(e);
        }

        eprintln!(
            "buzz-desktop: Pocket TTS model ready at {}",
            self.tts.model_dir(&self.models_dir).display()
        );
        Ok(())
    }
}

// ── Process-global singleton ──────────────────────────────────────────────────

static GLOBAL_MODEL_MANAGER: OnceLock<Option<ModelManager>> = OnceLock::new();

/// Return a reference to the process-global `ModelManager`.
pub fn global_model_manager() -> Option<&'static ModelManager> {
    GLOBAL_MODEL_MANAGER.get_or_init(ModelManager::new).as_ref()
}

// ── Standalone helpers ────────────────────────────────────────────────────────

/// Path to the STT model directory, or `None` if not ready.
pub fn stt_model_dir() -> Option<PathBuf> {
    global_model_manager()?.stt_model_dir()
}

/// sherpa-onnx model family of the selected STT model (English default when the
/// manager is unavailable). The huddle STT pipeline uses this to configure the
/// offline recognizer for the right model family.
pub fn stt_model_family() -> SttFamily {
    global_model_manager()
        .map(|m| m.stt_family())
        .unwrap_or(SttFamily::NemoCtc)
}

/// `true` if all expected STT model files are present on disk.
pub fn is_stt_ready() -> bool {
    global_model_manager()
        .map(|m| m.is_stt_ready())
        .unwrap_or(false)
}

/// Best-effort cleanup of the legacy Moonshine STT model directory.
///
/// Removes `~/.buzz/models/moonshine-tiny/` if present (~70 MB on disk).
/// Idempotent — no-op if the directory is absent. Errors are logged and
/// swallowed; the leftover is harmless and the user can remove it manually.
///
/// This is intentionally a free function rather than a method: it has no
/// dependency on `ModelManager` state, runs from both pre- and post-install
/// code paths, and the call site is meant to be easy to delete in a future
/// release once we're confident no users are still on the old model dir.
async fn cleanup_legacy_moonshine_dir(models_dir: &Path) {
    let legacy = models_dir.join("moonshine-tiny");
    if !legacy.exists() {
        return;
    }
    match tokio::fs::remove_dir_all(&legacy).await {
        Ok(()) => eprintln!(
            "buzz-desktop: removed legacy STT model dir {}",
            legacy.display()
        ),
        Err(e) => eprintln!(
            "buzz-desktop: could not remove legacy STT model dir {}: {e} \
             (harmless — remove manually to reclaim disk space)",
            legacy.display()
        ),
    }
}

/// Path to the TTS model directory, or `None` if not ready.
pub fn tts_model_dir() -> Option<PathBuf> {
    global_model_manager()?.tts_model_dir()
}

/// `true` if all expected TTS model files are present on disk.
pub fn is_tts_ready() -> bool {
    global_model_manager()
        .map(|m| m.is_tts_ready())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
