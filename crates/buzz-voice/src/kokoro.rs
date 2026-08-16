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
pub const KOKORO_STYLE_FRAMES: usize = 510;
pub const KOKORO_STYLE_DIM: usize = 256;
pub const KOKORO_STYLE_BYTES: usize = KOKORO_STYLE_FRAMES * KOKORO_STYLE_DIM * 4;

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
            && model_dir.join("espeak-ng-data").join("phontab").is_file()
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
        ])
        .ok_or_else(|| {
            "Kokoro espeak-ng-data is missing. Download German speech models and try again."
                .to_string()
        })?;
        let kokoro = OfflineTtsKokoroModelConfig {
            model: Some(model.to_string_lossy().into_owned()),
            voices: Some(voices_bin.to_string_lossy().into_owned()),
            tokens: Some(tokens.to_string_lossy().into_owned()),
            data_dir: Some(data_dir.to_string_lossy().into_owned()),
            lang: Some("de".into()),
            length_scale: 1.0,
            ..Default::default()
        };
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
        let victoria_available = voices_bin
            .metadata()
            .map(|meta| meta.len() as usize >= KOKORO_STYLE_BYTES * 2)
            .unwrap_or(false)
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

pub fn install_voices_bin(model_dir: &Path) -> Result<PathBuf, String> {
    ensure_voices_bin(model_dir)
}

fn ensure_voices_bin(model_dir: &Path) -> Result<PathBuf, String> {
    let voices_bin = model_dir.join("voices.bin");
    let martin = if voices_bin.is_file() {
        let existing = std::fs::read(&voices_bin).map_err(|e| format!("read voices.bin: {e}"))?;
        if existing.len() >= KOKORO_STYLE_BYTES {
            existing[..KOKORO_STYLE_BYTES].to_vec()
        } else {
            return Err("voices.bin is smaller than one Kokoro speaker".into());
        }
    } else {
        let npz = model_dir.join("voices-martin.npz");
        if !npz.is_file() {
            return Err("Kokoro voices.bin is missing".into());
        }
        style_payload_from_file(&npz)?
    };
    let mut styles = vec![martin];
    let victoria_pt = model_dir.join("victoria.pt");
    if victoria_pt.is_file() {
        match style_payload_from_file(&victoria_pt) {
            Ok(victoria) => styles.push(victoria),
            Err(error) => {
                return Err(format!("Victoria voice could not be installed: {error}"));
            }
        }
    }
    let merged = merge_speaker_styles(&styles)?;
    std::fs::write(&voices_bin, merged).map_err(|e| format!("write voices.bin: {e}"))?;
    Ok(voices_bin)
}

pub fn merge_speaker_styles(styles: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if styles.is_empty() {
        return Err("at least one Kokoro speaker style is required".into());
    }
    let mut out = Vec::with_capacity(styles.len() * KOKORO_STYLE_BYTES);
    for style in styles {
        if style.len() != KOKORO_STYLE_BYTES {
            return Err(format!(
                "Kokoro style must be {KOKORO_STYLE_BYTES} bytes, got {}",
                style.len()
            ));
        }
        out.extend_from_slice(style);
    }
    Ok(out)
}

pub fn style_payload_from_file(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return style_payload_from_zip(&bytes);
    }
    extract_style_payload(&bytes)
}

fn style_payload_from_zip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("style archive is not a zip: {e}"))?;
    let mut fallback = None;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("read style zip entry: {e}"))?;
        let mut data = Vec::new();
        std::io::copy(&mut entry, &mut data).map_err(|e| format!("read style zip bytes: {e}"))?;
        if data.len() == KOKORO_STYLE_BYTES {
            return Ok(data);
        }
        if fallback.is_none() {
            if let Ok(payload) = extract_style_payload(&data) {
                fallback = Some(payload);
            }
        }
    }
    fallback.ok_or_else(|| "style archive contained no 510x256 f32 speaker".to_string())
}

fn extract_style_payload(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() == KOKORO_STYLE_BYTES {
        return Ok(bytes.to_vec());
    }
    if bytes.len() >= 10 && &bytes[0..6] == b"\x93NUMPY" {
        let payload = skip_npy_header(bytes)?;
        if payload.len() != KOKORO_STYLE_BYTES {
            return Err(format!(
                "npy style payload must be {KOKORO_STYLE_BYTES} bytes, got {}",
                payload.len()
            ));
        }
        return Ok(payload.to_vec());
    }
    Err(format!(
        "unsupported Kokoro style artifact ({} bytes)",
        bytes.len()
    ))
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

    #[test]
    fn merges_martin_and_victoria_styles() {
        let martin = vec![1u8; KOKORO_STYLE_BYTES];
        let victoria = vec![2u8; KOKORO_STYLE_BYTES];
        let merged = merge_speaker_styles(&[martin.clone(), victoria.clone()]).unwrap();
        assert_eq!(merged.len(), KOKORO_STYLE_BYTES * 2);
        assert_eq!(&merged[..KOKORO_STYLE_BYTES], martin);
        assert_eq!(&merged[KOKORO_STYLE_BYTES..], victoria);
    }

    #[test]
    fn extracts_raw_and_npy_style_payloads() {
        let raw = vec![7u8; KOKORO_STYLE_BYTES];
        assert_eq!(extract_style_payload(&raw).unwrap(), raw);

        let mut npy = b"\x93NUMPY\x01\x00".to_vec();
        npy.extend_from_slice(&(20u16).to_le_bytes());
        npy.extend_from_slice(&[b' '; 20]);
        npy.extend_from_slice(&raw);
        assert_eq!(extract_style_payload(&npy).unwrap(), raw);
    }

    #[test]
    fn extracts_victoria_pt_zip_payload() {
        let style = vec![9u8; KOKORO_STYLE_BYTES];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("victoria.pt");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(
                "victoria_ep2/data/0",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            use std::io::Write;
            zip.write_all(&style).unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(style_payload_from_file(&path).unwrap(), style);
    }
}
