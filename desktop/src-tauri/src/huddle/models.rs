//! Model download manager for STT (Parakeet family) and TTS (Pocket TTS) models.
//!
//! The STT model is selectable via the `STT_MODELS` registry (issue #2478):
//! the English Parakeet TDT-CTC 110M default, or the multilingual Parakeet TDT
//! 0.6B v3, chosen by `BUZZ_STT_MODEL` / system locale in `selected_stt_model`.
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

/// HuggingFace base URL for the sherpa-onnx Pocket TTS fp32 repackage.
///
/// Pinned to commit 96d1e53ce3311ca6c2c6a35e2062d36b4cec6fa3
/// (2026-02-10) for reproducible downloads.
///
/// fp32 (not int8): a direct same-runtime A/B (k2-fsa/sherpa-onnx#3172)
/// found the ONNX int8 quantization audibly degraded Pocket TTS output and
/// that fp32 "significantly improved quality even at 1 step". The runtime
/// bundle grows from ~189 MB to ~473 MB; encoder, text conditioner, both
/// JSON tables, and LICENSE are byte-identical between the two repos — only
/// the three quantized sessions (lm_main, lm_flow, decoder) change.
const POCKET_HF_BASE: &str =
    "https://huggingface.co/csukuangfj2/sherpa-onnx-pocket-tts-2026-01-26/resolve/96d1e53ce3311ca6c2c6a35e2062d36b4cec6fa3";

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

/// SHA-256 hashes for individual Pocket TTS model files.
/// Computed from known-good pinned downloads. Update when upgrading model versions.
#[rustfmt::skip]
const TTS_FILE_HASHES: &[(&str, &str)] = &[
    ("decoder.onnx",          "f267880fde6c58b17b0a8f3647eaf8dcfad321f833f32d583ebc2fb2d1a15f10"),
    ("encoder.onnx",          "e8f2f6d301ffb96e398b138a7dc6d3038622d236044636b73d920bab85890260"),
    ("lm_flow.onnx",          "79c013a554a54e63319c33c0cc8830cbbedc9b7e448ae7e26f7923ae11f9873e"),
    ("lm_main.onnx",          "255d1a9263c5abdf36034abfc19c11d21cc5f40f0f87d8361288e972cbd5c578"),
    ("text_conditioner.onnx", "0b84e837d7bfaf2c896627b03e3f080320309f37f4fc7df7698c644f7ba5e6b1"),
    ("vocab.json",            "6fb646346cf931016f70c4921aab0900ce7a304b893cb02135c74e294abfea01"),
    ("token_scores.json",     "5be2f278caf9b9800741f0fd82bff677f4943ec764c356f907213434b622d958"),
    ("LICENSE",               "fe7b4ce83b8381cc5b216bbb4af73c570688d1b819c73bbaed8ca401f4677cd6"),
    ("reference_sample.wav",  "a35b0468382218e9f37a9a7494d1e4b74deaf18d7ced22265b4e325bb55c183f"),
];

// ── Model versioning ──────────────────────────────────────────────────────────
//
// A version manifest is written alongside model files after successful download.
// If the on-disk manifest doesn't match the compiled-in version, the model is
// considered stale and re-downloaded. Increment when upgrading model files.

// STT manifest versions are per model — see `SttModel::version` in the
// `STT_MODELS` registry below.

/// Model manifest version for Pocket TTS. Increment when upgrading model files.
/// Bumped "1" → "2" when the bundled reference voice changed from KevinAHM's
/// anonymous 16 kHz sample to Mary (VCTK p333, 32 kHz, ai-coustics-enhanced)
/// from kyutai/tts-voices. The hash mismatch on `reference_sample.wav` would
/// fail readiness on its own, but the manifest bump makes the re-download
/// reason explicit and skips the failing-then-re-fetching transient state.
/// Bumped "2" → "3" for the int8 → fp32 model swap (see `POCKET_HF_BASE`):
/// existing int8 installs must re-download the suffixless fp32 sessions.
const TTS_MODEL_VERSION: &str = "3";

