use super::*;
use crate::huddle::tts_voice_registry::POCKET_VOICES;

const PRESET_VOICE_TTS_MODEL_VERSION: &str = "4";

/// Attribution written beside every installed Pocket model and voice asset.
pub(super) const TTS_LICENSE_TEXT: &str = "\
Pocket TTS
© Kyutai.

Licensed under the Creative Commons Attribution 4.0 International License
(CC-BY-4.0). License text: https://creativecommons.org/licenses/by/4.0/

Original model by Kyutai: https://huggingface.co/kyutai/pocket-tts
Paper: Charles, Roebel, et al., Pocket TTS (arXiv:2509.06926).
Mimi neural codec by Kyutai is bundled as part of the model.

April 2026 ONNX export by KevinAHM:
https://huggingface.co/KevinAHM/pocket-tts-onnx
Pinned revision: 58a6d00cf13d239b6748cb0769f35c580a8f606c

Bundled English VCTK presets: Anna (p228), Vera (p229), Fantine (p244),
Charles (p254), Paul (p259), Eponine (p262), Azelma (p303), George (p315),
Mary (p333), Jane (p339), Michael (p360), and Eve (p361). These exact,
ai-coustics-enhanced WAVs come from Kyutai's tts-voices repository at revision
323332d33f997de8394f24a193e1a76df720e01a.
Source: https://huggingface.co/kyutai/tts-voices/tree/323332d33f997de8394f24a193e1a76df720e01a/vctk
Original recordings: Voice Cloning Toolkit (VCTK) corpus,
https://datashare.ed.ac.uk/handle/10283/3443 (CC-BY-4.0).
Enhancement (denoise/dereverb): ai-coustics, https://ai-coustics.com/

Buzz ships the ONNX/model artifacts and voice WAVs unmodified, renamed only
by placement in the local model directory.

Provided \"AS IS\", without warranty of any kind, express or implied. See the
license text for full warranty disclaimer.
";

fn is_embedded_voice_file(filename: &str) -> bool {
    POCKET_VOICES
        .iter()
        .any(|voice| voice.bytes.is_some() && voice.reference_file == filename)
}

/// Add the official VCTK presets to an otherwise-ready v4 install.
///
/// Model artifacts and Mary already exist in v4. The manifest is written last,
/// so interruption leaves v4 intact and the next launch retries.
pub(super) fn install_vctk_presets_into_v4_model(models_dir: &Path) -> Result<(), String> {
    let model_dir = models_dir.join(TTS_MODEL_DIR_NAME);
    let manifest_path = model_dir.join(MANIFEST_FILENAME);
    let version = match std::fs::read_to_string(&manifest_path) {
        Ok(version) => version,
        Err(_) => return Ok(()),
    };
    if version.trim() != PRESET_VOICE_TTS_MODEL_VERSION {
        return Ok(());
    }
    if !TTS_EXPECTED_FILES
        .iter()
        .filter(|filename| !is_embedded_voice_file(filename))
        .all(|filename| model_dir.join(filename).is_file())
    {
        return Ok(());
    }

    for voice in POCKET_VOICES {
        let Some(bytes) = voice.bytes else {
            continue;
        };
        std::fs::write(model_dir.join(voice.reference_file), bytes)
            .map_err(|error| format!("write bundled {} voice: {error}", voice.display_name))?;
    }
    let retired_marius = model_dir.join("marius.wav");
    if retired_marius.is_file() {
        std::fs::remove_file(retired_marius)
            .map_err(|error| format!("remove retired Marius voice: {error}"))?;
    }
    std::fs::write(model_dir.join(TTS_LICENSE_FILE_NAME), TTS_LICENSE_TEXT)
        .map_err(|error| format!("update Pocket voice notice: {error}"))?;
    std::fs::write(manifest_path, TTS_MODEL_VERSION)
        .map_err(|error| format!("update Pocket model manifest: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v4_install_adds_presets_without_redownloading_models() {
        let temp = tempfile::tempdir().expect("tempdir");
        let model_dir = temp.path().join(TTS_MODEL_DIR_NAME);
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        for file in TTS_EXPECTED_FILES
            .iter()
            .filter(|filename| !is_embedded_voice_file(filename))
        {
            std::fs::write(model_dir.join(file), b"existing").expect("write prior file");
        }
        std::fs::write(
            model_dir.join(MANIFEST_FILENAME),
            PRESET_VOICE_TTS_MODEL_VERSION,
        )
        .expect("write prior manifest");
        std::fs::write(model_dir.join("marius.wav"), b"retired").expect("write retired voice");

        install_vctk_presets_into_v4_model(temp.path()).expect("in-place upgrade");

        for voice in POCKET_VOICES {
            if let Some(bytes) = voice.bytes {
                assert_eq!(
                    std::fs::read(model_dir.join(voice.reference_file))
                        .expect("bundled voice installed"),
                    bytes
                );
            }
        }
        assert_eq!(
            std::fs::read_to_string(model_dir.join(MANIFEST_FILENAME)).expect("updated manifest"),
            TTS_MODEL_VERSION
        );
        assert!(!model_dir.join("marius.wav").exists());
        assert!(
            ModelSlot::new(TTS_MODEL_DIR_NAME, TTS_EXPECTED_FILES, TTS_MODEL_VERSION)
                .is_ready(temp.path())
        );
    }
}
