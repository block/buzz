//! Flag-gated Smart Turn v3.1 CPU graph cache under `~/.buzz/models/`.

use super::*;

pub(super) const SMART_TURN_MODEL_FILENAME: &str = "smart-turn-v3.1-cpu.onnx";
const SMART_TURN_MODEL_REVISION: &str = "16c8130e06d3af59663be3fd7c9ac80624850e6c";
pub(super) const SMART_TURN_MODEL_SHA256: &str =
    "fb68d55c2d542ce79e44b12013bfd571e90df8594ab096d757198e851b0c6594";
pub(super) const SMART_TURN_MODEL_SIZE: u64 = 8_679_180;
const SMART_TURN_DOWNLOAD_URL: &str = "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/\
     16c8130e06d3af59663be3fd7c9ac80624850e6c/smart-turn-v3.1-cpu.onnx";

/// Identifies the pinned Pipecat Smart Turn v3.1 CPU graph.
pub(super) const SMART_TURN_MODEL_VERSION: &str = "1";

/// Maximum Smart Turn model size (16 MB — pinned graph is 8,679,180 bytes).
const MAX_SMART_TURN_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;

/// Final directory name under `~/.buzz/models/`.
pub(super) const SMART_TURN_MODEL_DIR_NAME: &str = "smart-turn-v3.1";

const SMART_TURN_EXPECTED_FILES: &[&str] = &[SMART_TURN_MODEL_FILENAME];

fn smart_turn_expected_size(filename: &str) -> Option<u64> {
    (filename == SMART_TURN_MODEL_FILENAME).then_some(SMART_TURN_MODEL_SIZE)
}

pub(super) fn smart_turn_model_slot() -> ModelSlot {
    ModelSlot::new(
        SMART_TURN_MODEL_DIR_NAME,
        SMART_TURN_EXPECTED_FILES,
        SMART_TURN_MODEL_VERSION,
    )
    .with_expected_sizes(smart_turn_expected_size)
}

/// Slot plus interrupted-install recovery, used from `ModelManager::new`.
pub(super) fn prepared_smart_turn_slot(models_dir: &Path) -> ModelSlot {
    let slot = smart_turn_model_slot();
    slot.recover_interrupted_install(models_dir);
    slot
}

/// Whether the experimental semantic turn gate is enabled for this process.
pub fn smart_turn_feature_enabled() -> bool {
    std::env::var("BUZZ_HUDDLE_SMART_TURN").is_ok_and(|value| value == "1")
}

/// Path to the pinned Smart Turn graph, or `None` when its cache is incomplete.
pub fn smart_turn_model_path() -> Option<PathBuf> {
    global_model_manager()?.smart_turn_model_path()
}

impl ModelManager {
    /// Path to the pinned Smart Turn graph, or `None` if its cache is not ready.
    pub fn smart_turn_model_path(&self) -> Option<PathBuf> {
        self.smart_turn
            .dir_if_ready(&self.models_dir)
            .map(|directory| directory.join(SMART_TURN_MODEL_FILENAME))
    }

    /// Start the optional Smart Turn download when the feature flag is on.
    pub(super) fn maybe_start_smart_turn_download(&self, http_client: reqwest::Client) {
        if !smart_turn_feature_enabled() {
            return;
        }
        let manager = self.clone();
        self.smart_turn.start_download(
            &self.models_dir,
            http_client,
            "Smart Turn",
            move |client| async move { manager.download_smart_turn_model(client).await },
        );
    }

    /// Download and verify the pinned Smart Turn v3.1 CPU graph.
    async fn download_smart_turn_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;
        let temp_dir = self
            .models_dir
            .join(format!("{SMART_TURN_MODEL_DIR_NAME}.tmp"));
        fresh_temp_dir(&temp_dir).await?;
        let destination = temp_dir.join(SMART_TURN_MODEL_FILENAME);

        eprintln!(
            "buzz-desktop: downloading Smart Turn v3.1 CPU model from {SMART_TURN_DOWNLOAD_URL}"
        );
        let response = fetch_url(
            &http_client,
            SMART_TURN_DOWNLOAD_URL,
            "Smart Turn v3.1 CPU model",
        )
        .await
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&temp_dir);
        })?;
        let slot = self.smart_turn.clone();
        let bytes = download_file(
            response,
            &destination,
            MAX_SMART_TURN_DOWNLOAD_BYTES,
            "Smart Turn v3.1 CPU model",
            move |downloaded, content_length| {
                if let Some(percent) =
                    content_length.and_then(|total| (downloaded * 89).checked_div(total))
                {
                    slot.set_status(ModelStatus::Downloading {
                        progress_percent: percent.min(89) as u8,
                    });
                }
            },
        )
        .await
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&temp_dir);
        })?;
        if bytes != SMART_TURN_MODEL_SIZE {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!(
                "Smart Turn model size check failed: expected {SMART_TURN_MODEL_SIZE}, got {bytes}"
            ));
        }
        let hash = sha256_file(&destination).await?;
        if hash != SMART_TURN_MODEL_SHA256 {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(format!(
                "Smart Turn model integrity check failed at revision {SMART_TURN_MODEL_REVISION}: \
                 expected {SMART_TURN_MODEL_SHA256}, got {hash}"
            ));
        }

        self.smart_turn.set_status(ModelStatus::Downloading {
            progress_percent: 90,
        });
        self.smart_turn
            .verify_and_install(&self.models_dir, &temp_dir, None)
            .await
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&temp_dir);
            })?;
        eprintln!(
            "buzz-desktop: Smart Turn v3.1 CPU model ready at {}",
            self.smart_turn.model_dir(&self.models_dir).display()
        );
        Ok(())
    }
}
