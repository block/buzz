//! Device-local Pocket reference voice validation, canonicalization, and storage.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymphoniaError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

const MAX_SOURCE_BYTES: u64 = 25 * 1024 * 1024;
const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 96_000;
const MIN_DURATION_SECONDS: f64 = 2.0;
const MAX_DURATION_SECONDS: f64 = 30.0;
pub const CANONICAL_SAMPLE_RATE: u32 = 32_000;
const REGISTRY_VERSION: u32 = 1;
const REGISTRY_FILE: &str = "registry.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedVoice {
    pub key: String,
    pub display_name: String,
    pub content_hash: String,
    pub file_name: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedVoiceRegistry {
    version: u32,
    voices: Vec<ImportedVoice>,
}

#[derive(Clone, Debug)]
pub struct PocketVoiceLibrary {
    root: PathBuf,
}

impl PocketVoiceLibrary {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn registry_path(&self) -> PathBuf {
        self.root.join(REGISTRY_FILE)
    }

    pub fn load(&self) -> Result<Vec<ImportedVoice>, String> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("could not read imported voices: {error}"))?;
        let registry: ImportedVoiceRegistry = serde_json::from_slice(&bytes)
            .map_err(|error| format!("imported voice registry is invalid: {error}"))?;
        if registry.version > REGISTRY_VERSION {
            return Err(format!(
                "imported voice registry version {} is newer than this Buzz build supports",
                registry.version
            ));
        }
        Ok(registry
            .voices
            .into_iter()
            .filter(valid_identity)
            .filter(|voice| self.resolve_file(voice).is_ok())
            .collect())
    }

    fn save(&self, voices: &[ImportedVoice]) -> Result<(), String> {
        ensure_storage_dir(&self.root)?;
        let payload = serde_json::to_vec_pretty(&ImportedVoiceRegistry {
            version: REGISTRY_VERSION,
            voices: voices.to_vec(),
        })
        .map_err(|error| format!("could not encode imported voice registry: {error}"))?;
        atomic_write_restricted(&self.registry_path(), &payload)
            .map_err(|error| format!("could not save imported voice registry: {error}"))
    }

    pub fn resolve_file(&self, voice: &ImportedVoice) -> Result<PathBuf, String> {
        if !valid_identity(voice) {
            return Err("Imported voice registry contains an invalid file identity".to_string());
        }
        let path = self.root.join(&voice.file_name);
        if !is_regular_file_without_symlink(&path) {
            return Err(format!("Imported voice {} is missing", voice.display_name));
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("could not verify imported voice: {error}"))?;
        if hex::encode(Sha256::digest(bytes)) != voice.content_hash {
            return Err(format!(
                "Imported voice {} does not match its content identity",
                voice.display_name
            ));
        }
        Ok(path)
    }

    pub fn find(&self, key: &str) -> Result<Option<ImportedVoice>, String> {
        Ok(self.load()?.into_iter().find(|voice| voice.key == key))
    }

    pub fn import_path(&self, source: &Path) -> Result<ImportedVoice, String> {
        let metadata = fs::metadata(source)
            .map_err(|error| format!("could not inspect selected audio: {error}"))?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err("Voice audio must be 25 MB or smaller".to_string());
        }
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| "Voice audio must have a supported file extension".to_string())?;
        let samples = if extension == "wav" {
            let source_bytes = fs::read(source)
                .map_err(|error| format!("could not read selected audio: {error}"))?;
            decode_wav(&source_bytes)?
        } else {
            decode_media(source, &extension)?
        };
        let canonical_samples = resample_linear(&samples.samples, samples.sample_rate);
        let canonical = encode_pcm16_wav(&canonical_samples, CANONICAL_SAMPLE_RATE);
        let hash = hex::encode(Sha256::digest(&canonical));
        let key = format!("pocket:imported:{hash}");
        let file_name = format!("{hash}.wav");
        let display_name = source
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("Imported voice")
            .chars()
            .take(80)
            .collect::<String>();

        ensure_storage_dir(&self.root)?;
        let file_path = self.root.join(&file_name);
        let file_created = !file_path.exists();
        if file_created {
            atomic_write_restricted(&file_path, &canonical)
                .map_err(|error| format!("could not save imported voice audio: {error}"))?;
        } else {
            if !is_regular_file_without_symlink(&file_path) {
                return Err("Imported voice storage contains an unsafe file entry".to_string());
            }
            let existing = fs::read(&file_path)
                .map_err(|error| format!("could not verify imported voice audio: {error}"))?;
            if hex::encode(Sha256::digest(&existing)) != hash {
                return Err("Imported voice storage contains mismatched audio data".to_string());
            }
        }

        let mut imported = ImportedVoice {
            key,
            display_name,
            content_hash: hash,
            file_name,
        };
        let mut voices = self.load()?;
        if let Some(existing) = voices
            .iter()
            .find(|voice| voice.content_hash == imported.content_hash)
        {
            imported = existing.clone();
        } else {
            voices.push(imported.clone());
        }
        if let Err(error) = self.save(&voices) {
            if file_created {
                let _ = fs::remove_file(&file_path);
            }
            return Err(error);
        }
        Ok(imported)
    }

    pub fn delete(&self, key: &str) -> Result<(), String> {
        let mut voices = self.load()?;
        let index = voices
            .iter()
            .position(|voice| voice.key == key)
            .ok_or_else(|| format!("Unknown imported voice: {key}"))?;
        let previous_voices = voices.clone();
        let removed = voices.remove(index);
        self.save(&voices)?;
        let path = self.root.join(removed.file_name);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                self.save(&previous_voices).map_err(|rollback_error| {
                    format!(
                        "Imported voice audio could not be deleted ({error}), and its registry \
                         entry could not be restored ({rollback_error})"
                    )
                })?;
                Err(format!(
                    "Imported voice audio could not be deleted: {error}"
                ))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PcmStats {
    pub sample_count: usize,
    pub sample_rate: u32,
    pub duration_seconds: f64,
    pub peak: f32,
    pub rms: f32,
    pub non_silent_samples: usize,
}

impl PcmStats {
    pub fn analyze(samples: &[f32], sample_rate: u32) -> Self {
        let peak = samples
            .iter()
            .filter(|sample| sample.is_finite())
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let square_sum = samples
            .iter()
            .filter(|sample| sample.is_finite())
            .map(|sample| sample * sample)
            .sum::<f32>();
        let rms = if samples.is_empty() {
            0.0
        } else {
            (square_sum / samples.len() as f32).sqrt()
        };
        Self {
            sample_count: samples.len(),
            sample_rate,
            duration_seconds: if sample_rate == 0 {
                0.0
            } else {
                samples.len() as f64 / f64::from(sample_rate)
            },
            peak,
            rms,
            non_silent_samples: samples
                .iter()
                .filter(|sample| sample.is_finite() && sample.abs() >= 0.001)
                .count(),
        }
    }

    pub fn is_non_silent(self) -> bool {
        self.peak >= 0.001 && self.rms >= 0.0001 && self.non_silent_samples > 0
    }
}

pub fn write_pcm16_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let bytes = encode_pcm16_wav(samples, sample_rate);
    fs::write(path, bytes).map_err(|error| format!("could not write PCM evidence: {error}"))
}

