//! Persistent full-duplex interaction. Provider VAD commits input; this client
//! schedules responses only after playback/history and tool fences settle.
use super::*;
use crate::realtime_media::{self as media, Action, Command, Playback};

struct Output {
    response: String,
    item: String,
    emitted: u64,
    played: u64,
    sequence: u64,
}

struct State {
    output: Option<Output>,
    aside: Option<Output>,
    completed_playback: Option<Output>,
    input_sequence: u64,
    interrupted: bool,
    awaiting_playback: bool,
    awaiting_truncate: bool,
    pending_input: bool,
    response_requested: bool,
    fence_deadline: Option<Instant>,
    response_deadline: Option<Instant>,
}

impl State {
    fn new() -> Self {
        Self {
            output: None,
            aside: None,
            completed_playback: None,
            input_sequence: 0,
            interrupted: false,
            awaiting_playback: false,
            awaiting_truncate: false,
            pending_input: false,
            response_requested: false,
            fence_deadline: None,
            response_deadline: None,
        }
    }

    fn playback(&mut self, p: &Playback) -> Result<bool, AgentError> {
        // A device clock can repeat the final position after scheduling retired its item.
        if self.completed_playback.as_ref().is_some_and(|o| {
            o.response == p.response_id
                && o.item == p.item_id
                && p.content_index == 0
                && p.played_samples == o.emitted
                && o.played == o.emitted
        }) {
            return Ok(false);
        }
        let output = self
            .output
            .as_mut()
            .ok_or_else(|| error("no playback item"))?;
        if output.response != p.response_id
            || output.item != p.item_id
            || p.content_index != 0
            || p.played_samples < output.played
            || p.played_samples > output.emitted
        {
            return Err(error("stale or invalid playback position"));
        }
        if self.interrupted && !self.awaiting_playback {
            return Err(error("playback already stopped"));
        }
        output.played = p.played_samples;
        Ok(true)
    }

    async fn interrupt(
        &mut self,
        live: &mut Live,
        output: &wire::WireSender,
        sid: &str,
        stream: &str,
    ) -> Result<(), AgentError> {
        if self.interrupted {
            return Ok(());
        }
        self.interrupted = true;
        self.fence_deadline = Some(Instant::now() + Duration::from_secs(5));
        if let Some(id) = live.active_response.as_ref() {
            live.sender.cancel_response(id).await?;
        }
        self.awaiting_playback = self.output.is_some();
        media::notify(
            output,
            sid,
            stream,
            json!({"type":"clear", "responseId":live.active_response,
            "itemId":self.output.as_ref().map(|o| &o.item)}),
        )
        .await
    }

    async fn truncate(&mut self, live: &mut Live) -> Result<(), AgentError> {
        if let Some(output) = self.output.as_ref() {
            live.sender
                .truncate_audio(&output.item, 0, (output.played / 24) as u32)
                .await?;
            self.awaiting_truncate = true;
            self.fence_deadline = Some(Instant::now() + Duration::from_secs(5));
        }
        self.awaiting_playback = false;
        Ok(())
    }

    fn acknowledge_truncate(&mut self, event: &Value) -> Result<(), AgentError> {
        if !self.awaiting_truncate
            || self.output.as_ref().map(|o| o.item.as_str()) != event["item_id"].as_str()
            || event["content_index"] != 0
            || self.output.as_ref().map(|o| o.played / 24) != event["audio_end_ms"].as_u64()
        {
            return Err(error("uncorrelated truncation"));
        }
        self.awaiting_truncate = false;
        Ok(())
    }

    async fn schedule(&mut self, live: &mut Live, ctx: &RunCtx<'_>) -> Result<(), AgentError> {
        if self.awaiting_playback
            || self.awaiting_truncate
            || live.active_response.is_some()
            || self.response_requested
        {
            return Ok(());
        }
        if self.interrupted {
            self.output = None;
            self.interrupted = false;
            self.fence_deadline = None;
        }
        if self.pending_input {
            // Never overwrite accounting for an item that is still playing.
            if self.output.as_ref().is_some_and(|o| o.played < o.emitted) {
                return Ok(());
            }
            if let Some(output) = self.output.take() {
                self.completed_playback = Some(output);
            }
            self.pending_input = false;
            self.response_requested = true;
            self.response_deadline = Some(Instant::now() + ctx.cfg.llm_timeout);
            live.sender.create_response().await?;
        }
        Ok(())
    }
}