/// Filename for the version manifest written alongside model files.
const MANIFEST_FILENAME: &str = ".buzz-model-manifest";

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum expected Pocket TTS file size (400 MB per file — largest is
/// `lm_main.onnx` at ~303 MB fp32).
const MAX_TTS_FILE_BYTES: u64 = 400 * 1024 * 1024;

// ── STT model registry (multilingual — issue #2478) ───────────────────────────
//
// Historically the huddle STT model was hard-pinned to an English-only build,
// so non-English speech transcribed as garbage (issue #2478). The registry
// makes the model selectable: the English default stays the default, and a
// multilingual model is picked automatically for non-English locales (or
// forced with the `BUZZ_STT_MODEL` env override). Adding a model is pure data
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
    /// CC-BY-4.0 §3(a)(1) attribution written next to the model bytes.
    pub license_text: &'static str,
    /// Human-readable language coverage (About dialog / logs).
    pub languages: &'static str,
    /// `true` when multilingual — used by the non-English locale auto-select.
    pub multilingual: bool,
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
        multilingual: false,
    },
    // NVIDIA Parakeet TDT 0.6B v3 (multilingual, int8) — packaged for
    // sherpa-onnx by k2-fsa. Transducer family (encoder/decoder/joiner). Auto
    // language-ID + punctuation across 25 European languages. This is the
    // multilingual default for non-English locales (issue #2478).
    //
    // Coverage note: Parakeet v3 covers European languages only. CJK/Korean
    // (the concrete case in #2478) is not covered by this model; the registry
    // makes adding a CJK model (e.g. SenseVoice-Small) a follow-up — one data
    // entry, no engine change beyond a family already handled here.
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
        multilingual: true,
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
/// the Buzz-written attribution sidecar (the upstream archives ship no LICENSE,
/// so readiness requires the local CC-BY-4.0 notice to travel with the bytes).
fn stt_expected_files(model: &SttModel) -> Vec<&'static str> {
    let mut files = model.model_files.to_vec();
    files.push(STT_LICENSE_FILE_NAME);
    files
}