fn ensure_storage_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("could not create local voice storage: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not restrict local voice storage: {error}"))?;
    }
    Ok(())
}

fn atomic_write_restricted(path: &Path, payload: &[u8]) -> Result<(), String> {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut file = AtomicWriteFile::open(&resolved)
        .map_err(|error| format!("open {} for atomic write: {error}", resolved.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("set {} permissions: {error}", resolved.display()))?;
    }
    file.write_all(payload)
        .map_err(|error| format!("write {}: {error}", resolved.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", resolved.display()))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_identity(voice: &ImportedVoice) -> bool {
    valid_hash(&voice.content_hash)
        && voice.key == format!("pocket:imported:{}", voice.content_hash)
        && voice.file_name == format!("{}.wav", voice.content_hash)
}

fn is_regular_file_without_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
}

#[derive(Debug)]
struct DecodedAudio {
    sample_rate: u32,
    samples: Vec<f32>,
}

fn decode_wav(bytes: &[u8]) -> Result<DecodedAudio, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Selected file is not a valid RIFF/WAVE file".to_string());
    }
    let mut offset = 12usize;
    let mut format = None;
    let mut data = None;
    while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[offset..offset + 4];
        let size =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap_or([0; 4])) as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or("WAV chunk size overflow")?;
        if end > bytes.len() {
            return Err("Selected WAV contains a truncated chunk".to_string());
        }
        if id == b"fmt " {
            format = Some(&bytes[start..end]);
        } else if id == b"data" {
            data = Some(&bytes[start..end]);
        }
        offset = end + (size & 1);
    }
    let format = format.ok_or("Selected WAV has no format chunk")?;
    let data = data.ok_or("Selected WAV has no audio data")?;
    if format.len() < 16 {
        return Err("Selected WAV has an invalid format chunk".to_string());
    }
    let encoding = u16::from_le_bytes(format[0..2].try_into().unwrap_or([0; 2]));
    let encoding = if encoding == 0xfffe && format.len() >= 40 {
        u16::from_le_bytes(format[24..26].try_into().unwrap_or([0; 2]))
    } else {
        encoding
    };
    let channels = u16::from_le_bytes(format[2..4].try_into().unwrap_or([0; 2]));
    let sample_rate = u32::from_le_bytes(format[4..8].try_into().unwrap_or([0; 4]));
    let block_align = u16::from_le_bytes(format[12..14].try_into().unwrap_or([0; 2])) as usize;
    let bits = u16::from_le_bytes(format[14..16].try_into().unwrap_or([0; 2]));
    if channels == 0 || channels > 8 {
        return Err("Voice WAV must contain between 1 and 8 channels".to_string());
    }
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate) {
        return Err("Voice WAV sample rate must be between 8 and 96 kHz".to_string());
    }
    let bytes_per_sample = usize::from(bits.div_ceil(8));
    if block_align != bytes_per_sample * usize::from(channels)
        || block_align == 0
        || data.len() % block_align != 0
    {
        return Err("Voice WAV has invalid sample alignment".to_string());
    }
    if !matches!((encoding, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
        return Err("Voice WAV must contain PCM or 32-bit float audio".to_string());
    }
    let frames = data.len() / block_align;
    let duration = frames as f64 / f64::from(sample_rate);
    if !(MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&duration) {
        return Err("Voice WAV must be between 2 and 30 seconds long".to_string());
    }

    let mut samples = Vec::with_capacity(frames);
    for frame in data.chunks_exact(block_align) {
        let mut mono = 0.0_f32;
        for chunk in frame.chunks_exact(bytes_per_sample) {
            let sample = match (encoding, bits) {
                (1, 8) => (f32::from(chunk[0]) - 128.0) / 128.0,
                (1, 16) => f32::from(i16::from_le_bytes([chunk[0], chunk[1]])) / 32768.0,
                (1, 24) => {
                    let raw = i32::from_le_bytes([
                        chunk[0],
                        chunk[1],
                        chunk[2],
                        if chunk[2] & 0x80 == 0 { 0 } else { 0xff },
                    ]);
                    raw as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes(chunk.try_into().map_err(|_| "invalid PCM sample")?) as f32
                        / 2_147_483_648.0
                }
                (3, 32) => f32::from_le_bytes(
                    chunk
                        .try_into()
                        .map_err(|_| "invalid floating-point sample")?,
                ),
                _ => unreachable!(),
            };
            if !sample.is_finite() {
                return Err("Voice WAV contains non-finite samples".to_string());
            }
            mono += sample;
        }
        samples.push((mono / f32::from(channels)).clamp(-1.0, 1.0));
    }
    let stats = PcmStats::analyze(&samples, sample_rate);
    if !stats.is_non_silent() {
        return Err("Voice WAV is silent or too quiet to clone".to_string());
    }
    Ok(DecodedAudio {
        sample_rate,
        samples,
    })
}

