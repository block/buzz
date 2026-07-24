//! Model download manager for STT (Parakeet TDT-CTC 110M) and TTS (Pocket TTS) models.
//!
//! Mental model:
//!   app launch → start_stt_download (background) → ~/.buzz/models/parakeet-tdt-ctc-110m-en/
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
// All model artifacts are verified against pinned SHA-256 hashes before
// installation. This is defense-in-depth: HTTPS protects the transport,
// hashes protect the content.
//
// To recompute hashes: download each file, run `shasum -a 256 <file>`, and
// update the corresponding constant.

/// SHA-256 hash of the STT archive
/// (sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8.tar.bz2).
/// Computed from a known-good download. Update when upgrading model versions.
const STT_ARCHIVE_SHA256: &str = "17f945007b52ccd8b7200ffc7c5652e9e8e961dfdf479cefcabd06cf5703630b";

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

/// Model manifest version for the STT model. Increment when upgrading model files.
/// Bumped from "1" → "2" alongside the migration from Moonshine Tiny to
/// Parakeet TDT-CTC 110M — the model directory name also changed, so this
/// is technically belt-and-suspenders, but it keeps the manifest semantics
/// honest (each version tag identifies one specific set of model bytes).
const STT_MODEL_VERSION: &str = "2";

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

/// Maximum expected STT archive size (200 MB — actual is ~100 MB).
const MAX_STT_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Maximum expected Pocket TTS file size (400 MB per file — largest is
/// `lm_main.onnx` at ~303 MB fp32).
const MAX_TTS_FILE_BYTES: u64 = 400 * 1024 * 1024;

/// NVIDIA Parakeet TDT-CTC 110M (English, int8) — packaged for sherpa-onnx by
/// k2-fsa. Single ONNX file (CTC head) + tokens.txt. Avg WER ~7.5% across
/// the OpenASR-style benchmarks; ~half the WER of Moonshine Tiny at ~2× the
/// disk footprint. CTC blank-token decoding eliminates the silence/cut-audio
/// hallucination class that hurts encoder-decoder models on noisy huddle audio.
/// License: CC-BY-4.0 (attribution required — see About dialog).
const STT_DOWNLOAD_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/\
     sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8.tar.bz2";

/// Subdirectory name produced by `tar xjf` on the archive.
const STT_ARCHIVE_SUBDIR: &str = "sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8";

/// Final directory name under `~/.buzz/models/`.
const STT_MODEL_DIR_NAME: &str = "parakeet-tdt-ctc-110m-en";

/// All files that must be present for the model to be considered ready.
///
/// Includes the attribution sidecar written by Buzz during install. The
/// upstream archive does not ship a license file, so readiness should require
/// the local CC-BY-4.0 attribution to travel with the cached model bytes.
const STT_EXPECTED_FILES: &[&str] = &["model.int8.onnx", "tokens.txt", STT_LICENSE_FILE_NAME];

/// CC-BY-4.0 §3(a)(1) attribution block written next to the STT model files
/// after install. Travels with the bytes — if a user copies the model
/// directory, the attribution comes with it. Mirrored in About/Credits.
///
/// Covers all five §3(a)(1) bullets: creator, copyright notice, license
/// notice, warranty disclaimer reference, and URI to the source material.
const STT_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";
const STT_LICENSE_TEXT: &str = "\
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
    dir_name: &'static str,                  // subdir under ~/.buzz/models/
    expected_files: &'static [&'static str], // files required for "ready"
    version: &'static str,                   // manifest version; increment to force re-download
    status: Arc<Mutex<ModelStatus>>,
    just_ready: Arc<AtomicBool>, // fires once when download completes
}

impl ModelSlot {
    fn new(
        dir_name: &'static str,
        expected_files: &'static [&'static str],
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
    stt: ModelSlot,
    tts: ModelSlot,
}

impl ModelManager {
    /// Create a new `ModelManager` rooted at `~/.buzz/models/`.
    ///
    /// Returns `None` if the home directory cannot be resolved.
    pub fn new() -> Option<Self> {
        let models_dir = dirs::home_dir()?.join(".buzz").join("models");
        Some(Self {
            models_dir,
            stt: ModelSlot::new(STT_MODEL_DIR_NAME, STT_EXPECTED_FILES, STT_MODEL_VERSION),
            tts: ModelSlot::new(TTS_MODEL_DIR_NAME, TTS_EXPECTED_FILES, TTS_MODEL_VERSION),
        })
    }

    // ── STT accessors ────────────────────────────────────────────────────────

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
        let archive_path = self
            .models_dir
            .join(format!("{STT_MODEL_DIR_NAME}.tar.bz2"));
        let temp_dir = self.models_dir.join(format!("{STT_MODEL_DIR_NAME}.tmp"));

        eprintln!("buzz-desktop: downloading STT model from {STT_DOWNLOAD_URL}");
        let response = fetch_url(&http_client, STT_DOWNLOAD_URL, "stt archive").await?;

        let slot = self.stt.clone();
        let bytes = download_file(
            response,
            &archive_path,
            MAX_STT_DOWNLOAD_BYTES,
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

        // Verify archive integrity before extraction.
        let hash = sha256_file(&archive_path).await?;
        if hash != STT_ARCHIVE_SHA256 {
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(format!(
                "STT archive integrity check failed: expected {STT_ARCHIVE_SHA256}, got {hash}"
            ));
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

        let extracted_subdir = temp_dir.join(STT_ARCHIVE_SUBDIR);
        if !extracted_subdir.is_dir() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!(
                "expected subdir '{STT_ARCHIVE_SUBDIR}' not found after extraction"
            ));
        }

        // Write the CC-BY-4.0 attribution sidecar before the atomic install,
        // so it lands in the final model dir as part of the same rename. The
        // upstream tarball ships no LICENSE/NOTICE, so we provide it ourselves
        // per §3(a)(1) (license must travel with Shared material).
        let license_path = extracted_subdir.join(STT_LICENSE_FILE_NAME);
        if let Err(e) = tokio::fs::write(&license_path, STT_LICENSE_TEXT).await {
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
        let slot = ModelSlot::new(TTS_MODEL_DIR_NAME, TTS_EXPECTED_FILES, TTS_MODEL_VERSION);
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
}