impl RealtimeSession {
    pub(crate) async fn run_media(
        &mut self,
        ctx: &mut RunCtx<'_>,
        prompt: Vec<ContentBlock>,
        mut ingress: media::Receiver,
    ) -> Result<StopReason, AgentError> {
        // Mode changes cannot silently discard an existing provider conversation.
        if self.terminal || self.live.is_some() {
            return Err(error("live media requires a fresh ACP session"));
        }
        if prompt.len() > 16 {
            return Err(error("initial media context limit"));
        }
        let mut parts = Vec::new();
        let mut bytes = 0;
        for block in prompt {
            match block {
                ContentBlock::Text { text } => {
                    bytes += text.len();
                    parts.push(json!({"type":"input_text","text":text}));
                }
                _ => return Err(error("live media initial context supports text only")),
            }
        }
        if bytes > 16384 {
            return Err(error("initial media context limit"));
        }
        let result = async {
            self.connect(ctx, true, true).await?;
            let live = self.live.as_mut().ok_or_else(|| error("missing session"))?;
            if !parts.is_empty() {
                live.sender.create_item(json!({"type":"message","role":"user","content":parts})).await?;
            }
            media::notify(ctx.wire, ctx.session_id, &ctx.run_id, json!({"type":"ready","format":"s16le","rate":24000,"channels":1,
                "maxFrameBytes":media::FRAME_BYTES,"queueFrames":media::QUEUE_FRAMES})).await?;
            let mut state = State::new();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let deadline = state.fence_deadline.into_iter().chain(state.response_deadline).fold(live.expires, Instant::min);
                let command = tokio::select! {
                    biased;
                    _ = ctx.cancel.changed() => return Err(AgentError::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => return Err(error("media session or response deadline exceeded")),
                    command = ingress.control.recv() => Some(command.ok_or_else(|| error("media controls closed"))?),
                    _ = heartbeat.tick() => {
                        media::notify(ctx.wire, ctx.session_id, &ctx.run_id, json!({"type":"heartbeat"})).await?;
                        None
                    },
                    event = live.events.recv() => {
                        let event = event.ok_or_else(|| error("session disconnected"))??;
                        if let Some(calls) = state.event(live, ctx, event).await? {
                            state.tools(live, ctx, &mut ingress, &calls).await?;
                        }
                        None
                    },
                    command = ingress.audio.recv() => Some(command.ok_or_else(|| error("media capture closed"))?),
                };
                if let Some(command) = command {
                    if state.command(live, ctx.wire, ctx.session_id, &ctx.run_id, command).await? { return Ok(StopReason::EndTurn); }
                }
                state.schedule(live, ctx).await?;
            }
        }.await;
        self.terminal = true;
        if let Some(live) = self.live.as_mut() {
            if let Some(id) = live.active_response.take() {
                let _ = live.sender.cancel_response(&id).await;
            }
        }
        self.live = None;
        let _ = media::notify(
            ctx.wire,
            ctx.session_id,
            &ctx.run_id,
            json!({"type":"closed","failed":result.is_err()}),
        )
        .await;
        match result {
            Err(AgentError::Cancelled) => Ok(StopReason::Cancelled),
            other => other,
        }
    }
}

impl State {
    async fn command(
        &mut self,
        live: &mut Live,
        output: &wire::WireSender,
        sid: &str,
        stream: &str,
        command: Command,
    ) -> Result<bool, AgentError> {
        let close = matches!(command.action, Action::Close);
        let result: Result<(), AgentError> = async {
            match command.action {
                Action::Append {
                    sequence,
                    pcm,
                    playback_pcm,
                } => {
                    if sequence != self.input_sequence {
                        Err(error("out-of-order capture frame"))
                    } else {
                        let playback = if live.duplex_audio {
                            playback_pcm.as_deref()
                        } else {
                            None
                        };
                        live.sender.append_duplex_audio(&pcm, playback).await?;
                        self.input_sequence = self
                            .input_sequence
                            .checked_add(1)
                            .ok_or_else(|| error("input sequence exhausted"))?;
                        Ok(())
                    }
                }
                Action::Playback(position) => {
                    let current = self.playback(&position)?;
                    if current && position.stopped && self.awaiting_playback {
                        self.truncate(live).await?;
                    }
                    Ok(())
                }
                Action::Interrupt(position) => {
                    let current = if let Some(position) = position.as_ref() {
                        self.playback(position)?
                    } else {
                        false
                    };
                    self.interrupt(live, output, sid, stream).await?;
                    if current {
                        self.truncate(live).await?;
                    }
                    Ok(())
                }
                Action::Close => Ok(()),
            }
        }
        .await;
        let reply = match result {
            Ok(()) => wire::ok(command.id, Value::Null),
            Err(e) => wire::err(command.id, wire::INVALID_PARAMS, &e.to_string()),
        };
        tokio::time::timeout(Duration::from_secs(2), wire::send_checked(output, reply))
            .await
            .map_err(|_| error("media consumer stalled"))?
            .map_err(|_| AgentError::Cancelled)?;
        Ok(close)
    }