fn decode_media(source: &Path, extension: &str) -> Result<DecodedAudio, String> {
    let supported = ["m4a", "mp3", "flac", "ogg", "oga", "aif", "aiff"];
    if !supported.contains(&extension) {
        return Err(format!(
            "Unsupported voice audio format .{extension}. Choose WAV, M4A, MP3, FLAC, OGG, or AIFF"
        ));
    }

    let file = fs::File::open(source)
        .map_err(|error| format!("could not read selected audio: {error}"))?;
    let media = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            media,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("could not recognize selected audio: {error}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "Selected audio has no decodable track".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("could not initialize audio decoder: {error}"))?;
    let mut sample_rate = None;
    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => {
                return Err("Selected audio changes format mid-stream".to_string());
            }
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(format!("could not read selected audio: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("could not decode selected audio: {error}")),
        };
        let spec = *decoded.spec();
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&spec.rate) {
            return Err("Voice audio sample rate must be between 8 and 96 kHz".to_string());
        }
        if sample_rate.is_some_and(|rate| rate != spec.rate) {
            return Err("Selected audio changes sample rate mid-stream".to_string());
        }
        sample_rate = Some(spec.rate);
        let channels = spec.channels.count();
        if channels == 0 || channels > 8 {
            return Err("Voice audio must contain between 1 and 8 channels".to_string());
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks_exact(channels) {
            let mono = frame.iter().copied().sum::<f32>() / channels as f32;
            if !mono.is_finite() {
                return Err("Voice audio contains non-finite samples".to_string());
            }
            samples.push(mono.clamp(-1.0, 1.0));
        }
        if samples.len() as f64 > MAX_DURATION_SECONDS * f64::from(spec.rate) {
            return Err("Voice audio must be between 2 and 30 seconds long".to_string());
        }
    }

    let sample_rate =
        sample_rate.ok_or_else(|| "Selected audio contains no samples".to_string())?;
    validate_decoded_audio(&samples, sample_rate)?;
    Ok(DecodedAudio {
        sample_rate,
        samples,
    })
}

fn validate_decoded_audio(samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let stats = PcmStats::analyze(samples, sample_rate);
    if !(MIN_DURATION_SECONDS..=MAX_DURATION_SECONDS).contains(&stats.duration_seconds) {
        return Err("Voice audio must be between 2 and 30 seconds long".to_string());
    }
    if !stats.is_non_silent() {
        return Err("Voice audio is silent or too quiet to clone".to_string());
    }
    Ok(())
}

fn resample_linear(samples: &[f32], source_rate: u32) -> Vec<f32> {
    if source_rate == CANONICAL_SAMPLE_RATE {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as u64 * u64::from(CANONICAL_SAMPLE_RATE)
        + u64::from(source_rate) / 2)
        / u64::from(source_rate)) as usize;
    (0..output_len)
        .map(|index| {
            let source = index as f64 * f64::from(source_rate) / f64::from(CANONICAL_SAMPLE_RATE);
            let left = source.floor() as usize;
            let fraction = (source - left as f64) as f32;
            let a = samples[left.min(samples.len() - 1)];
            let b = samples[(left + 1).min(samples.len() - 1)];
            a + (b - a) * fraction
        })
        .collect()
}

