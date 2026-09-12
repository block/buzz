//! Bounded ACP WAV <-> Realtime PCM boundary. No file or URI fetching.
use crate::types::AgentError;
use base64::{engine::general_purpose::STANDARD, Engine};

pub(crate) const MAX_PCM: usize = 24_000 * 2 * 30;
fn invalid() -> AgentError {
    AgentError::InvalidParams("realtime audio: expected bounded mono PCM16 WAV at 24000 Hz".into())
}

pub(crate) fn decode_wav(data: &str, mime: &str) -> Result<Vec<u8>, AgentError> {
    if mime != "audio/wav" || data.len() > (MAX_PCM + 4096).div_ceil(3) * 4 {
        return Err(invalid());
    }
    let wav = STANDARD.decode(data).map_err(|_| invalid())?;
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let u32_at =
        |i: usize| u32::from_le_bytes([wav[i], wav[i + 1], wav[i + 2], wav[i + 3]]) as usize;
    if u32_at(4) != wav.len() - 8 {
        return Err(invalid());
    }
    let mut offset = 12;
    let mut format = false;
    let mut pcm = None;
    while offset + 8 <= wav.len() {
        let size = u32_at(offset + 4);
        let start = offset + 8;
        if size > wav.len() - start {
            return Err(invalid());
        }
        let chunk = &wav[start..start + size];
        match &wav[offset..offset + 4] {
            b"fmt " => {
                if format
                    || size != 16
                    || chunk[..2] != [1, 0]
                    || chunk[2..4] != [1, 0]
                    || chunk[4..8] != 24000u32.to_le_bytes()
                    || chunk[8..12] != 48000u32.to_le_bytes()
                    || chunk[12..14] != [2, 0]
                    || chunk[14..16] != [16, 0]
                {
                    return Err(invalid());
                }
                format = true;
            }
            b"data" => {
                if pcm.is_some() || !(4800..=MAX_PCM).contains(&size) || size % 2 != 0 {
                    return Err(invalid());
                }
                pcm = Some(chunk.to_vec());
            }
            _ => {}
        }
        offset = start + size + (size & 1);
    }
    if !format || offset != wav.len() {
        return Err(invalid());
    }
    pcm.ok_or_else(invalid)
}

pub(crate) fn encode_wav(pcm: &[u8]) -> Result<String, AgentError> {
    if pcm.is_empty() || pcm.len() > MAX_PCM || !pcm.len().is_multiple_of(2) {
        return Err(invalid());
    }
    let size = pcm.len() as u32; // bounded above
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(size + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&[1, 0, 1, 0]);
    wav.extend_from_slice(&24000u32.to_le_bytes());
    wav.extend_from_slice(&48000u32.to_le_bytes());
    wav.extend_from_slice(&[2, 0, 16, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&size.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(STANDARD.encode(wav))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_wav_roundtrip_and_malformed_headers() {
        let pcm = vec![0; 4800];
        let encoded = encode_wav(&pcm).unwrap();
        assert_eq!(decode_wav(&encoded, "audio/wav").unwrap(), pcm);
        assert!(decode_wav(&encoded, "audio/mp3").is_err());
        let bytes = STANDARD.decode(encoded).unwrap();
        for offset in [0, 4, 8, 16, 20, 22, 24, 28, 32, 34, 40] {
            let mut bad = bytes.clone();
            bad[offset] ^= 255;
            assert!(
                decode_wav(&STANDARD.encode(bad), "audio/wav").is_err(),
                "offset {offset}"
            );
        }
        for len in [0, 1, 12, 43, bytes.len() - 1] {
            assert!(decode_wav(&STANDARD.encode(&bytes[..len]), "audio/wav").is_err());
        }
    }
}