    async fn event(
        &mut self,
        live: &mut Live,
        ctx: &RunCtx<'_>,
        event: Value,
    ) -> Result<Option<Vec<ToolCall>>, AgentError> {
        match event["type"].as_str() {
            Some("error") => {
                return Err(provider_error(
                    "provider rejected media operation",
                    &event["error"],
                ))
            }
            Some("input_audio_buffer.speech_started") => {
                // Also interrupt completed generation whose audio has not finished playing.
                if live.active_response.is_some()
                    || self.response_requested
                    || self.output.as_ref().is_some_and(|o| o.played < o.emitted)
                {
                    self.interrupt(live, ctx.wire, ctx.session_id, &ctx.run_id)
                        .await?;
                }
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"speech_started"}),
                )
                .await?;
            }
            Some("conversation.item.deleted") => {
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"input_removed", "itemId":field(&event,"item_id")?}),
                )
                .await?;
            }
            Some("conversation.item.input_audio_transcription.completed") => {
                let text = field(&event, "transcript")?;
                if text.len() > 16384 {
                    return Err(error("input transcript limit"));
                }
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({
                        "type":"input_transcript", "itemId":field(&event,"item_id")?, "text":text
                    }),
                )
                .await?;
            }
            Some("input_audio_buffer.speech_stopped") => {
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"speech_stopped", "lastSpeechMs":event["frankie_last_speech_ms"]}),
                )
                .await?;
            }
            Some("input_audio_buffer.cleared") => {
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"input_cleared"}),
                )
                .await?;
            }
            Some("input_audio_buffer.committed") => {
                self.pending_input = true;
            }
            Some("conversation.item.truncated") => {
                self.acknowledge_truncate(&event)?;
            }
            Some("frankie.backchannel.delta") => {
                if !live.backchannels {
                    return Err(error("unnegotiated backchannel audio"));
                }
                let id = field(&event, "id")?;
                if self.aside.is_none() {
                    live.remember(id)?;
                    self.aside = Some(Output {
                        response: id.into(),
                        item: id.into(),
                        emitted: 0,
                        played: 0,
                        sequence: 0,
                    });
                }
                let aside = self
                    .aside
                    .as_mut()
                    .ok_or_else(|| error("missing backchannel"))?;
                if aside.item != id || event["sequence"].as_u64() != Some(aside.sequence) {
                    return Err(error("uncorrelated backchannel audio"));
                }
                let delta = field(&event, "delta")?;
                if delta.len() > 12000 {
                    return Err(error("backchannel frame too large"));
                }
                let pcm = STANDARD
                    .decode(delta)
                    .map_err(|_| error("invalid backchannel PCM"))?;
                if pcm.is_empty()
                    || !pcm.len().is_multiple_of(2)
                    || aside.emitted + pcm.len() as u64 / 2 > 30720
                {
                    return Err(error("backchannel audio limit"));
                }
                media::notify(ctx.wire, ctx.session_id, &ctx.run_id,
                    json!({"type":"backchannel_audio","id":id,"startSample":aside.emitted,"data":delta})).await?;
                aside.emitted += pcm.len() as u64 / 2;
                aside.sequence += 1;
            }
            Some("frankie.backchannel.done") => {
                if !live.backchannels {
                    return Err(error("unnegotiated backchannel completion"));
                }
                let id = field(&event, "id")?;
                if let Some(aside) = self.aside.take() {
                    if aside.item != id || event["samples"].as_u64() != Some(aside.emitted) {
                        return Err(error("backchannel completion mismatch"));
                    }
                } else if event["samples"].as_u64() == Some(0) {
                    live.remember(id)?;
                } else {
                    return Err(error("missing backchannel audio"));
                }
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"backchannel_done","id":id}),
                )
                .await?;
            }
            Some("response.created") => {
                if live.active_response.is_some() || !self.response_requested {
                    return Err(error("unsolicited media response"));
                }
                let id = field(&event["response"], "id")?;
                live.remember(id)?;
                live.active_response = Some(id.to_owned());
                self.response_requested = false;
                if self.interrupted {
                    live.sender.cancel_response(id).await?;
                }
            }
            Some("response.output_audio.delta") => {
                let response = field(&event, "response_id")?;
                if live.active_response.as_deref() != Some(response) || event["content_index"] != 0
                {
                    return Err(error("uncorrelated media audio"));
                }
                if self.interrupted {
                    return Ok(None);
                }
                let item = field(&event, "item_id")?;
                let output = self.output.get_or_insert_with(|| Output {
                    response: response.to_owned(),
                    item: item.to_owned(),
                    emitted: 0,
                    played: 0,
                    sequence: 0,
                });
                if output.item != item || output.response != response {
                    return Err(error("multiple media audio items"));
                }
                let delta = field(&event, "delta")?;
                let pcm = STANDARD
                    .decode(delta)
                    .map_err(|_| error("invalid audio base64"))?;
                if pcm.is_empty()
                    || !pcm.len().is_multiple_of(2)
                    || output.emitted + pcm.len() as u64 / 2 > live.output_audio_limit
                {
                    return Err(error("media output limit"));
                }
                media::notify(
                    ctx.wire,
                    ctx.session_id,
                    &ctx.run_id,
                    json!({"type":"audio","responseId":response,"itemId":item,"contentIndex":0,
                    "sequence":output.sequence,"startSample":output.emitted,"data":delta}),
                )
                .await?;
                output.emitted += pcm.len() as u64 / 2;
                output.sequence += 1;
            }
            Some("response.done") => {
                let response = &event["response"];
                if live.active_response.as_deref() != Some(field(response, "id")?) {
                    return Err(error("uncorrelated media response.done"));
                }
                live.active_response = None;
                self.response_deadline = None;
                // Output budgets end a response, not the live microphone session.
                // Never execute even apparently complete calls from a truncated response.
                let limited = response["status"] == "incomplete"
                    && matches!(
                        response["status_details"]["reason"].as_str(),
                        Some("max_output_tokens" | "max_output_audio")
                    );
                if response["status"] != "completed"
                    && !limited
                    && !(self.interrupted && response["status"] == "cancelled")
                {
                    return Err(provider_error(
                        "media response did not complete",
                        &response["status_details"]["error"],
                    ));
                }
                let mut calls = Vec::new();
                let items = response["output"]
                    .as_array()
                    .ok_or_else(|| error("missing response output"))?;
                if items.len() > MAX_CALLS {
                    return Err(error("too many output items"));
                }
                if limited {
                    media::notify(ctx.wire, ctx.session_id, &ctx.run_id, json!({"type":"response_limited","responseId":response["id"],"reason":response["status_details"]["reason"]})).await?;
                }
                for item in items.iter().filter(|_| !limited) {
                    live.remember(field(item, "id")?)?;
                    if item["type"] == "function_call" {
                        if item["status"] != "completed" {
                            return Err(error("incomplete tool call"));
                        }
                        let id = field(item, "call_id")?;
                        live.remember(id)?;
                        calls.push(ToolCall {
                            provider_id: id.to_owned(),
                            name: field(item, "name")?.to_owned(),
                            arguments: serde_json::from_str(field(item, "arguments")?)
                                .map_err(|_| error("invalid function arguments"))?,
                            provider_extra: Default::default(),
                        });
                    } else if item["type"] != "message" || item["role"] != "assistant" {
                        return Err(error("unsupported media output"));
                    } else if response["status"] == "completed" {
                        // Completed provider text is display-only; playback positions remain
                        // sample-clock reports and never derive from this transcript.
                        if let Some(content) = item["content"].as_array() {
                            for part in content {
                                let text = match part["type"].as_str() {
                                    Some("output_audio") => part["transcript"].as_str(),
                                    Some("output_text") => part["text"].as_str(),
                                    _ => None,
                                };
                                if let Some(text) = text.filter(|text| !text.is_empty()) {
                                    wire::send(
                                        ctx.wire,
                                        wire::session_update(
                                            ctx.session_id,
                                            json!({
                                                "sessionUpdate":"agent_message_chunk",
                                                "content":{"type":"text","text":text}
                                            }),
                                        ),
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }
                media::notify(ctx.wire, ctx.session_id, &ctx.run_id, json!({"type":"response_done","responseId":response["id"],"status":response["status"]})).await?;
                if !calls.is_empty() {
                    return Ok(Some(calls));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    async fn tools(
        &mut self,
        live: &mut Live,
        ctx: &mut RunCtx<'_>,
        ingress: &mut media::Receiver,
        calls: &[ToolCall],
    ) -> Result<(), AgentError> {
        // The existing executor must finish its cancellation cleanup before this returns.
        let mut original = ctx.cancel.clone();
        let (cancel, local) = watch::channel(self.interrupted);
        let saved = std::mem::replace(ctx.cancel, local);
        let start = ctx.history.len();
        let result = {
            let output = ctx.wire.clone();
            let sid = ctx.session_id.to_owned();
            let stream = ctx.run_id.clone();
            let execute = ctx.execute_calls(calls);
            tokio::pin!(execute);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = original.changed() => { let _ = cancel.send(true); execute.await; break Err(AgentError::Cancelled); }
                    _ = tokio::time::sleep_until(self.fence_deadline.map_or(live.expires, |d| d.min(live.expires))) => { let _ = cancel.send(true); execute.await; break Err(error("session or playback fence expired during tools")); }
                    _ = heartbeat.tick() => {
                        if let Err(e) = media::notify(&output, &sid, &stream, json!({"type":"heartbeat"})).await {
                            let _ = cancel.send(true); execute.await; break Err(e);
                        }
                    }
                    command = ingress.control.recv() => {
                        let Some(command) = command else { let _ = cancel.send(true); execute.await; break Err(AgentError::Cancelled); };
                        if matches!(command.action, Action::Close | Action::Interrupt(_)) {
                            let _ = cancel.send(true);
                        }
                        match self.command(live, &output, &sid, &stream, command).await {
                            Ok(true) => { execute.await; break Err(AgentError::Cancelled); }
                            Ok(false) => {},
                            Err(e) => { let _ = cancel.send(true); execute.await; break Err(e); }
                        }
                    }
                    command = ingress.audio.recv() => {
                        let Some(command) = command else { let _ = cancel.send(true); execute.await; break Err(AgentError::Cancelled); };
                        if let Err(e) = self.command(live, &output, &sid, &stream, command).await {
                            let _ = cancel.send(true); execute.await; break Err(e);
                        }
                    }
                    event = live.events.recv() => {
                        match event {
                            Some(Ok(event)) => match event["type"].as_str() {
                                Some("input_audio_buffer.speech_started") => {
                                    let _ = cancel.send(true);
                                    if let Err(e) = self.interrupt(live, &output, &sid, &stream).await {
                                        execute.await; break Err(e);
                                    }
                                    if let Err(e) = media::notify(&output, &sid, &stream, json!({"type":"speech_started"})).await {
                                        execute.await; break Err(e);
                                    }
                                }
                                Some("conversation.item.truncated") => {
                                    if let Err(e) = self.acknowledge_truncate(&event) {
                                        let _ = cancel.send(true); execute.await; break Err(e);
                                    }
                                }
                                Some("input_audio_buffer.speech_stopped") => {
                                    if let Err(e) = media::notify(&output, &sid, &stream, json!({"type":"speech_stopped", "lastSpeechMs":event["frankie_last_speech_ms"]})).await {
                                        let _ = cancel.send(true); execute.await; break Err(e);
                                    }
                                }
                                Some("input_audio_buffer.committed") => self.pending_input = true,
                                Some("error") => { let _ = cancel.send(true); execute.await; break Err(provider_error("provider rejected media operation", &event["error"])); }
                                Some("rate_limits.updated" | "conversation.item.created" | "conversation.item.added" | "conversation.item.done") => {},
                                _ => { let _ = cancel.send(true); execute.await; break Err(error("unexpected provider event during tools")); }
                            },
                            _ => { let _ = cancel.send(true); execute.await; break Err(error("provider disconnected during tools")); }
                        }
                    }
                    _ = &mut execute => break Ok(()),
                }
            }
        };
        *ctx.cancel = saved;
        result?;
        let results: Vec<_> = ctx.history.drain(start..).collect();
        for result in results {
            let HistoryItem::ToolResult(result) = result else {
                return Err(error("invalid executor result"));
            };
            live.sender.create_item(json!({"type":"function_call_output","call_id":result.provider_id,"output":result.text()})).await?;
        }
        if !self.interrupted {
            self.pending_input = true;
        }
        Ok(())
    }
}
