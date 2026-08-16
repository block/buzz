//! Kokoro German TTS via sherpa-onnx OfflineTts.
//!
//! One runtime, two voices: Martin (sid 0) and Victoria (sid 1 when present).

use std::path::{Path, PathBuf};

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use crate::german_normalize::normalize_german_text;

pub const KOKORO_SAMPLE_RATE: u32 = 24_000;

pub struct KokoroGerman {
    engine: OfflineTts,
    victoria_available: bool,
}

impl KokoroGerman {
    pub fn initialize(model_dir: &Path) -> Result<Self, String> {
        Self::load(model_dir)
    }

    pub fn is_available(model_dir: &Path) -> bool {
        model_dir.join("model.onnx").is_file()
            && (model_dir.join("voices.bin").is_file()
                || model_dir.join("voices-martin.npz").is_file())
            && model_dir.join("tokens.txt").is_file()
    }

    pub fn load(model_dir: &Path) -> Result<Self, String> {
        eprintln!("buzz-desktop: Loading Kokoro...");
        let voices_bin = ensure_voices_bin(model_dir)?;
        let tokens = model_dir.join("tokens.txt");
        let model = model_dir.join("model.onnx");
        if !model.is_file() || !tokens.is_file() {
            return Err(format!(
                "Kokoro model files are missing in {}",
                model_dir.display()
            ));
        }
        let data_dir = first_existing(&[
            model_dir.join("espeak-ng-data"),
            model_dir.join("espeak-ng-data-dir"),
        ]);
        let mut kokoro = OfflineTtsKokoroModelConfig {
            model: Some(model.to_string_lossy().into_owned()),
            voices: Some(voices_bin.to_string_lossy().into_owned()),
            tokens: Some(tokens.to_string_lossy().into_owned()),
            lang: Some("de".into()),
            length_scale: 1.0,
            ..Default::default()
        };
        if let Some(dir) = data_dir {
            kokoro.data_dir = Some(dir.to_string_lossy().into_owned());
        }
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro,
                num_threads: 2,
                debug: false,
                ..Default::default()
            },
            max_num_sentences: 1,
            ..Default::default()
        };
        let engine = OfflineTts::create(&config)
            .ok_or_else(|| "Kokoro OfflineTts::create failed".to_string())?;
        let victoria_available = model_dir.join("victoria.pt").is_file()
            || model_dir.join("voices-victoria.bin").is_file()
            || engine.num_speakers() > 1;
        eprintln!("buzz-desktop: Kokoro ready");
        Ok(Self {
            engine,
            victoria_available,
        })
    }

    pub fn synthesize(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
    ) -> Result<(Vec<f32>, u32), String> {
        if text.trim().is_empty() {
            return Ok((Vec::new(), KOKORO_SAMPLE_RATE));
        }
        let sid = voice_sid(voice, self.victoria_available)?;
        eprintln!("buzz-desktop: Voice: {}", voice_label(voice));
        let normalized = normalize_german_text(text);
        let config = GenerationConfig {
            speed: speed.clamp(0.5, 2.0),
            sid,
            ..Default::default()
        };
        let audio = self
            .engine
            .generate_with_config(&normalized, &config, None::<fn(&[f32], f32) -> bool>)
            .ok_or_else(|| "Kokoro synthesis failed".to_string())?;
        let sample_rate = audio.sample_rate().max(0) as u32;
        Ok((audio.samples().to_vec(), sample_rate))
    }

    pub fn stop(&self) {}

    pub fn unload(self) {}
}

fn voice_sid(voice: &str, victoria_available: bool) -> Result<i32, String> {
    match voice {
        "de_victoria" | "kokoro:de_victoria" | "victoria" => {
            if victoria_available {
                Ok(1)
            } else {
                Err("Victoria is not installed. Download German speech models and try again.".into())
            }
        }
        _ => Ok(0),
    }
}

fn voice_label(voice: &str) -> &'static str {
    match voice {
        "de_victoria" | "kokoro:de_victoria" | "victoria" => "victoria",
        _ => "martin",
    }
}

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|path| path.exists()).cloned()
}

fn ensure_voices_bin(model_dir: &Path) -> Result<PathBuf, String> {
    let voices_bin = model_dir.join("voices.bin");
    if voices_bin.is_file() {
        return Ok(voices_bin);
    }
    let npz = model_dir.join("voices-martin.npz");
    if npz.is_file() {
        convert_npz_to_bin(&npz, &voices_bin)?;
        return Ok(voices_bin);
    }
    Err("Kokoro voices.bin is missing".into())
}

fn convert_npz_to_bin(npz_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(npz_path).map_err(|e| format!("open voices npz: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("voices npz is not a zip: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("read voices npz entry: {e}"))?;
        if !entry.name().ends_with(".npy") {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::copy(&mut entry, &mut bytes).map_err(|e| format!("read npy: {e}"))?;
        let payload = skip_npy_header(&bytes)?;
        std::fs::write(dest, payload).map_err(|e| format!("write voices.bin: {e}"))?;
        return Ok(());
    }
    Err("voices npz contained no .npy arrays".into())
}

fn skip_npy_header(bytes: &[u8]) -> Result<&[u8], String> {
    if bytes.len() < 10 || &bytes[0..6] != b"\x93NUMPY" {
        return Err("invalid npy header".into());
    }
    let major = bytes[6];
    let header_len = if major == 1 {
        u16::from_le_bytes([bytes[8], bytes[9]]) as usize + 10
    } else {
        let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        len + 12
    };
    bytes
        .get(header_len..)
        .ok_or_else(|| "truncated npy payload".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn martin_is_sid_zero() {
        assert_eq!(voice_sid("kokoro:de_martin", true).unwrap(), 0);
    }

    #[test]
    fn victoria_requires_the_voice_artifact() {
        assert!(voice_sid("kokoro:de_victoria", false).is_err());
        assert_eq!(voice_sid("kokoro:de_victoria", true).unwrap(), 1);
    }

    #[test]
    fn missing_model_is_not_available() {
        assert!(!KokoroGerman::is_available(Path::new("/tmp/missing-kokoro")));
    }
}
