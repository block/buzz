//! Manual-turn ACP driver for a persistent, server-owned Realtime conversation.
//! Transport loss is terminal: never replay possibly committed tool effects.

use std::{collections::HashSet, time::Duration};

use crate::realtime_audio::{decode_wav, encode_wav, MAX_PCM};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::Instant,
};

use crate::{
    agent::{prompt_to_text, RunCtx},
    realtime::{RealtimeConnection, RealtimeSender},
    types::{AgentError, ContentBlock, HistoryItem, StopReason, ToolCall},
    wire,
};

const SESSION_LIMIT: Duration = Duration::from_secs(60 * 60);
const MAX_CALLS: usize = 64;
const MAX_SESSION_IDS: usize = 4096;

fn error(message: &str) -> AgentError {
    AgentError::Llm(format!("realtime: {message}"))
}

#[derive(Default)]
pub(crate) struct RealtimeSession {
    live: Option<Live>,
    terminal: bool,
}

enum Input {
    Text(String),
    Audio(Vec<u8>),
}

struct Live {
    sender: RealtimeSender,
    events: mpsc::Receiver<Result<Value, AgentError>>,
    reader: JoinHandle<()>,
    expires: Instant,
    ids: HashSet<String>,
    active_response: Option<String>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl RealtimeSession {
    pub(crate) async fn run(
        &mut self,
        ctx: &mut RunCtx<'_>,
        prompt: Vec<ContentBlock>,
    ) -> Result<StopReason, AgentError> {
        if prompt.is_empty() || prompt.len() > 32 {
            return Err(AgentError::InvalidParams(
                "realtime: invalid prompt count".into(),
            ));
        }
        let mut inputs = Vec::new();
        let mut total = 0usize;
        let mut audio_output = false;
        for block in prompt {
            let input = match block {
                ContentBlock::Audio { data, mime_type } => {
                    audio_output = true;
                    Input::Audio(decode_wav(&data, &mime_type)?)
                }
                other => Input::Text(prompt_to_text(vec![other])?),
            };
            total += match &input {
                Input::Text(text) => text.len(),
                Input::Audio(pcm) => pcm.len(),
            };
            inputs.push(input);
        }
        if total == 0 || total > MAX_PCM {
            return Err(AgentError::InvalidParams(
                "realtime: empty or oversized prompt".into(),
            ));
        }
        if self.terminal {
            return Err(error(
                "session ended; create a new ACP session (no automatic replay)",
            ));
        }
        let result = self.run_inner(ctx, inputs, audio_output).await;
        if !matches!(result, Ok(StopReason::EndTurn)) {
            self.terminal = true;
            // A cancelled read is terminal, but the independent writer can still
            // cancel the response before both socket halves are dropped.
            if let Some(live) = self.live.as_mut() {
                if let Some(id) = live.active_response.take() {
                    if let Err(e) = live.sender.cancel_response(&id).await {
                        tracing::debug!("realtime cancellation transport failed: {e}");
                    }
                }
            }
            self.live = None;
        }
        match result {
            Err(AgentError::Cancelled) => Ok(StopReason::Cancelled),
            other => other,
        }
    }

    async fn run_inner(
        &mut self,
        ctx: &mut RunCtx<'_>,
        inputs: Vec<Input>,
        audio_output: bool,
    ) -> Result<StopReason, AgentError> {
        if *ctx.cancel.borrow() {
            return Err(AgentError::Cancelled);
        }
        if self.live.is_none() {
            let endpoint = endpoint(&ctx.cfg.base_url, ctx.effective_model)?;
            let connection = tokio::select! {
                biased;
                _ = ctx.cancel.changed() => return Err(AgentError::Cancelled),
                result = RealtimeConnection::connect(&endpoint, &ctx.cfg.api_key) => result?,
            };
            let (sender, mut receiver) = connection.split();
            let (tx, events) = mpsc::channel(8);
            let expires = Instant::now() + SESSION_LIMIT;
            let reader = tokio::spawn(async move {
                loop {
                    let event = match tokio::time::timeout_at(expires, receiver.next_event()).await
                    {
                        Ok(event) => event,
                        Err(_) => Err(error("session expired")),
                    };
                    let terminal = event.is_err();
                    // The queue is bounded and the absolute session deadline also
                    // bounds a peer that stops consuming events while idle.
                    if !matches!(
                        tokio::time::timeout_at(expires, tx.send(event)).await,
                        Ok(Ok(()))
                    ) || terminal
                    {
                        break;
                    }
                }
            });
            self.live = Some(Live {
                sender,
                events,
                reader,
                expires,
                ids: HashSet::new(),
                active_response: None,
            });
            let live = self.live.as_mut().ok_or_else(|| error("missing session"))?;
            let mut tools = ctx.mcp.tools();
            if !ctx.skills.is_empty() {
                tools.push(crate::builtin::load_skill_def());
            }
            let tools: Vec<_> = tools
                .iter()
                .map(|tool| {
                    json!({
                        "type":"function", "name":tool.name, "description":tool.description,
                        "parameters":tool.input_schema,
                    })
                })
                .collect();
            live.sender
                .update_session(json!({
                    "type":"realtime", "model":ctx.effective_model,
                    "instructions":ctx.system_prompt, "tools":tools,
                    "output_modalities":[if audio_output { "audio" } else { "text" }],
                    "audio":{"input":{"turn_detection":null}},
                }))
                .await?;
            let deadline = Instant::now() + ctx.cfg.llm_timeout;
            let updated = live.next(ctx, deadline).await?;
            if updated["type"] != "session.updated"
                || updated["session"]["output_modalities"]
                    != json!([if audio_output { "audio" } else { "text" }])
                || updated["session"]["audio"]["input"].get("turn_detection") != Some(&Value::Null)
            {
                return Err(error("provider did not acknowledge manual text session"));
            }
        }
        let live = self.live.as_mut().ok_or_else(|| error("missing session"))?;
        if Instant::now() >= live.expires || live.reader.is_finished() {
            return Err(error("session disconnected or expired"));
        }
        // Apply the turn modality explicitly even on a reused connection.
        live.sender.update_session(json!({"type":"realtime", "output_modalities":[if audio_output { "audio" } else { "text" }]})).await?;
        let deadline = Instant::now() + ctx.cfg.llm_timeout;
        loop {
            let event = live.next(ctx, deadline).await?;
            if event["type"] == "session.updated" {
                if event["session"]["output_modalities"]
                    != json!([if audio_output { "audio" } else { "text" }])
                {
                    return Err(error("output modality not acknowledged"));
                }
                break;
            }
        }
        for input in inputs {
            if *ctx.cancel.borrow() {
                return Err(AgentError::Cancelled);
            }
            match input {
                Input::Text(text) => live.sender.create_item(json!({"type":"message", "role":"user", "content":[{"type":"input_text", "text":text}]})).await?,
                Input::Audio(pcm) => {
                    for chunk in pcm.chunks(24_000) {
                        if *ctx.cancel.borrow() { return Err(AgentError::Cancelled); }
                        live.sender.append_audio(chunk).await?;
                    }
                    live.sender.commit_audio().await?;
                }
            }
        }
        // Realtime has its own bounded server-side context. This slice does not
        // run the coding API's compactor or maintain a competing text history.
        for _ in 0..if ctx.cfg.max_rounds == 0 {
            64
        } else {
            ctx.cfg.max_rounds.min(64)
        } {
            if *ctx.cancel.borrow() {
                return Err(AgentError::Cancelled);
            }
            live.sender.create_response().await?;
            let deadline = Instant::now() + ctx.cfg.llm_timeout;
            let mut pcm = Vec::new();
            let mut audio_item: Option<String> = None;
            let response = loop {
                let event = live.next(ctx, deadline).await?;
                match event["type"].as_str() {
                    Some("response.created") => {
                        let id = field(&event["response"], "id")?;
                        if live.active_response.is_some() {
                            return Err(error("overlapping response"));
                        }
                        live.remember(id)?;
                        live.active_response = Some(id.to_owned());
                    }
                    Some("response.done") => {
                        let response = &event["response"];
                        if live.active_response.as_deref() != Some(field(response, "id")?) {
                            return Err(error("uncorrelated response.done"));
                        }
                        live.active_response = None;
                        break response.clone();
                    }
                    // Text comes from the authoritative completed output. No
                    // partial function arguments ever reach the tool executor.
                    Some("response.output_audio.delta") if audio_output => {
                        if live.active_response.as_deref() != Some(field(&event, "response_id")?)
                            || event["content_index"] != 0
                        {
                            return Err(error("uncorrelated audio"));
                        }
                        let item = field(&event, "item_id")?;
                        if audio_item.as_deref().is_some_and(|id| id != item) {
                            return Err(error("multiple audio items unsupported"));
                        }
                        audio_item = Some(item.to_owned());
                        let chunk = STANDARD
                            .decode(field(&event, "delta")?)
                            .map_err(|_| error("invalid audio base64"))?;
                        if chunk.is_empty()
                            || chunk.len() % 2 != 0
                            || pcm.len() + chunk.len() > MAX_PCM
                        {
                            return Err(error("audio output limit"));
                        }
                        pcm.extend_from_slice(&chunk);
                    }
                    Some("response.output_audio.delta" | "input_audio_buffer.speech_started") => {
                        return Err(error("unsolicited audio in manual text session"));
                    }
                    _ => {}
                }
            };
            if response["status"] == "cancelled" {
                return Err(AgentError::Cancelled);
            }
            if response["status"] != "completed" {
                return Err(error("response did not complete"));
            }
            let output = response["output"]
                .as_array()
                .ok_or_else(|| error("missing response output"))?;
            if output.len() > MAX_CALLS {
                return Err(error("too many output items"));
            }
            let mut calls = Vec::new();
            let mut text = String::new();
            // Validate the entire response and reserve all IDs BEFORE effects.
            for item in output {
                live.remember(field(item, "id")?)?;
                match item["type"].as_str() {
                    Some("function_call") => {
                        if item["status"] != "completed" {
                            return Err(error("incomplete tool call"));
                        }
                        let call_id = field(item, "call_id")?;
                        live.remember(call_id)?;
                        let arguments = serde_json::from_str(field(item, "arguments")?)
                            .map_err(|_| error("invalid function arguments"))?;
                        calls.push(ToolCall {
                            provider_id: call_id.to_owned(),
                            name: field(item, "name")?.to_owned(),
                            arguments,
                            provider_extra: Default::default(),
                        });
                    }
                    Some("message")
                        if item["role"] == "assistant" && item["status"] == "completed" =>
                    {
                        for content in item["content"]
                            .as_array()
                            .ok_or_else(|| error("missing message content"))?
                        {
                            match content["type"].as_str() {
                                Some("output_text") if !audio_output => {
                                    text.push_str(field(content, "text")?)
                                }
                                Some("output_audio")
                                    if audio_output
                                        && audio_item.as_deref() == item["id"].as_str() =>
                                {
                                    text.push_str(field(content, "transcript")?);
                                }
                                _ => return Err(error("unsupported response content")),
                            }
                        }
                    }
                    _ => return Err(error("unsupported output item")),
                }
            }
            if !text.is_empty() {
                wire::send(ctx.wire, wire::session_update(ctx.session_id, json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":text}}))).await;
            }
            if calls.is_empty() {
                if audio_output {
                    let data = encode_wav(&pcm)?;
                    wire::send(ctx.wire, wire::session_update(ctx.session_id, json!({"sessionUpdate":"agent_message_chunk", "content":{"type":"audio", "mimeType":"audio/wav", "data":data}}))).await;
                }
                return Ok(StopReason::EndTurn);
            }
            if !pcm.is_empty() {
                return Err(error("combined tool and audio output unsupported"));
            }
            let start = ctx.history.len();
            let stop = live.execute_calls(ctx, &calls).await?;
            let results: Vec<_> = ctx.history.drain(start..).collect();
            if stop.is_some() || *ctx.cancel.borrow() {
                return Err(AgentError::Cancelled);
            }
            if Instant::now() >= live.expires || live.reader.is_finished() {
                return Err(error(
                    "session ended during tool execution; effects are not replayed",
                ));
            }
            for result in results {
                let HistoryItem::ToolResult(result) = result else {
                    return Err(error("invalid executor result"));
                };
                live.sender.create_item(json!({"type":"function_call_output", "call_id":result.provider_id, "output":result.text()})).await?;
            }
        }
        Ok(StopReason::MaxTurnRequests)
    }
}

impl Live {
    // Reuse the existing executor, but feed it cancellation from BOTH ACP and
    // provider lifetime. Never drop its future mid-tool: let it drain/kill MCP
    // work through the same cancellation path as the coding API.
    async fn execute_calls(
        &mut self,
        ctx: &mut RunCtx<'_>,
        calls: &[ToolCall],
    ) -> Result<Option<StopReason>, AgentError> {
        let mut original = ctx.cancel.clone();
        let (cancel_tx, local) = watch::channel(*original.borrow());
        let saved = std::mem::replace(ctx.cancel, local);
        let result = {
            let execute = ctx.execute_calls(calls);
            tokio::pin!(execute);
            loop {
                tokio::select! {
                    biased;
                    _ = original.changed() => {
                        let _ = cancel_tx.send(true);
                        execute.await;
                        break Err(AgentError::Cancelled);
                    }
                    _ = tokio::time::sleep_until(self.expires) => {
                        let _ = cancel_tx.send(true);
                        execute.await;
                        break Err(error("session expired during tool execution"));
                    }
                    event = self.events.recv() => {
                        match event {
                            Some(Ok(event)) if matches!(event["type"].as_str(), Some("rate_limits.updated" | "conversation.item.created" | "conversation.item.added" | "conversation.item.done")) => {}
                            _ => {
                                let _ = cancel_tx.send(true);
                                execute.await;
                                break Err(error("session disconnected or sent unexpected event during tool execution"));
                            }
                        }
                    }
                    stop = &mut execute => break Ok(stop),
                }
            }
        };
        *ctx.cancel = saved;
        result
    }

    fn remember(&mut self, id: &str) -> Result<(), AgentError> {
        if self.ids.len() >= MAX_SESSION_IDS || !self.ids.insert(id.to_owned()) {
            return Err(error("duplicate ID or session item limit reached"));
        }
        Ok(())
    }

    async fn next(&mut self, ctx: &mut RunCtx<'_>, deadline: Instant) -> Result<Value, AgentError> {
        if *ctx.cancel.borrow() {
            return Err(AgentError::Cancelled);
        }
        tokio::select! {
            biased;
            _ = ctx.cancel.changed() => Err(AgentError::Cancelled),
            result = tokio::time::timeout_at(deadline.min(self.expires), self.events.recv()) => {
                result.map_err(|_| error("response deadline exceeded"))?
                    .ok_or_else(|| error("session disconnected"))?
            }
        }
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, AgentError> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| error("missing response string"))
}

fn endpoint(base: &str, model: &str) -> Result<String, AgentError> {
    let mut url = url::Url::parse(base).map_err(|_| error("invalid base URL"))?;
    // Explicit ws(s) endpoints retain their path and query; HTTP API roots get
    // the standard Realtime suffix. There is no provider-host inference.
    match url.scheme() {
        "http" | "https" => {
            let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
            url.set_scheme(scheme)
                .map_err(|_| error("invalid URL scheme"))?;
            url.set_path(&format!("{}/realtime", url.path().trim_end_matches('/')));
        }
        "ws" | "wss" => {}
        _ => return Err(error("invalid endpoint scheme")),
    }
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key != "model")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("model", model);
    Ok(url.to_string())
}
