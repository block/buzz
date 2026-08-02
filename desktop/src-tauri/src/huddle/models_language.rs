//! Language-aware model selection and Portuguese model installation.

use super::*;

struct ArchiveModelSpec<'a> {
    slot: &'a ModelSlot,
    url: &'static str,
    expected_hash: &'static str,
    archive_subdir: &'static str,
    archive_stem: &'static str,
    max_bytes: u64,
    license_text: &'static str,
    label: &'static str,
}

impl ModelManager {
    pub fn set_speech_language(&self, language: &str) -> Result<(), String> {
        if !matches!(language, "en-US" | "pt-BR") {
            return Err(format!("unsupported speech language: {language}"));
        }
        *self
            .speech_language
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = language.to_string();
        Ok(())
    }

    fn uses_portuguese(&self) -> bool {
        self.speech_language
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_str()
            == "pt-BR"
    }

    fn active_stt(&self) -> &ModelSlot {
        if self.uses_portuguese() {
            &self.stt_pt
        } else {
            &self.stt
        }
    }

    fn active_tts(&self) -> &ModelSlot {
        if self.uses_portuguese() {
            &self.tts_pt
        } else {
            &self.tts
        }
    }

    pub fn stt_model_dir(&self) -> Option<PathBuf> {
        self.active_stt().dir_if_ready(&self.models_dir)
    }

    pub fn is_stt_ready(&self) -> bool {
        self.active_stt().is_ready(&self.models_dir)
    }

    pub fn stt_status(&self) -> ModelStatus {
        self.active_stt().status()
    }

    pub fn take_stt_ready(&self) -> bool {
        self.active_stt().take_ready()
    }

    pub fn tts_model_dir(&self) -> Option<PathBuf> {
        self.active_tts().dir_if_ready(&self.models_dir)
    }

    pub fn is_tts_ready(&self) -> bool {
        self.active_tts().is_ready(&self.models_dir)
    }

    pub fn tts_status(&self) -> ModelStatus {
        self.active_tts().status()
    }

    pub fn take_tts_ready(&self) -> bool {
        self.active_tts().take_ready()
    }

    /// Start the active-language STT download. English keeps the legacy-model cleanup path.
    pub fn start_stt_download(&self, http_client: reqwest::Client) {
        if self.uses_portuguese() {
            let manager = self.clone();
            self.stt_pt.start_download(
                &self.models_dir,
                http_client,
                "stt-pt-BR",
                move |client| async move { manager.download_stt_pt_model(client).await },
            );
            return;
        }
        let manager = self.clone();
        self.stt.start_download(
            &self.models_dir,
            http_client,
            "stt",
            move |client| async move { manager.download_stt_model(client).await },
        );
        if self.stt.is_ready(&self.models_dir) {
            let models_dir = self.models_dir.clone();
            tauri::async_runtime::spawn(async move {
                cleanup_legacy_moonshine_dir(&models_dir).await;
            });
        }
    }

    /// Start the active-language TTS download.
    pub fn start_tts_download(&self, http_client: reqwest::Client) {
        if self.uses_portuguese() {
            let manager = self.clone();
            self.tts_pt.start_download(
                &self.models_dir,
                http_client,
                "tts-pt-BR",
                move |client| async move { manager.download_tts_pt_model(client).await },
            );
            return;
        }
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

    async fn download_archive_model(
        &self,
        http_client: reqwest::Client,
        spec: ArchiveModelSpec<'_>,
    ) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|error| format!("create models dir: {error}"))?;

        let archive_path = self
            .models_dir
            .join(format!("{}.tar.bz2", spec.archive_stem));
        let temp_dir = self.models_dir.join(format!("{}.tmp", spec.archive_stem));
        let response = fetch_url(&http_client, spec.url, spec.label).await?;
        let progress_slot = spec.slot.clone();
        download_file(
            response,
            &archive_path,
            spec.max_bytes,
            spec.label,
            move |downloaded, content_length| {
                if let Some(percent) =
                    content_length.and_then(|total| (downloaded * 89).checked_div(total))
                {
                    progress_slot.set_status(ModelStatus::Downloading {
                        progress_percent: percent.min(89) as u8,
                    });
                }
            },
        )
        .await?;

        let actual_hash = sha256_file(&archive_path).await?;
        if actual_hash != spec.expected_hash {
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(format!(
                "{} integrity check failed: expected {}, got {actual_hash}",
                spec.label, spec.expected_hash
            ));
        }

        spec.slot.set_status(ModelStatus::Downloading {
            progress_percent: 90,
        });
        fresh_temp_dir(&temp_dir).await?;
        let (archive, destination) = (archive_path.clone(), temp_dir.clone());
        tokio::task::spawn_blocking(move || extract_archive(&archive, &destination))
            .await
            .map_err(|error| format!("archive extraction task panicked: {error}"))??;
        let extracted_subdir = temp_dir.join(spec.archive_subdir);
        if !extracted_subdir.is_dir() {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(format!(
                "expected subdir '{}' not found after extraction",
                spec.archive_subdir
            ));
        }
        tokio::fs::write(
            extracted_subdir.join(TTS_LICENSE_FILE_NAME),
            spec.license_text,
        )
        .await
        .map_err(|error| format!("write model license sidecar: {error}"))?;

        if let Err(error) = spec
            .slot
            .verify_and_install(&self.models_dir, &extracted_subdir, Some(&temp_dir))
            .await
        {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            let _ = tokio::fs::remove_file(&archive_path).await;
            return Err(error);
        }
        let _ = tokio::fs::remove_file(&archive_path).await;
        eprintln!(
            "buzz-desktop: {} ready at {}",
            spec.label,
            spec.slot.model_dir(&self.models_dir).display()
        );
        Ok(())
    }

    async fn download_stt_pt_model(&self, client: reqwest::Client) -> Result<(), String> {
        self.download_archive_model(
            client,
            ArchiveModelSpec {
                slot: &self.stt_pt,
                url: STT_PT_DOWNLOAD_URL,
                expected_hash: STT_PT_ARCHIVE_SHA256,
                archive_subdir: STT_PT_ARCHIVE_SUBDIR,
                archive_stem: STT_PT_MODEL_DIR_NAME,
                max_bytes: MAX_STT_DOWNLOAD_BYTES,
                license_text: STT_PT_LICENSE_TEXT,
                label: "Whisper Tiny multilingual STT",
            },
        )
        .await
    }

    async fn download_tts_pt_model(&self, client: reqwest::Client) -> Result<(), String> {
        self.download_archive_model(
            client,
            ArchiveModelSpec {
                slot: &self.tts_pt,
                url: TTS_PT_DOWNLOAD_URL,
                expected_hash: TTS_PT_ARCHIVE_SHA256,
                archive_subdir: TTS_PT_ARCHIVE_SUBDIR,
                archive_stem: TTS_PT_MODEL_DIR_NAME,
                max_bytes: MAX_STT_DOWNLOAD_BYTES,
                license_text: TTS_PT_LICENSE_TEXT,
                label: "Piper pt-BR TTS",
            },
        )
        .await
    }
}