/// Pick the STT model from an explicit override id and a best-effort locale.
///
/// Precedence (issue #2478 options 2 + 3):
///   1. `override_id` (from `BUZZ_STT_MODEL`) when it names a known model.
///   2. a non-English locale → the multilingual model.
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
        if !lang.is_empty() && lang != "en" {
            if let Some(model) = STT_MODELS.iter().find(|m| m.multilingual) {
                return model;
            }
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

// ── Pocket TTS model ──────────────────────────────────────────────────────────

/// Final directory name under `~/.buzz/models/`.
const TTS_MODEL_DIR_NAME: &str = "pocket-tts";

/// Attribution sidecar written next to the Pocket TTS model files.
const TTS_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";

/// CC-BY-4.0 §3(a)(1) attribution block for Pocket TTS, its ONNX packaging,
/// and the bundled reference voice WAV.
const TTS_LICENSE_TEXT: &str = "\
Pocket TTS
© Kyutai.

Licensed under the Creative Commons Attribution 4.0 International License
(CC-BY-4.0). License text: https://creativecommons.org/licenses/by/4.0/

Original model by Kyutai: https://huggingface.co/kyutai/pocket-tts
Paper: Charles, Roebel, et al., Pocket TTS (arXiv:2509.06926).
Mimi neural codec by Kyutai is bundled as part of the model.

ONNX export by KevinAHM: https://huggingface.co/KevinAHM/pocket-tts-onnx
Sherpa-onnx repackage by csukuangfj / k2-fsa:
https://huggingface.co/csukuangfj2/sherpa-onnx-pocket-tts-2026-01-26

Bundled reference voice (reference_sample.wav):
\"Mary (f, conversation)\" preset from the Kyutai TTS demo voice catalogue
(https://kyutai.org/tts), distributed via
https://huggingface.co/kyutai/tts-voices as `vctk/p333_023_enhanced.wav`.
Original recording from the Voice Cloning Toolkit (VCTK) corpus, speaker p333:
https://datashare.ed.ac.uk/handle/10283/3443 (CC-BY-4.0).
Recording enhancement (denoise/dereverb) by ai-coustics:
https://ai-coustics.com/

Buzz ships all ONNX/model artifacts and the reference voice WAV unmodified,
renamed only by placement in the local model directory.

Provided \"AS IS\", without warranty of any kind, express or implied. See the
license text for full warranty disclaimer.
";

/// All files that must be present for Pocket TTS to be considered ready.
const TTS_EXPECTED_FILES: &[&str] = &[
    "decoder.onnx",
    "encoder.onnx",
    "lm_flow.onnx",
    "lm_main.onnx",
    "text_conditioner.onnx",
    "vocab.json",
    "token_scores.json",
    "LICENSE",
    "reference_sample.wav",
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
            status: Arc::new(Mutex::new(ModelStatus::NotDownloaded)),
            just_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    fn model_dir(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.dir_name)
    }

    fn is_ready(&self, models_dir: &Path) -> bool {
        let dir = self.model_dir(models_dir);
        std::fs::read_to_string(dir.join(MANIFEST_FILENAME))
            .map(|v| v.trim() == self.version)
            .unwrap_or(false)
            && self.expected_files.iter().all(|f| dir.join(f).is_file())
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

        std::fs::write(final_dir.join(MANIFEST_FILENAME), self.version)
            .map_err(|e| format!("write model manifest: {e}"))?;
        let _ = tokio::fs::remove_dir_all(&backup_dir).await;
        if let Some(extra) = temp_cleanup {
            let _ = tokio::fs::remove_dir_all(extra).await;
        }

        self.set_status(ModelStatus::Ready);
        self.just_ready.store(true, Ordering::Release);
        Ok(())
    }
}

// ── ModelManager ──────────────────────────────────────────────────────────────

/// Manages download and location of STT/TTS model files.
///
/// Cheap to clone — all inner state is behind `Arc`.
#[derive(Clone)]
pub struct ModelManager {
    /// `~/.buzz/models/`
    models_dir: PathBuf,
    /// The STT model selected for this process (override / locale / default).
    stt_model: &'static SttModel,
    stt: ModelSlot,
    tts: ModelSlot,
}

impl ModelManager {
    /// Create a new `ModelManager` rooted at `~/.buzz/models/`.
    ///
    /// The STT model is resolved once here from `BUZZ_STT_MODEL`, then the
    /// system locale, then the English default (issue #2478).
    ///
    /// Returns `None` if the home directory cannot be resolved.
    pub fn new() -> Option<Self> {
        let models_dir = dirs::home_dir()?.join(".buzz").join("models");
        let stt_model = selected_stt_model();
        eprintln!(
            "buzz-desktop: STT model '{}' selected ({})",
            stt_model.id, stt_model.languages
        );
        Some(Self {
            models_dir,
            stt_model,
            stt: ModelSlot::new(
                stt_model.dir_name,
                stt_expected_files(stt_model),
                stt_model.version,
            ),
            tts: ModelSlot::new(
                TTS_MODEL_DIR_NAME,
                TTS_EXPECTED_FILES.to_vec(),
                TTS_MODEL_VERSION,
            ),
        })
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

    /// Start a background Pocket TTS download (~189 MB). No-op if already ready or downloading.
    pub fn start_tts_download(&self, http_client: reqwest::Client) {
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

        // Write the CC-BY-4.0 attribution sidecar before the atomic install,
        // so it lands in the final model dir as part of the same rename. The
        // upstream tarball ships no LICENSE/NOTICE, so we provide it ourselves
        // per §3(a)(1) (license must travel with Shared material).
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
    ///   - five ONNX sessions (Pocket TTS + Mimi codec)
    ///   - `vocab.json` / `token_scores.json` for sherpa-onnx text conditioning
    ///   - upstream `LICENSE` plus Buzz's `MODEL_LICENSE.txt` attribution sidecar
    ///   - `reference_sample.wav` as the bundled default voice
    ///
    /// Files are written to a temp directory first, then moved atomically.
    async fn download_tts_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;

        let temp_dir = self.models_dir.join("pocket-tts.tmp");
        fresh_temp_dir(&temp_dir).await?;

        let model_files = [
            "decoder.onnx",
            "encoder.onnx",
            "lm_flow.onnx",
            "lm_main.onnx",
            "text_conditioner.onnx",
            "vocab.json",
            "token_scores.json",
            "LICENSE",
        ];
        let mut downloads: Vec<(String, &'static str)> = model_files
            .iter()
            .map(|filename| (format!("{POCKET_HF_BASE}/{filename}"), *filename))
            .collect();
        downloads.push((POCKET_REFERENCE_WAV_URL.to_string(), "reference_sample.wav"));
        let total_files = downloads.len() as u32;

        for (i, (url, filename)) in downloads.iter().enumerate() {
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

            let expected = TTS_FILE_HASHES
                .iter()
                .find(|(n, _)| *n == *filename)
                .map(|(_, hash)| *hash)
                .ok_or_else(|| format!("missing expected hash for Pocket TTS file: {filename}"))?;
            let actual = sha256_file(&dest).await?;
            if actual != expected {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!(
                    "Pocket TTS {filename} integrity check failed: expected {expected}, got {actual}"
                ));
            }

            // Ensure progress reflects file completion even without content-length.
            let pct = (((i as u32 + 1) * 89) / total_files).min(89) as u8;
            self.tts.set_status(ModelStatus::Downloading {
                progress_percent: pct,
            });
        }

        tokio::fs::write(temp_dir.join(TTS_LICENSE_FILE_NAME), TTS_LICENSE_TEXT)
            .await
            .map_err(|e| format!("write TTS model license sidecar: {e}"))?;

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
mod tests {
    use super::*;

    #[test]
    fn tts_readiness_requires_license_sidecar() {
        let temp = tempfile::tempdir().expect("tempdir");
        let slot = ModelSlot::new(
            TTS_MODEL_DIR_NAME,
            TTS_EXPECTED_FILES.to_vec(),
            TTS_MODEL_VERSION,
        );
        let model_dir = temp.path().join(TTS_MODEL_DIR_NAME);
        std::fs::create_dir_all(&model_dir).expect("create model dir");

        for file in TTS_EXPECTED_FILES {
            std::fs::write(model_dir.join(file), b"test").expect("write expected file");
        }
        std::fs::write(model_dir.join(MANIFEST_FILENAME), TTS_MODEL_VERSION).expect("manifest");

        assert!(slot.is_ready(temp.path()));

        std::fs::remove_file(model_dir.join(TTS_LICENSE_FILE_NAME)).expect("remove sidecar");
        assert!(!slot.is_ready(temp.path()));
    }

    // ── STT model selection (issue #2478) ─────────────────────────────────────

    #[test]
    fn defaults_to_english_without_override_or_locale() {
        assert_eq!(select_stt_model(None, None).id, "parakeet-en");
        assert_eq!(select_stt_model(None, Some("en-US")).id, "parakeet-en");
        assert_eq!(select_stt_model(None, Some("en")).id, "parakeet-en");
        // Neutral C/POSIX locales are dropped by detect_locale, but pass the raw
        // primary subtag here to prove "en" specifically stays on English.
        assert!(!select_stt_model(None, None).multilingual);
    }

    #[test]
    fn non_english_locale_selects_multilingual_model() {
        for locale in ["de-DE", "uk_UA", "fr", "es-ES", "pl_PL.UTF-8"] {
            let model = select_stt_model(None, Some(locale));
            assert!(model.multilingual, "locale {locale} should be multilingual");
            assert_eq!(model.id, "parakeet-v3", "locale {locale}");
        }
    }

    #[test]
    fn explicit_override_wins_over_locale() {
        // Force multilingual even on an English locale.
        assert_eq!(
            select_stt_model(Some("parakeet-v3"), Some("en-US")).id,
            "parakeet-v3"
        );
        // Force English even on a non-English locale.
        assert_eq!(
            select_stt_model(Some("parakeet-en"), Some("de-DE")).id,
            "parakeet-en"
        );
        // Override id is case-insensitive.
        assert_eq!(
            select_stt_model(Some("PARAKEET-V3"), None).id,
            "parakeet-v3"
        );
    }

    #[test]
    fn unknown_or_empty_override_falls_back() {
        // Unknown override id → fall through to locale, then default.
        assert_eq!(
            select_stt_model(Some("does-not-exist"), Some("en-US")).id,
            "parakeet-en"
        );
        assert_eq!(
            select_stt_model(Some("does-not-exist"), Some("fr-FR")).id,
            "parakeet-v3"
        );
        // Empty / whitespace override is ignored.
        assert_eq!(select_stt_model(Some("   "), None).id, "parakeet-en");
    }

    #[test]
    fn registry_invariants_hold() {
        assert!(!STT_MODELS.is_empty());
        assert_eq!(default_stt_model().id, "parakeet-en");
        // The English default must always be integrity-pinned.
        assert!(
            default_stt_model().archive_sha256.is_some(),
            "English default must ship a pinned SHA-256"
        );
        // At least one multilingual model exists (covers #2478).
        assert!(STT_MODELS.iter().any(|m| m.multilingual));
        // Ids are unique (case-insensitive); every model declares files + a cap.
        for (i, model) in STT_MODELS.iter().enumerate() {
            assert!(!model.model_files.is_empty(), "{} has no files", model.id);
            assert!(model.max_download_bytes > 0, "{} has no size cap", model.id);
            for other in &STT_MODELS[i + 1..] {
                assert!(
                    !model.id.eq_ignore_ascii_case(other.id),
                    "duplicate model id {}",
                    model.id
                );
            }
        }
    }

    #[test]
    fn expected_files_always_include_license_sidecar() {
        for model in STT_MODELS {
            let files = stt_expected_files(model);
            assert!(
                files.contains(&STT_LICENSE_FILE_NAME),
                "{} missing license sidecar in expected files",
                model.id
            );
            for f in model.model_files {
                assert!(files.contains(f), "{} missing {f}", model.id);
            }
        }
    }

    #[test]
    fn readiness_uses_per_model_expected_files() {
        // A transducer model needs encoder/decoder/joiner — the single-file
        // check must not pass until all three plus tokens + license exist.
        let v3 = stt_model_by_id("parakeet-v3").expect("v3 registered");
        let temp = tempfile::tempdir().expect("tempdir");
        let slot = ModelSlot::new(v3.dir_name, stt_expected_files(v3), v3.version);
        let dir = temp.path().join(v3.dir_name);
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join(MANIFEST_FILENAME), v3.version).expect("manifest");

        // Only some files present → not ready.
        std::fs::write(dir.join("encoder.int8.onnx"), b"x").expect("write");
        std::fs::write(dir.join("tokens.txt"), b"x").expect("write");
        assert!(!slot.is_ready(temp.path()));

        // All expected files present → ready.
        for f in stt_expected_files(v3) {
            std::fs::write(dir.join(f), b"x").expect("write");
        }
        assert!(slot.is_ready(temp.path()));
    }
}
