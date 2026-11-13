//! Bounded SSE framing and provider response assembly, independent of ACP.
mod anthropic;
mod chat;
mod responses;

use reqwest::Response;
use serde_json::Value;
use tokio::sync::mpsc;

use super::MAX_LLM_RESPONSE_BYTES;
use crate::types::AgentError;

#[derive(Debug)]
pub(crate) enum LlmDelta {
    Text(String),
}

#[derive(Clone, Copy)]
pub(super) enum Protocol {
    Chat,
    Responses,
    Anthropic,
}

impl Protocol {
    pub(super) fn for_path(path: &str) -> Self {
        if path.ends_with("/responses") {
            Self::Responses
        } else if path.ends_with("/messages") {
            Self::Anthropic
        } else {
            Self::Chat
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct StreamRequest<'a> {
    protocol: Protocol,
    updates: &'a mpsc::Sender<LlmDelta>,
}

impl<'a> StreamRequest<'a> {
    pub(super) fn new(protocol: Protocol, updates: &'a mpsc::Sender<LlmDelta>) -> Self {
        Self { protocol, updates }
    }
}

pub(super) fn configure_request(body: &mut Value, protocol: Protocol, enabled: bool) {
    if enabled {
        body["stream"] = Value::Bool(true);
        if matches!(protocol, Protocol::Chat) {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
    }
}

pub(super) fn is_event_stream(response: &Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
        })
}

pub(super) fn error(message: impl core::fmt::Display) -> AgentError {
    let mut detail = message.to_string();
    let mut end = detail.len().min(super::MAX_LLM_ERROR_BODY_BYTES);
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail.truncate(end);
    AgentError::Llm(format!("stream: {detail}"))
}

/// Line boundaries and UTF-8 characters may both straddle HTTP chunks. Keep only
/// the unfinished line/event; comments (including provider keepalives) are ignored.
#[derive(Default)]
struct Decoder {
    line: Vec<u8>,
    data: String,
    after_cr: bool,
    first_line: bool,
}

impl Decoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, AgentError> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.after_cr && byte == b'\n' {
                self.after_cr = false;
                continue;
            }
            self.after_cr = byte == b'\r';
            if matches!(byte, b'\r' | b'\n') {
                let mut line = core::str::from_utf8(&self.line).map_err(error)?;
                if !self.first_line {
                    line = line.trim_start_matches('\u{feff}');
                    self.first_line = true;
                }
                if line.is_empty() {
                    if !self.data.is_empty() {
                        self.data.pop(); // Each data field appends one LF.
                        events.push(core::mem::take(&mut self.data));
                    }
                } else if let Some(value) = line.strip_prefix("data:") {
                    self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                    self.data.push('\n');
                }
                self.line.clear();
            } else {
                self.line.push(byte);
            }
            if self.line.len().saturating_add(self.data.len()) > MAX_LLM_RESPONSE_BYTES {
                return Err(error("event exceeds response byte limit"));
            }
        }
        Ok(events)
    }
}

enum Assembly {
    Chat(chat::Chat),
    Responses(responses::Responses),
    Anthropic(anthropic::Anthropic),
}

impl Assembly {
    fn is_done(&self) -> bool {
        match self {
            Self::Chat(v) => v.is_done(),
            Self::Responses(v) => v.is_done(),
            Self::Anthropic(v) => v.is_done(),
        }
    }
    fn event(&mut self, data: &str) -> Result<Option<String>, AgentError> {
        match self {
            Self::Chat(v) => v.event(data),
            Self::Responses(v) => v.event(data),
            Self::Anthropic(v) => v.event(data),
        }
    }

    fn finish(self) -> Result<Value, AgentError> {
        match self {
            Self::Chat(v) => v.finish(),
            Self::Responses(v) => v.finish(),
            Self::Anthropic(v) => v.finish(),
        }
    }
}

/// Only a provider terminal event commits the assembled response. A broken SSE
/// stream is never replayed automatically: its text may already be on screen.
/// Dropping this future closes the HTTP body and cancels any blocked send.
pub(super) async fn read_response(
    mut response: Response,
    request: StreamRequest<'_>,
) -> Result<Value, AgentError> {
    let mut decoder = Decoder::default();
    let mut assembly = match request.protocol {
        Protocol::Chat => Assembly::Chat(chat::Chat::default()),
        Protocol::Responses => Assembly::Responses(responses::Responses::default()),
        Protocol::Anthropic => Assembly::Anthropic(anthropic::Anthropic::default()),
    };
    let mut received = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(error)? {
        received = received.saturating_add(chunk.len());
        if received > MAX_LLM_RESPONSE_BYTES {
            return Err(error("response exceeds byte limit"));
        }
        for data in decoder.push(&chunk)? {
            if data.trim().is_empty() {
                continue;
            }
            if let Some(text) = assembly.event(&data)?.filter(|v| !v.is_empty()) {
                request
                    .updates
                    .send(LlmDelta::Text(text))
                    .await
                    .map_err(|_| error("response consumer closed"))?;
            }
            if assembly.is_done() {
                return assembly.finish();
            }
        }
    }
    assembly.finish()
}

#[cfg(test)]
mod tests;
