//! German speech model slots (Kroko ASR + Kokoro TTS). Loaded only when Deutsch
//! is selected — English startup never touches these downloads.

use super::*;

const KROKO_MODEL_DIR_NAME: &str = "kroko-de";
const KROKO_MODEL_VERSION: &str = "1";
const KROKO_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";
const KROKO_EXPECTED_FILES: &[&str] = &[
    "encoder.onnx",
    "decoder.onnx",
    "joiner.onnx",
    "tokens.txt",
    KROKO_LICENSE_FILE_NAME,
];

const KROKO_BASE: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-de-kroko-2025-08-06/resolve/main";

const KROKO_ENCODER_SHA256: &str = "6e83993d6967ec7a3498b055b7e85ace85b5d64d1b1e8773cb29a43a11f5edb5";
const KROKO_DECODER_SHA256: &str = "94a29592b403c53fa2231b478637da1ab4abcef7f5e46e432098416a4a3ed562";
const KROKO_JOINER_SHA256: &str = "28356bff070aea51ab1d725a3278e81d19f9300f860d3248a7014292264df15a";
const KROKO_TOKENS_SHA256: &str = "86e8370994ff2c01149ba8c4f8709aa93cdc18914b27a717e291e96faf39a6eb";

const KROKO_LICENSE_TEXT: &str = "\
Kroko German ASR (sherpa-onnx streaming zipformer, 2025-08-06)
Original model: Banafo/Kroko-ASR
Packaged for sherpa-onnx by csukuangfj.

See license at https://huggingface.co/Banafo/Kroko-ASR

Provided \"AS IS\", without warranty of any kind.
";

const KOKORO_MODEL_DIR_NAME: &str = "kokoro-de";
const KOKORO_MODEL_VERSION: &str = "1";
const KOKORO_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";
const KOKORO_EXPECTED_FILES: &[&str] = &[
    "model.onnx",
    "voices-martin.npz",
    "tokens.txt",
    KOKORO_LICENSE_FILE_NAME,
];

const KOKORO_MARTIN_ONNX_URL: &str =
    "https://huggingface.co/Godelaune/Kokoro-82M-ONNX-German-Martin/resolve/main/kokoro-martin.onnx";
const KOKORO_MARTIN_VOICES_URL: &str =
    "https://huggingface.co/Godelaune/Kokoro-82M-ONNX-German-Martin/resolve/main/voices-martin.npz";
const KOKORO_TOKENS_URL: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-kokoro-multi-lang-v1_0/resolve/main/tokens.txt";
const KOKORO_VICTORIA_URL: &str =
    "https://huggingface.co/kikiri-tts/kikiri-german-victoria/resolve/main/voices/victoria.pt";

const KOKORO_LICENSE_TEXT: &str = "\
Kokoro German TTS
Martin ONNX: Godelaune/Kokoro-82M-ONNX-German-Martin (Apache-2.0)
Martin source: kikiri-tts/kikiri-german-martin (Apache-2.0)
Victoria voice: kikiri-tts/kikiri-german-victoria (Apache-2.0)
Kokoro architecture: hexgrad/Kokoro-82M
Runtime: sherpa-onnx OfflineTts Kokoro (Apache-2.0)

Provided \"AS IS\", without warranty of any kind.
";

const MAX_KROKO_FILE_BYTES: u64 = 120 * 1024 * 1024;
const MAX_KOKORO_FILE_BYTES: u64 = 400 * 1024 * 1024;

impl ModelManager {
    pub(super) fn with_german_slots(mut self) -> Self {
        self.kroko = ModelSlot::new(
            KROKO_MODEL_DIR_NAME,
            KROKO_EXPECTED_FILES,
            KROKO_MODEL_VERSION,
        );
        self.kokoro = ModelSlot::new(
            KOKORO_MODEL_DIR_NAME,
            KOKORO_EXPECTED_FILES,
            KOKORO_MODEL_VERSION,
        );
        self
    }

    pub fn kroko_model_dir(&self) -> Option<PathBuf> {
        self.kroko.dir_if_ready(&self.models_dir)
    }

    pub fn is_kroko_ready(&self) -> bool {
        self.kroko.is_ready(&self.models_dir)
    }

    pub fn kroko_status(&self) -> ModelStatus {
        self.kroko.status()
    }

    pub fn take_kroko_ready(&self) -> bool {
        self.kroko.take_ready()
    }

    pub fn kokoro_model_dir(&self) -> Option<PathBuf> {
        self.kokoro.dir_if_ready(&self.models_dir)
    }

    pub fn is_kokoro_ready(&self) -> bool {
        self.kokoro.is_ready(&self.models_dir)
    }

    pub fn kokoro_status(&self) -> ModelStatus {
        self.kokoro.status()
    }

    pub fn take_kokoro_ready(&self) -> bool {
        self.kokoro.take_ready()
    }

    pub fn start_kroko_download(&self, http_client: reqwest::Client) {
        let manager = self.clone();
        self.kroko.start_download(
            &self.models_dir,
            http_client,
            "kroko",
            move |client| async move { manager.download_kroko_model(client).await },
        );
    }

    pub fn start_kokoro_download(&self, http_client: reqwest::Client) {
        let manager = self.clone();
        self.kokoro.start_download(
            &self.models_dir,
            http_client,
            "kokoro",
            move |client| async move { manager.download_kokoro_model(client).await },
        );
    }

    pub async fn remove_kroko_model(&self) -> Result<(), String> {
        remove_model_dir(&self.models_dir.join(KROKO_MODEL_DIR_NAME)).await?;
        self.kroko.set_status(ModelStatus::NotDownloaded);
        Ok(())
    }