fn encode_pcm16_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(sample_rate: u32, seconds: usize, amplitude: f32) -> Vec<u8> {
        let samples = (0..sample_rate as usize * seconds)
            .map(|index| {
                amplitude
                    * (std::f32::consts::TAU * 220.0 * index as f32 / sample_rate as f32).sin()
            })
            .collect::<Vec<_>>();
        encode_pcm16_wav(&samples, sample_rate)
    }

    fn stereo_fixture(sample_rate: u32, seconds: usize, amplitude: f32) -> Vec<u8> {
        let mono = fixture(sample_rate, seconds, amplitude);
        let mono_data = &mono[44..];
        let mut stereo_data = Vec::with_capacity(mono_data.len() * 2);
        for sample in mono_data.chunks_exact(2) {
            stereo_data.extend_from_slice(sample);
            stereo_data.extend_from_slice(sample);
        }
        let mut stereo = mono[..44].to_vec();
        stereo[4..8].copy_from_slice(&(36 + stereo_data.len() as u32).to_le_bytes());
        stereo[22..24].copy_from_slice(&2_u16.to_le_bytes());
        stereo[28..32].copy_from_slice(&(sample_rate * 4).to_le_bytes());
        stereo[32..34].copy_from_slice(&4_u16.to_le_bytes());
        stereo[40..44].copy_from_slice(&(stereo_data.len() as u32).to_le_bytes());
        stereo.extend_from_slice(&stereo_data);
        stereo
    }

    #[test]
    fn imports_persists_reloads_and_deletes_canonical_voice() {
        let temp = tempfile::tempdir().expect("temp voice workspace");
        let source = temp.path().join("My voice.wav");
        fs::write(&source, fixture(44_100, 2, 0.5)).expect("write source");
        let library = PocketVoiceLibrary::new(temp.path().join("library"));

        let imported = library.import_path(&source).expect("import voice");
        assert!(imported.key.starts_with("pocket:imported:"));
        assert_eq!(imported.display_name, "My voice");

        let relaunched = PocketVoiceLibrary::new(library.root());
        assert_eq!(
            relaunched.load().expect("reload registry"),
            vec![imported.clone()]
        );
        let stored = relaunched
            .resolve_file(&imported)
            .expect("resolve stored voice");
        let decoded = decode_wav(&fs::read(&stored).expect("read stored voice"))
            .expect("decode canonical voice");
        assert_eq!(decoded.sample_rate, CANONICAL_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), CANONICAL_SAMPLE_RATE as usize * 2);

        assert_eq!(
            relaunched.import_path(&source).expect("idempotent import"),
            imported
        );
        assert_eq!(relaunched.load().expect("deduplicated registry").len(), 1);

        relaunched.delete(&imported.key).expect("delete voice");
        assert!(relaunched.load().expect("empty registry").is_empty());
        assert!(!stored.exists());
    }

    #[test]
    fn common_stereo_audio_is_downmixed_to_canonical_mono() {
        let temp = tempfile::tempdir().expect("temp voice workspace");
        let source = temp.path().join("stereo.wav");
        fs::write(&source, stereo_fixture(44_100, 2, 0.5)).expect("write stereo");
        let library = PocketVoiceLibrary::new(temp.path().join("library"));

        let imported = library.import_path(&source).expect("import stereo");
        let stored = library
            .resolve_file(&imported)
            .expect("resolve stored voice");
        let decoded = decode_wav(&fs::read(stored).expect("read stored voice"))
            .expect("decode canonical voice");
        assert_eq!(decoded.sample_rate, CANONICAL_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), CANONICAL_SAMPLE_RATE as usize * 2);
    }

    #[test]
    #[ignore = "requires BUZZ_VOICE_IMPORT_TEST_DIR with common-format fixtures"]
    fn imports_common_audio_format_fixtures() {
        let fixtures =
            PathBuf::from(std::env::var("BUZZ_VOICE_IMPORT_TEST_DIR").expect("fixture directory"));
        let temp = tempfile::tempdir().expect("temp voice workspace");
        let library = PocketVoiceLibrary::new(temp.path().join("library"));

        for file_name in [
            "voice.wav",
            "voice.m4a",
            "voice.mp3",
            "voice.flac",
            "voice.ogg",
            "voice.aiff",
        ] {
            let imported = library
                .import_path(&fixtures.join(file_name))
                .unwrap_or_else(|error| panic!("import {file_name}: {error}"));
            let stored = library
                .resolve_file(&imported)
                .unwrap_or_else(|error| panic!("resolve {file_name}: {error}"));
            let decoded = decode_wav(&fs::read(stored).expect("read canonical voice"))
                .expect("decode canonical voice");
            assert_eq!(decoded.sample_rate, CANONICAL_SAMPLE_RATE);
            assert!(decoded.samples.len() >= CANONICAL_SAMPLE_RATE as usize * 2);
        }
    }

    #[test]
    fn invalid_unsupported_and_silent_files_do_not_mutate_registry() {
        let temp = tempfile::tempdir().expect("temp voice workspace");
        let library = PocketVoiceLibrary::new(temp.path().join("library"));

        let garbage = temp.path().join("garbage.wav");
        fs::write(&garbage, b"not a wave").expect("write garbage");
        assert!(library
            .import_path(&garbage)
            .expect_err("garbage rejected")
            .contains("RIFF/WAVE"));

        let silent = temp.path().join("silent.wav");
        fs::write(&silent, fixture(32_000, 2, 0.0)).expect("write silence");
        assert!(library
            .import_path(&silent)
            .expect_err("silence rejected")
            .contains("silent"));

        let unsupported_container = temp.path().join("voice.txt");
        fs::write(&unsupported_container, b"not audio").expect("write unsupported container");
        assert!(library
            .import_path(&unsupported_container)
            .expect_err("container rejected")
            .contains("Unsupported voice audio format"));

        let mut unsupported = fixture(32_000, 2, 0.5);
        unsupported[20..22].copy_from_slice(&6_u16.to_le_bytes());
        let unsupported_path = temp.path().join("unsupported.wav");
        fs::write(&unsupported_path, unsupported).expect("write unsupported");
        assert!(library
            .import_path(&unsupported_path)
            .expect_err("unsupported rejected")
            .contains("PCM or 32-bit float"));

        assert!(library.load().expect("unchanged registry").is_empty());
    }

    #[test]
    fn pcm_analysis_distinguishes_signal_from_silence() {
        let signal = (0..24_000)
            .map(|index| (std::f32::consts::TAU * 440.0 * index as f32 / 24_000.0).sin() * 0.5)
            .collect::<Vec<_>>();
        let signal_stats = PcmStats::analyze(&signal, 24_000);
        assert!(signal_stats.is_non_silent());
        assert_eq!(signal_stats.duration_seconds, 1.0);
        assert!(signal_stats.peak > 0.49);
        assert!(signal_stats.rms > 0.3);

        let silence = vec![0.0; 24_000];
        assert!(!PcmStats::analyze(&silence, 24_000).is_non_silent());
    }
}
