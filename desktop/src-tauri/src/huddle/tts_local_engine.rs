//! Local TTS engine abstraction for the English Pocket and Portuguese Piper models.

use super::*;

const SYNTH_STEPS: usize = 1;

pub(super) enum LocalTtsEngine {
    Pocket(Box<PocketTts>),
    Piper {
        engine: sherpa_onnx::OfflineTts,
        sample_rate: u32,
    },
}

impl LocalTtsEngine {
    pub(super) fn load(model_dir: &std::path::Path) -> Result<Self, String> {
        let piper_model = model_dir.join("pt_BR-faber-medium.onnx");
        if piper_model.is_file() {
            let mut config = sherpa_onnx::OfflineTtsConfig::default();
            config.model.vits.model = Some(piper_model.to_string_lossy().into_owned());
            config.model.vits.tokens =
                Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
            config.model.vits.data_dir = Some(
                model_dir
                    .join("espeak-ng-data")
                    .to_string_lossy()
                    .into_owned(),
            );
            config.model.num_threads = 1;
            config.model.debug = false;
            let engine = sherpa_onnx::OfflineTts::create(&config)
                .ok_or("Piper pt-BR engine initialization failed")?;
            let sample_rate = u32::try_from(engine.sample_rate())
                .map_err(|_| "Piper reported an invalid sample rate")?;
            return Ok(Self::Piper {
                engine,
                sample_rate,
            });
        }
        load_text_to_speech(&model_dir.to_string_lossy())
            .map(|engine| Self::Pocket(Box::new(engine)))
    }

    pub(super) fn sample_rate(&self) -> u32 {
        match self {
            Self::Pocket(_) => SAMPLE_RATE,
            Self::Piper { sample_rate, .. } => *sample_rate,
        }
    }

    pub(super) fn uses_reference_voice(&self) -> bool {
        matches!(self, Self::Pocket(_))
    }

    pub(super) fn language(&self) -> &'static str {
        match self {
            Self::Pocket(_) => "en-US",
            Self::Piper { .. } => "pt-BR",
        }
    }

    pub(super) fn split_text_into_chunks(&self, text: &str) -> Result<Vec<String>, String> {
        match self {
            Self::Pocket(engine) => engine.split_text_into_chunks(text),
            Self::Piper { .. } => Ok(vec![text.to_string()]),
        }
    }

    pub(super) fn synth_chunk(
        &self,
        text: &str,
        style: Option<&VoiceStyle>,
    ) -> Result<Vec<f32>, String> {
        match self {
            Self::Pocket(engine) => engine.synth_chunk(
                text,
                "en",
                style.ok_or("Pocket voice style is unavailable")?,
                SYNTH_STEPS,
            ),
            Self::Piper { engine, .. } => engine
                .generate_with_config::<fn(&[f32], f32) -> bool>(
                    text,
                    &sherpa_onnx::GenerationConfig::default(),
                    None,
                )
                .map(|audio| audio.samples().to_vec())
                .ok_or_else(|| "Piper pt-BR synthesis failed".to_string()),
        }
    }
}

pub(super) fn reconcile_local_voice(
    model_dir: &std::path::Path,
    selected_voice: &Mutex<String>,
    voice_name: &mut String,
    style: &mut Option<VoiceStyle>,
) -> bool {
    style
        .as_mut()
        .is_some_and(|style| reconcile_selected_voice(model_dir, selected_voice, voice_name, style))
}