    pub async fn remove_kokoro_model(&self) -> Result<(), String> {
        remove_model_dir(&self.models_dir.join(KOKORO_MODEL_DIR_NAME)).await?;
        self.kokoro.set_status(ModelStatus::NotDownloaded);
        Ok(())
    }

    async fn download_kroko_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        eprintln!("buzz-desktop: Loading Kroko...");
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;
        let temp_dir = self.models_dir.join(format!("{KROKO_MODEL_DIR_NAME}.tmp"));
        fresh_temp_dir(&temp_dir).await?;

        let files = [
            ("encoder.onnx", KROKO_ENCODER_SHA256),
            ("decoder.onnx", KROKO_DECODER_SHA256),
            ("joiner.onnx", KROKO_JOINER_SHA256),
            ("tokens.txt", KROKO_TOKENS_SHA256),
        ];
        for (index, (filename, expected_hash)) in files.iter().enumerate() {
            let url = format!("{KROKO_BASE}/{filename}");
            let dest = temp_dir.join(filename);
            let response = fetch_url(&http_client, &url, filename).await?;
            download_file(
                response,
                &dest,
                MAX_KROKO_FILE_BYTES,
                filename,
                |downloaded, total| {
                    if let Some(pct) = total.and_then(|t| (downloaded * 20).checked_div(t)) {
                        let overall = (index as u64 * 20 + pct.min(20)) as u8;
                        self.kroko.set_status(ModelStatus::Downloading {
                            progress_percent: overall.min(89),
                        });
                    }
                },
            )
            .await?;
            let hash = sha256_file(&dest).await?;
            if hash != *expected_hash {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Err(format!(
                    "Kroko {filename} integrity check failed: expected {expected_hash}, got {hash}"
                ));
            }
        }

        tokio::fs::write(temp_dir.join(KROKO_LICENSE_FILE_NAME), KROKO_LICENSE_TEXT)
            .await
            .map_err(|e| format!("write Kroko license: {e}"))?;

        if let Err(e) = self
            .kroko
            .verify_and_install(&self.models_dir, &temp_dir, None)
            .await
        {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(e);
        }
        eprintln!(
            "buzz-desktop: Kroko ready at {}",
            self.kroko.model_dir(&self.models_dir).display()
        );
        Ok(())
    }

    async fn download_kokoro_model(&self, http_client: reqwest::Client) -> Result<(), String> {
        eprintln!("buzz-desktop: Loading Kokoro...");
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| format!("create models dir: {e}"))?;
        let temp_dir = self.models_dir.join(format!("{KOKORO_MODEL_DIR_NAME}.tmp"));
        fresh_temp_dir(&temp_dir).await?;

        let files = [
            (KOKORO_MARTIN_ONNX_URL, "model.onnx"),
            (KOKORO_MARTIN_VOICES_URL, "voices-martin.npz"),
            (KOKORO_TOKENS_URL, "tokens.txt"),
            (KOKORO_VICTORIA_URL, "victoria.pt"),
        ];
        for (index, (url, filename)) in files.iter().enumerate() {
            let dest = temp_dir.join(filename);
            match fetch_url(&http_client, url, filename).await {
                Ok(response) => {
                    download_file(
                        response,
                        &dest,
                        MAX_KOKORO_FILE_BYTES,
                        filename,
                        |downloaded, total| {
                            if let Some(pct) = total.and_then(|t| (downloaded * 20).checked_div(t))
                            {
                                let overall = (index as u64 * 20 + pct.min(20)) as u8;
                                self.kokoro.set_status(ModelStatus::Downloading {
                                    progress_percent: overall.min(89),
                                });
                            }
                        },
                    )
                    .await?;
                }
                Err(error) if *filename == "victoria.pt" || *filename == "tokens.txt" => {
                    eprintln!("buzz-desktop: optional Kokoro artifact {filename} skipped: {error}");
                }
                Err(error) => {
                    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                    return Err(error);
                }
            }
        }

        tokio::fs::write(temp_dir.join(KOKORO_LICENSE_FILE_NAME), KOKORO_LICENSE_TEXT)
            .await
            .map_err(|e| format!("write Kokoro license: {e}"))?;

        if !temp_dir.join("tokens.txt").is_file() {
            tokio::fs::write(temp_dir.join("tokens.txt"), KOKORO_FALLBACK_TOKENS)
                .await
                .map_err(|e| format!("write Kokoro tokens: {e}"))?;
        }

        if let Err(e) = self
            .kokoro
            .verify_and_install(&self.models_dir, &temp_dir, None)
            .await
        {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Err(e);
        }
        eprintln!(
            "buzz-desktop: Kokoro ready at {}",
            self.kokoro.model_dir(&self.models_dir).display()
        );
        Ok(())
    }
}

async fn remove_model_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|e| format!("remove {}: {e}", path.display()))?;
    }
    Ok(())
}

pub fn kroko_model_dir() -> Option<PathBuf> {
    global_model_manager()?.kroko_model_dir()
}

pub fn is_kroko_ready() -> bool {
    global_model_manager()
        .map(|m| m.is_kroko_ready())
        .unwrap_or(false)
}

pub fn kokoro_model_dir() -> Option<PathBuf> {
    global_model_manager()?.kokoro_model_dir()
}

pub fn is_kokoro_ready() -> bool {
    global_model_manager()
        .map(|m| m.is_kokoro_ready())
        .unwrap_or(false)
}

const KOKORO_FALLBACK_TOKENS: &str = include_str!("kokoro_tokens.txt");
