//! Speech language routing for huddle ASR/TTS.
//!
//! English keeps the upstream Parakeet + Pocket path. Deutsch switches to
//! Kroko ASR and Kokoro TTS without loading those models on English startup.

use serde::{Deserialize, Serialize};

pub const SPEECH_LANGUAGE_EN: &str = "en";
pub const SPEECH_LANGUAGE_DE: &str = "de";

pub const KOKORO_BACKEND_ID: &str = "kokoro";
pub const GERMAN_VOICE_MARTIN: &str = "kokoro:de_martin";
pub const GERMAN_VOICE_VICTORIA: &str = "kokoro:de_victoria";
pub const GERMAN_VOICE_MARTIN_ID: &str = "de_martin";
pub const GERMAN_VOICE_VICTORIA_ID: &str = "de_victoria";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpeechLanguage {
    #[default]
    #[serde(alias = "en", alias = "english")]
    En,
    #[serde(alias = "de", alias = "deutsch", alias = "german")]
    De,
}

impl SpeechLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => SPEECH_LANGUAGE_EN,
            Self::De => SPEECH_LANGUAGE_DE,
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "english" | "en-us" | "en-gb" => Ok(Self::En),
            "de" | "deutsch" | "german" | "de-de" => Ok(Self::De),
            other => Err(format!("Unsupported speech language: {other}")),
        }
    }

    pub fn asr_backend(self) -> AsrBackend {
        match self {
            Self::En => AsrBackend::Parakeet,
            Self::De => AsrBackend::Kroko,
        }
    }

    pub fn tts_backend(self) -> TtsBackend {
        match self {
            Self::En => TtsBackend::Pocket,
            Self::De => TtsBackend::Kokoro,
        }
    }

    pub fn should_load_kroko(self) -> bool {
        matches!(self, Self::De)
    }

    pub fn should_load_kokoro(self) -> bool {
        matches!(self, Self::De)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrBackend {
    Parakeet,
    Kroko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsBackend {
    Pocket,
    Kokoro,
}

pub fn default_german_voice_key() -> &'static str {
    GERMAN_VOICE_MARTIN
}

pub fn is_german_voice_key(key: &str) -> bool {
    key == GERMAN_VOICE_MARTIN || key == GERMAN_VOICE_VICTORIA
}

pub fn resolve_german_voice_key(preferences: &[String]) -> &'static str {
    preferences
        .iter()
        .find(|key| is_german_voice_key(key))
        .map(|key| {
            if key.as_str() == GERMAN_VOICE_VICTORIA {
                GERMAN_VOICE_VICTORIA
            } else {
                GERMAN_VOICE_MARTIN
            }
        })
        .unwrap_or(GERMAN_VOICE_MARTIN)
}

pub fn german_voice_id(key: &str) -> &'static str {
    if key == GERMAN_VOICE_VICTORIA {
        GERMAN_VOICE_VICTORIA_ID
    } else {
        GERMAN_VOICE_MARTIN_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_keeps_upstream_backends() {
        assert_eq!(SpeechLanguage::En.asr_backend(), AsrBackend::Parakeet);
        assert_eq!(SpeechLanguage::En.tts_backend(), TtsBackend::Pocket);
        assert!(!SpeechLanguage::En.should_load_kroko());
        assert!(!SpeechLanguage::En.should_load_kokoro());
    }

    #[test]
    fn deutsch_routes_to_kroko_and_kokoro() {
        assert_eq!(SpeechLanguage::De.asr_backend(), AsrBackend::Kroko);
        assert_eq!(SpeechLanguage::De.tts_backend(), TtsBackend::Kokoro);
        assert!(SpeechLanguage::De.should_load_kroko());
        assert!(SpeechLanguage::De.should_load_kokoro());
    }

    #[test]
    fn deutsch_plus_martin_selects_kokoro_martin() {
        assert_eq!(
            resolve_german_voice_key(&[GERMAN_VOICE_MARTIN.to_string()]),
            GERMAN_VOICE_MARTIN
        );
        assert_eq!(german_voice_id(GERMAN_VOICE_MARTIN), GERMAN_VOICE_MARTIN_ID);
    }

    #[test]
    fn deutsch_plus_victoria_selects_kokoro_victoria() {
        assert_eq!(
            resolve_german_voice_key(&[GERMAN_VOICE_VICTORIA.to_string()]),
            GERMAN_VOICE_VICTORIA
        );
        assert_eq!(
            german_voice_id(GERMAN_VOICE_VICTORIA),
            GERMAN_VOICE_VICTORIA_ID
        );
    }

    #[test]
    fn missing_german_voice_defaults_to_martin() {
        assert_eq!(
            resolve_german_voice_key(&["pocket:mary".to_string()]),
            GERMAN_VOICE_MARTIN
        );
    }

    #[test]
    fn parse_accepts_stable_language_ids() {
        assert_eq!(SpeechLanguage::parse("en").unwrap(), SpeechLanguage::En);
        assert_eq!(SpeechLanguage::parse("de").unwrap(), SpeechLanguage::De);
        assert!(SpeechLanguage::parse("fr").is_err());
    }
}
