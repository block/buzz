use reqwest::blocking::{multipart, Client};
use serde::Deserialize;
use std::{net::IpAddr, time::Duration};
use url::Url;

const DECODE_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) struct OpenAiSttConfig {
    endpoint: Url,
    api_key: String,
    model: String,
    language: String,
    prompt: String,
}

impl OpenAiSttConfig {
    pub(super) fn from_env() -> Result<Option<Self>, String> {
        let endpoint = match std::env::var("BUZZ_STT_OPENAI_URL") {
            Ok(endpoint) if !endpoint.trim().is_empty() => endpoint,
            _ => return Ok(None),
        };
        let api_key = std::env::var("BUZZ_STT_OPENAI_API_KEY")
            .map_err(|_| "BUZZ_STT_OPENAI_API_KEY is required when local STT is configured")?;
        let model = std::env::var("BUZZ_STT_OPENAI_MODEL")
            .unwrap_or_else(|_| "whisper-large-v3-turbo-asr-fp16".into());
        let language = std::env::var("BUZZ_STT_OPENAI_LANGUAGE").unwrap_or_else(|_| "tr".into());
        let prompt = std::env::var("BUZZ_STT_OPENAI_PROMPT").unwrap_or_default();
        Self::new(endpoint, api_key, model, language, prompt).map(Some)
    }

    pub(super) fn new(
        endpoint: String,
        api_key: String,
        model: String,
        language: String,
        prompt: String,
    ) -> Result<Self, String> {
        let endpoint =
            Url::parse(&endpoint).map_err(|error| format!("invalid STT URL: {error}"))?;
        let is_loopback = match endpoint.host_str() {
            Some("localhost") => true,
            Some(host) => host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback()),
            None => false,
        };
        if endpoint.scheme() != "http" || !is_loopback {
            return Err("OpenAI-compatible STT endpoint must use HTTP on loopback".into());
        }
        if api_key.is_empty() || model.is_empty() || language.is_empty() {
            return Err("OpenAI-compatible STT key, model, and language are required".into());
        }
        Ok(Self {
            endpoint,
            api_key,
            model,
            language,
            prompt,
        })
    }
}

pub(super) fn is_configured() -> Result<bool, String> {
    Ok(OpenAiSttConfig::from_env()?.is_some())
}

pub(super) struct OpenAiSttDecoder {
    client: Client,
    config: OpenAiSttConfig,
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

impl OpenAiSttDecoder {
    pub(super) fn new(config: OpenAiSttConfig) -> Result<Self, String> {
        Self::new_with_timeout(config, DECODE_TIMEOUT)
    }

    fn new_with_timeout(config: OpenAiSttConfig, timeout: Duration) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| format!("failed to create STT HTTP client: {error}"))?;
        Ok(Self { client, config })
    }

    pub(super) fn decode(&self, speech: &[f32]) -> Result<String, String> {
        let file = multipart::Part::bytes(encode_pcm16_wav(speech))
            .file_name("huddle.wav")
            .mime_str("audio/wav")
            .map_err(|error| format!("failed to create STT audio part: {error}"))?;
        let mut form = multipart::Form::new()
            .part("file", file)
            .text("model", self.config.model.clone())
            .text("language", self.config.language.clone());
        if !self.config.prompt.is_empty() {
            form = form.text("prompt", self.config.prompt.clone());
        }
        let response = self
            .client
            .post(self.config.endpoint.clone())
            .bearer_auth(&self.config.api_key)
            .multipart(form)
            .send()
            .map_err(|error| format!("STT request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("STT endpoint rejected request: {error}"))?
            .json::<TranscriptionResponse>()
            .map_err(|error| format!("invalid STT response: {error}"))?;
        Ok(response.text.trim().to_string())
    }
}

fn encode_pcm16_wav(speech: &[f32]) -> Vec<u8> {
    let data_length = speech.len().saturating_mul(2).min(u32::MAX as usize) as u32;
    let mut wav = Vec::with_capacity(44 + data_length as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32.saturating_add(data_length)).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&32_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_length.to_le_bytes());
    for sample in speech.iter().take((data_length / 2) as usize) {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::{OpenAiSttConfig, OpenAiSttDecoder};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn sends_turkish_pcm_to_loopback_openai_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let (request_tx, request_rx) = mpsc::channel();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("read request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let header_end = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .expect("header terminator")
                        + 4;
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .expect("content length");
                    if request.len() >= header_end + content_length {
                        break;
                    }
                }
            }
            request_tx.send(request).expect("capture request");
            let body = r#"{"text":"Ananı sikerim, amına koyayım"}"#.as_bytes();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write response headers");
            stream.write_all(body).expect("write response body");
        });

        let config = OpenAiSttConfig::new(
            format!("http://{address}/v1/audio/transcriptions"),
            "test-key".into(),
            "whisper-large-v3-turbo-asr-fp16".into(),
            "tr".into(),
            "amına koyayım, ananı sikerim, ibne".into(),
        )
        .expect("valid loopback config");
        let decoder = OpenAiSttDecoder::new(config).expect("decoder");

        let text = decoder
            .decode(&vec![0.25; 16_000])
            .expect("transcription response");

        assert_eq!(text, "Ananı sikerim, amına koyayım");
        let request = request_rx.recv().expect("captured request");
        let request = String::from_utf8_lossy(&request);
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-key"));
        assert!(request.contains("whisper-large-v3-turbo-asr-fp16"));
        assert!(request.contains("name=\"language\""));
        assert!(request.contains("\r\n\r\ntr\r\n"));
        assert!(request.contains("amına koyayım, ananı sikerim, ibne"));
        assert!(request
            .as_bytes()
            .windows(4)
            .any(|window| window == b"RIFF"));
        server.join().expect("server thread");
    }

    #[test]
    fn rejects_non_loopback_endpoint() {
        let result = OpenAiSttConfig::new(
            "https://api.openai.com/v1/audio/transcriptions".into(),
            "test-key".into(),
            "whisper-large-v3-turbo-asr-fp16".into(),
            "tr".into(),
            String::new(),
        );
        let error = match result {
            Ok(_) => panic!("remote audio endpoint must be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("loopback"));
    }

    #[test]
    fn runaway_decode_times_out_without_blocking_the_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            thread::sleep(Duration::from_millis(200));
            let body = br#"{"text":"late transcript"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        });
        let config = OpenAiSttConfig::new(
            format!("http://{address}/v1/audio/transcriptions"),
            "test-key".into(),
            "whisper-large-v3-turbo-asr-fp16".into(),
            "tr".into(),
            String::new(),
        )
        .expect("valid loopback config");
        let decoder =
            OpenAiSttDecoder::new_with_timeout(config, Duration::from_millis(50)).expect("decoder");

        let started = Instant::now();
        let error = decoder
            .decode(&vec![0.25; 16_000])
            .expect_err("slow decode must time out");

        assert!(started.elapsed() < Duration::from_millis(150));
        assert!(error.contains("STT request failed"), "{error}");
        server.join().expect("server thread");
    }
}
