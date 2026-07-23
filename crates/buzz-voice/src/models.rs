use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MANIFEST_FILENAME: &str = ".buzz-model-manifest";

pub const STT_MODEL_VERSION: &str = "2";
pub const TTS_MODEL_VERSION: &str = "3";
pub const STT_MODEL_DIR_NAME: &str = "parakeet-tdt-ctc-110m-en";
pub const TTS_MODEL_DIR_NAME: &str = "pocket-tts";
pub const STT_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";
pub const TTS_LICENSE_FILE_NAME: &str = "MODEL_LICENSE.txt";

pub const STT_EXPECTED_FILES: &[&str] = &["model.int8.onnx", "tokens.txt", STT_LICENSE_FILE_NAME];

pub const TTS_EXPECTED_FILES: &[&str] = &[
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

pub const STT_MODEL_PACK: ModelPackSpec = ModelPackSpec {
    dir_name: STT_MODEL_DIR_NAME,
    expected_files: STT_EXPECTED_FILES,
    version: STT_MODEL_VERSION,
};

pub const TTS_MODEL_PACK: ModelPackSpec = ModelPackSpec {
    dir_name: TTS_MODEL_DIR_NAME,
    expected_files: TTS_EXPECTED_FILES,
    version: TTS_MODEL_VERSION,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    NotDownloaded,
    Downloading { progress_percent: u8 },
    Ready,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceModelStatus {
    pub stt: ModelStatus,
    pub tts: ModelStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPackSpec {
    pub dir_name: &'static str,
    pub expected_files: &'static [&'static str],
    pub version: &'static str,
}

impl ModelPackSpec {
    pub fn model_dir(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.dir_name)
    }

    pub fn is_ready(&self, models_dir: &Path) -> bool {
        let dir = self.model_dir(models_dir);
        std::fs::read_to_string(dir.join(MANIFEST_FILENAME))
            .map(|version| version.trim() == self.version)
            .unwrap_or(false)
            && self
                .expected_files
                .iter()
                .all(|file| dir.join(file).is_file())
    }

    pub fn dir_if_ready(&self, models_dir: &Path) -> Option<PathBuf> {
        self.is_ready(models_dir)
            .then(|| self.model_dir(models_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: ModelPackSpec = ModelPackSpec {
        dir_name: "model",
        expected_files: &["model.onnx", "tokens.txt"],
        version: "2",
    };

    #[test]
    fn reports_missing_model_as_not_ready() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!SPEC.is_ready(temp.path()));
        assert_eq!(SPEC.dir_if_ready(temp.path()), None);
    }

    #[test]
    fn reports_model_ready_when_manifest_and_files_match() {
        let temp = tempfile::tempdir().unwrap();
        let dir = SPEC.model_dir(temp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILENAME), "2\n").unwrap();
        std::fs::write(dir.join("model.onnx"), []).unwrap();
        std::fs::write(dir.join("tokens.txt"), []).unwrap();

        assert!(SPEC.is_ready(temp.path()));
        assert_eq!(SPEC.dir_if_ready(temp.path()), Some(dir));
    }

    #[test]
    fn rejects_wrong_manifest_version() {
        let temp = tempfile::tempdir().unwrap();
        let dir = SPEC.model_dir(temp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILENAME), "1\n").unwrap();
        std::fs::write(dir.join("model.onnx"), []).unwrap();
        std::fs::write(dir.join("tokens.txt"), []).unwrap();

        assert!(!SPEC.is_ready(temp.path()));
    }

    #[test]
    fn rejects_missing_expected_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = SPEC.model_dir(temp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILENAME), "2\n").unwrap();
        std::fs::write(dir.join("model.onnx"), []).unwrap();

        assert!(!SPEC.is_ready(temp.path()));
    }
}
