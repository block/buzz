//! Kokoro German TTS via ONNX Runtime.
//!
//! Martin/Victoria ONNX graphs are kokoro-onnx exports, not sherpa-onnx TTS
//! models. Inference is tokens + style + speed → waveform at 24 kHz.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use ort::session::Session;
use ort::value::Tensor;

use crate::german_normalize::normalize_german_text;

pub const KOKORO_SAMPLE_RATE: u32 = 24_000;
pub const KOKORO_STYLE_FRAMES: usize = 510;
pub const KOKORO_STYLE_DIM: usize = 256;
pub const KOKORO_STYLE_BYTES: usize = KOKORO_STYLE_FRAMES * KOKORO_STYLE_DIM * 4;
const KOKORO_MAX_PHONEMES: usize = 510;

const KOKORO_IPA_VOCAB: &[(char, i64)] = &[
    ('\u{003b}', 1),
    ('\u{003a}', 2),
    ('\u{002c}', 3),
    ('\u{002e}', 4),
    ('\u{0021}', 5),
    ('\u{003f}', 6),
    ('\u{2014}', 9),
    ('\u{2026}', 10),
    ('\u{0022}', 11),
    ('\u{0028}', 12),
    ('\u{0029}', 13),
    ('\u{201c}', 14),
    ('\u{201d}', 15),
    ('\u{0020}', 16),
    ('\u{0303}', 17),
    ('\u{02a3}', 18),
    ('\u{02a5}', 19),
    ('\u{02a6}', 20),
    ('\u{02a8}', 21),
    ('\u{1d5d}', 22),
    ('\u{ab67}', 23),
    ('\u{0041}', 24),
    ('\u{0049}', 25),
    ('\u{004f}', 31),
    ('\u{0051}', 33),
    ('\u{0053}', 35),
    ('\u{0054}', 36),
    ('\u{0057}', 39),
    ('\u{0059}', 41),
    ('\u{1d4a}', 42),
    ('\u{0061}', 43),
    ('\u{0062}', 44),
    ('\u{0063}', 45),
    ('\u{0064}', 46),
    ('\u{0065}', 47),
    ('\u{0066}', 48),
    ('\u{0068}', 50),
    ('\u{0069}', 51),
    ('\u{006a}', 52),
    ('\u{006b}', 53),
    ('\u{006c}', 54),
    ('\u{006d}', 55),
    ('\u{006e}', 56),
    ('\u{006f}', 57),
    ('\u{0070}', 58),
    ('\u{0071}', 59),
    ('\u{0072}', 60),
    ('\u{0073}', 61),
    ('\u{0074}', 62),
    ('\u{0075}', 63),
    ('\u{0076}', 64),
    ('\u{0077}', 65),
    ('\u{0078}', 66),
    ('\u{0079}', 67),
    ('\u{007a}', 68),
    ('\u{0251}', 69),
    ('\u{0250}', 70),
    ('\u{0252}', 71),
    ('\u{00e6}', 72),
    ('\u{03b2}', 75),
    ('\u{0254}', 76),
    ('\u{0255}', 77),
    ('\u{00e7}', 78),
    ('\u{0256}', 80),
    ('\u{00f0}', 81),
    ('\u{02a4}', 82),
    ('\u{0259}', 83),
    ('\u{025a}', 85),
    ('\u{025b}', 86),
    ('\u{025c}', 87),
    ('\u{025f}', 90),
    ('\u{0261}', 92),
    ('\u{0265}', 99),
    ('\u{0268}', 101),
    ('\u{026a}', 102),
    ('\u{029d}', 103),
    ('\u{026f}', 110),
    ('\u{0270}', 111),
    ('\u{014b}', 112),
    ('\u{0273}', 113),
    ('\u{0272}', 114),
    ('\u{0274}', 115),
    ('\u{00f8}', 116),
    ('\u{0278}', 118),
    ('\u{03b8}', 119),
    ('\u{0153}', 120),
    ('\u{0279}', 123),
    ('\u{027e}', 125),
    ('\u{027b}', 126),
    ('\u{0281}', 128),
    ('\u{027d}', 129),
    ('\u{0282}', 130),
    ('\u{0283}', 131),
    ('\u{0288}', 132),
    ('\u{02a7}', 133),
    ('\u{028a}', 135),
    ('\u{028b}', 136),
    ('\u{028c}', 138),
    ('\u{0263}', 139),
    ('\u{0264}', 140),
    ('\u{03c7}', 142),
    ('\u{028e}', 143),
    ('\u{0292}', 147),
    ('\u{0294}', 148),
    ('\u{02c8}', 156),
    ('\u{02cc}', 157),
    ('\u{02d0}', 158),
    ('\u{02b0}', 162),
    ('\u{02b2}', 164),
    ('\u{2193}', 169),
    ('\u{2192}', 171),
    ('\u{2197}', 172),
    ('\u{2198}', 173),
    ('\u{1d7b}', 177),
];

pub struct KokoroGerman {
    session: Session,
    styles: Vec<f32>,
    speaker_count: usize,
    espeak_data_parent: PathBuf,
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

    pub fn is_runtime_usable(model_dir: &Path) -> bool {
        Self::is_available(model_dir)
    }

    pub fn load(model_dir: &Path) -> Result<Self, String> {
        eprintln!("buzz-desktop: Loading Kokoro...");
        let voices_bin = ensure_voices_bin(model_dir)?;
        let model = model_dir.join("model.onnx");
        if !model.is_file() {
            return Err(format!(
                "Kokoro model files are missing in {}",
                model_dir.display()
            ));
        }
        if first_existing(&[
            model_dir.join("espeak-ng-data"),
            model_dir.join("espeak-ng-data-dir"),
        ])
        .is_none()
        {
            return Err(
                "Kokoro espeak-ng-data is missing. Download German speech models and try again."
                    .into(),
            );
        }
        let session = Session::builder()
            .map_err(|error| format!("create Kokoro ONNX session builder: {error}"))?
            .with_intra_threads(2)
            .map_err(|error| format!("configure Kokoro ONNX threads: {error}"))?
            .with_inter_threads(1)
            .map_err(|error| format!("configure Kokoro ONNX inter-op: {error}"))?
            .commit_from_file(&model)
            .map_err(|error| format!("load {}: {error}", model.display()))?;
        let raw = std::fs::read(&voices_bin).map_err(|error| format!("read voices.bin: {error}"))?;
        if raw.len() < KOKORO_STYLE_BYTES || !raw.len().is_multiple_of(KOKORO_STYLE_BYTES) {
            return Err(format!(
                "voices.bin must be a multiple of {KOKORO_STYLE_BYTES} bytes, got {}",
                raw.len()
            ));
        }
        let speaker_count = raw.len() / KOKORO_STYLE_BYTES;
        let mut styles = Vec::with_capacity(raw.len() / 4);
        for chunk in raw.chunks_exact(4) {
            styles.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        eprintln!("buzz-desktop: Kokoro ready");
        Ok(Self {
            session,
            styles,
            speaker_count,
            espeak_data_parent: model_dir.to_path_buf(),
            victoria_available: speaker_count > 1,
        })
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        voice: &str,
        speed: f32,
    ) -> Result<(Vec<f32>, u32), String> {
        if text.trim().is_empty() {
            return Ok((Vec::new(), KOKORO_SAMPLE_RATE));
        }
        let sid = voice_sid(voice, self.victoria_available)? as usize;
        eprintln!("buzz-desktop: Voice: {}", voice_label(voice));
        let normalized = normalize_german_text(text);
        let phonemes = phonemize_german(&normalized, &self.espeak_data_parent)?;
        let mut samples = Vec::new();
        for chunk in split_phonemes(&phonemes) {
            samples.extend(self.infer_chunk(&chunk, sid, speed.clamp(0.5, 2.0))?);
        }
        Ok((samples, KOKORO_SAMPLE_RATE))
    }

    pub fn stop(&self) {}

    pub fn unload(self) {}
}

fn phoneme_id(ch: char) -> Option<i64> {
    KOKORO_IPA_VOCAB
        .iter()
        .find_map(|(glyph, id)| (*glyph == ch).then_some(*id))
}

fn tokenize_phonemes(phonemes: &str) -> Vec<i64> {
    phonemes.chars().filter_map(phoneme_id).collect()
}

fn split_phonemes(phonemes: &str) -> Vec<String> {
    let chars: Vec<char> = phonemes.chars().collect();
    if chars.len() <= KOKORO_MAX_PHONEMES {
        return vec![phonemes.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + KOKORO_MAX_PHONEMES).min(chars.len());
        if end < chars.len() {
            if let Some(split_at) = chars[start..end].iter().rposition(|ch| *ch == ' ') {
                end = start + split_at;
            }
        }
        if end <= start {
            end = (start + KOKORO_MAX_PHONEMES).min(chars.len());
        }
        chunks.push(chars[start..end].iter().collect());
        start = end;
    }
    chunks
}

fn phonemize_german(text: &str, data_parent: &Path) -> Result<String, String> {
    let output = Command::new("espeak-ng")
        .args(["-v", "de", "--ipa=3", "-q"])
        .arg(format!("--path={}", data_parent.display()))
        .arg(text)
        .output()
        .or_else(|_| {
            Command::new("espeak")
                .args(["-v", "de", "--ipa=3", "-q"])
                .arg(format!("--path={}", data_parent.display()))
                .arg(text)
                .output()
        })
        .map_err(|error| {
            format!("espeak-ng is required for German Kokoro TTS: {error}")
        })?;
    if !output.status.success() {
        return Err(format!(
            "espeak-ng failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut phonemes: String = String::from_utf8_lossy(&output.stdout)
        .chars()
        .filter(|ch| *ch == ' ' || phoneme_id(*ch).is_some())
        .collect();
    phonemes = phonemes.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(mark) = text.trim().chars().last() {
        if matches!(mark, '.' | '!' | '?' | ',' | ';' | ':') && !phonemes.ends_with(mark) {
            phonemes.push(mark);
        }
    }
    if tokenize_phonemes(&phonemes).is_empty() {
        return Err(format!("no Kokoro phonemes for {text:?}"));
    }
    Ok(phonemes)
}

impl KokoroGerman {
    fn infer_chunk(
        &mut self,
        phonemes: &str,
        speaker: usize,
        speed: f32,
    ) -> Result<Vec<f32>, String> {
        let tokens = tokenize_phonemes(phonemes);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        if speaker >= self.speaker_count {
            return Err("Kokoro speaker is not installed".into());
        }
        let style_index = tokens.len().min(KOKORO_STYLE_FRAMES).saturating_sub(1);
        let style_offset =
            speaker * KOKORO_STYLE_FRAMES * KOKORO_STYLE_DIM + style_index * KOKORO_STYLE_DIM;
        let style = self.styles[style_offset..style_offset + KOKORO_STYLE_DIM].to_vec();
        let mut padded = Vec::with_capacity(tokens.len() + 2);
        padded.push(0);
        padded.extend_from_slice(&tokens);
        padded.push(0);
        let token_len = padded.len() as i64;
        let tokens = Tensor::from_array((vec![1_i64, token_len], padded))
            .map_err(|error| format!("create Kokoro token tensor: {error}"))?;
        let style = Tensor::from_array((vec![1_i64, KOKORO_STYLE_DIM as i64], style))
            .map_err(|error| format!("create Kokoro style tensor: {error}"))?;
        let speed = Tensor::from_array((vec![1_i64], vec![speed]))
            .map_err(|error| format!("create Kokoro speed tensor: {error}"))?;
        let outputs = self
            .session
            .run(ort::inputs!["tokens" => tokens, "style" => style, "speed" => speed])
            .map_err(|error| format!("run Kokoro: {error}"))?;
        let (_, waveform) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("extract Kokoro waveform: {error}"))?;
        Ok(waveform.to_vec())
    }
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

fn onnx_has_sherpa_kokoro_metadata(model: &Path) -> bool {
    onnx_region_contains(model, b"sample_rate")
}

fn onnx_region_contains(path: &Path, needle: &[u8]) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    const WINDOW: usize = 128 * 1024;
    let mut buf = vec![0u8; WINDOW];
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    if contains_bytes(&buf[..n], needle) {
        return true;
    }
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if meta.len() <= WINDOW as u64 {
        return false;
    }
    let Ok(tail) = i64::try_from(WINDOW) else {
        return false;
    };
    if file.seek(SeekFrom::End(-tail)).is_err() {
        return false;
    }
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    contains_bytes(&buf[..n], needle)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
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
    fn installed_martin_onnx_is_runtime_usable() {
        let dir = Path::new("/Users/cyberblade/.buzz/models/kokoro-de");
        if !KokoroGerman::is_available(dir) {
            return;
        }
        assert!(
            KokoroGerman::is_runtime_usable(dir),
            "Martin ONNX should run via ONNX Runtime without sherpa metadata"
        );
    }

    #[test]
    fn tokenizes_german_espeak_phonemes() {
        let tokens = tokenize_phonemes("hˈaloː vˈɛlt.");
        assert_eq!(tokens, vec![50, 156, 43, 54, 57, 158, 16, 64, 156, 86, 54, 62, 4]);
    }

    #[test]
    fn load_installed_kokoro_models() {
        let dir = Path::new("/Users/cyberblade/.buzz/models/kokoro-de");
        if !KokoroGerman::is_available(dir) {
            return;
        }
        let mut engine =
            KokoroGerman::load(dir).expect("Martin ONNX should load via ONNX Runtime");
        let (samples, sample_rate) = engine
            .synthesize("Hallo.", "martin", 1.0)
            .expect("Martin should speak German");
        assert_eq!(sample_rate, KOKORO_SAMPLE_RATE);
        assert!(samples.len() > 8_000, "got {} samples", samples.len());
        assert!(samples.iter().any(|sample| sample.abs() > 0.01));
    }

    #[test]
    fn rejects_onnx_without_sherpa_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"pytorch-onnx-without-sherpa-keys").unwrap();
        assert!(!onnx_has_sherpa_kokoro_metadata(&model));
        std::fs::write(&model, b"sherpa-onnx kokoro sample_rate=24000").unwrap();
        assert!(onnx_has_sherpa_kokoro_metadata(&model));
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
