import { CaptureQueue } from "./capture-queue.mjs";
export async function openVoice(token, ui, options = {}) {
  const pending = new Map(),
    stops = new Map();
  const eventsAbort = new AbortController();
  let serial = 0,
    sid,
    stream,
    context,
    node,
    tracks,
    permission,
    closed = false;
  let sequence = 0,
    captureQueue = new CaptureQueue(),
    systemQueue = new CaptureQueue(),
    sending = false,
    playbackPending = false,
    latestPlayback;
  let captureOrigin, lastSpeechMs, microphone;
  let capturedSamples = 0,
    sentSamples = 0,
    captureEnergy = 0,
    capturePeak = 0,
    lastCaptureAt = 0,
    lastDiagnostic = 0,
    diagnosticSamples = 0;
  let playbackGaps = 0,
    playbackGapSamples = 0,
    maxPlaybackGapSamples = 0;
  let lastEvent = performance.now(),
    active = false,
    suppressPlayback = false;
  const blocked = new Set();
  const evidence = [];
  // Test instrumentation observes the actual media client; it does not replace its queues or clock.
  ui.evidence(evidence);
  function status(text) {
    ui.status(text);
  }
  function record(type, detail = {}) {
    evidence.push({ time: performance.now(), type, ...detail });
    if (evidence.length > 20000) evidence.shift();
    ui.event(type, detail);
  }
  async function rpc(event) {
    const response = await fetch("/rpc", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(event),
      signal: AbortSignal.timeout(10000),
    });
    if (!response.ok) throw Error(`ACP request rejected (${response.status})`);
  }
  function request(method, params, lifetime = 10000) {
    const id = ++serial;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(Error(`${method} deadline`));
      }, lifetime);
      pending.set(id, { resolve, reject, timer });
      rpc({ jsonrpc: "2.0", id, method, params }).catch((error) => {
        clearTimeout(timer);
        pending.delete(id);
        reject(error);
      });
    });
  }
  function media(operation, params = {}) {
    return request(`_buzz/unstable/realtime/${operation}`, {
      sessionId: sid,
      streamId: stream,
      ...params,
    });
  }
  function base64(pcm) {
    const bytes = new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength);
    let text = "";
    for (const byte of bytes) text += String.fromCharCode(byte);
    return btoa(text);
  }
  function pcm16(data) {
    const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
    if (!bytes.length || bytes.length % 2) throw Error("invalid PCM");
    const view = new DataView(bytes.buffer);
    return Int16Array.from({ length: bytes.length / 2 }, (_, i) =>
      view.getInt16(i * 2, true),
    );
  }
  async function drainCapture() {
    if (sending) return;
    sending = true;
    try {
      while (captureQueue.length && !closed) {
        const pcm = captureQueue.shift(),
          playback = systemQueue.shift();
        if (pcm.length !== playback.length)
          throw Error("duplex capture alignment");
        await media("append", {
          sequence: sequence++,
          data: base64(pcm),
          playbackData: base64(playback),
        });
        sentSamples += pcm.length;
        record("capture_sent", { samples: pcm.length });
      }
    } catch (error) {
      fail(error);
    } finally {
      sending = false;
    }
  }
  async function reportPlayback() {
    if (playbackPending || suppressPlayback || !latestPlayback || closed)
      return;
    playbackPending = true;
    const playback = latestPlayback;
    latestPlayback = null;
    try {
      await media("playback", { playback });
    } catch (error) {
      if (!suppressPlayback) fail(error);
    } finally {
      playbackPending = false;
    }
  }
  function clearPlayback() {
    suppressPlayback = true;
    latestPlayback = null;
    const requestId = ++serial;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        stops.delete(requestId);
        reject(Error("playback stop deadline"));
      }, 2000);
      stops.set(requestId, { resolve, timer });
      node.port.postMessage({ type: "clear", requestId });
    });
  }
  async function interrupt(serverInitiated) {
    const playback = await clearPlayback();
    if (playback) blocked.add(playback.itemId);
    if (serverInitiated) {
      if (playback)
        await media("playback", { playback: { ...playback, stopped: true } });
    } else await media("interrupt", playback ? { playback } : {});
    record("playback_stopped", { playback });
    suppressPlayback = false;
  }
  function fail(error) {
    if (closed) return;
    status(error.message);
    record("error", { message: error.message });
    stop(false).catch(() => {});
  }
  async function stop(graceful = true) {
    if (closed) return;
    closed = true;
    active = false;
    captureQueue = new CaptureQueue();
    systemQueue = new CaptureQueue();
    tracks?.getTracks().forEach((track) => {
      track.stop();
    });
    node?.port.postMessage({ type: "capture", enabled: false });
    if (node) await clearPlayback().catch(() => {});
    try {
      if (stream) {
        if (graceful) {
          await media("close");
          const deadline = performance.now() + 5000;
          while (!evidence.some((e) => e.type === "prompt_done")) {
            if (performance.now() > deadline) throw Error("ACP close deadline");
            await new Promise((r) => setTimeout(r, 10));
          }
        } else
          await rpc({
            jsonrpc: "2.0",
            method: "session/cancel",
            params: { sessionId: sid },
          });
      }
    } catch (error) {
      status(error.message);
      record("error", { message: error.message });
      throw error;
    } finally {
      await context?.close();
      eventsAbort.abort();
      for (const p of pending.values()) {
        clearTimeout(p.timer);
        p.reject(Error("Session closed"));
      }
      pending.clear();
      ui.ended();
    }
    if (graceful) {
      status("Connection ended. Start again whenever you like.");
      record("client_closed");
    }
  }
  async function onEvent(event) {
    lastEvent = performance.now();
    if (event.method === "session/request_permission") {
      permission = event;
      ui.permission(
        event.params.subject?.toolCall || event.params.toolCall,
        decide,
      );
      record("permission");
      return;
    }
    if (event.id !== undefined && pending.has(event.id)) {
      const item = pending.get(event.id);
      pending.delete(event.id);
      clearTimeout(item.timer);
      event.error
        ? item.reject(Error(event.error.message))
        : item.resolve(event.result);
      return;
    }
    if (event.method === "session/update") {
      const u = event.params.update;
      if (
        u.sessionUpdate === "agent_message_chunk" &&
        u.content?.type === "text"
      )
        ui.transcript(u.content.text);
      if (
        u.sessionUpdate === "tool_call" ||
        u.sessionUpdate === "tool_call_update"
      )
        record(u.sessionUpdate, u);
    }
    if (event.method !== "_buzz/unstable/realtime/update") return;
    const u = event.params.update;
    if (u.type === "ready") {
      stream = event.params.streamId;
      active = true;
      record("ready");
      status(
        `Listening through ${microphone?.label || "your microphone"}. You can interrupt Frankie.`,
      );

      return;
    }
    if (u.type === "backchannel_audio") {
      if (closed) return;
      const pcm = pcm16(u.data);
      record("backchannel_audio", {
        id: u.id,
        startSample: u.startSample,
        samples: pcm.length,
      });
      node.port.postMessage({ type: "backchannel", id: u.id, pcm }, [
        pcm.buffer,
      ]);
    } else if (u.type === "audio") {
      if (blocked.has(u.itemId) || suppressPlayback || closed) return;
      const pcm = pcm16(u.data);
      record("audio_received", {
        responseId: u.responseId,
        itemId: u.itemId,
        samples: pcm.length,
        startSample: u.startSample,
      });
      node.port.postMessage(
        { type: "audio", responseId: u.responseId, itemId: u.itemId, pcm },
        [pcm.buffer],
      );
    } else if (u.type === "clear") {
      if (u.itemId) blocked.add(u.itemId);
      // Do not await an ACP reply inside its event reader.
      interrupt(true).catch(fail);
    } else if (u.type === "closed") {
      record("provider_closed", { failed: u.failed });
      if (!closed) {
        if (u.failed)
          setTimeout(
            () => fail(Error("Provider session failed without an ACP error")),
            500,
          );
        else fail(Error("Provider session closed"));
      }
    } else if (u.type === "response_limited") {
      status("Frankie reached the reply limit. You can keep talking.");
      record(u.type, { responseId: u.responseId, reason: u.reason });
    } else if (u.type === "input_removed") {
      ui.removeInput(u.itemId);
      record(u.type, { itemId: u.itemId });
    } else if (u.type === "input_transcript") {
      ui.userTranscript(u.text, u.itemId);
      record(u.type, { itemId: u.itemId, text: u.text });
    } else {
      if (u.type === "speech_started") lastSpeechMs = undefined;
      if (u.type === "speech_stopped" && Number.isFinite(u.lastSpeechMs))
        lastSpeechMs = u.lastSpeechMs;
      record(u.type, {
        lastSpeechMs: u.lastSpeechMs,
        responseId: u.responseId,
        status: u.status,
      });
    }
  }
  async function connect() {
    const response = await fetch(
      `/events?thinking=${encodeURIComponent(options.thinking || "none")}`,
      {
        signal: eventsAbort.signal,
        headers: { Authorization: `Bearer ${token}` },
      },
    );
    if (!response.ok) throw Error("Use the complete local launch URL");
    const reader = response.body.getReader(),
      decoder = new TextDecoder();
    (async () => {
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) throw Error("ACP disconnected");
        buffer += decoder.decode(value, { stream: true });
        if (buffer.length > 2 * 1024 * 1024) throw Error("ACP output limit");
        for (;;) {
          const index = buffer.indexOf("\n");
          if (index < 0) break;
          const line = buffer.slice(0, index);
          buffer = buffer.slice(index + 1);
          await onEvent(JSON.parse(line));
        }
      }
    })().catch(fail);
    const init = await request("initialize", {
      protocolVersion: 1,
      clientCapabilities: { _meta: { buzz: { realtimeAudio: 1 } } },
    });
    if (init.agentCapabilities?._meta?.buzz?.realtimeAudio !== 1)
      throw Error("Agent does not support live PCM");
    sid = (await request("session/new", {})).sessionId;
  }
  async function start() {
    context = new AudioContext({ sampleRate: 24000 });
    await context.resume();
    await context.audioWorklet.addModule("/audio-worklet.mjs");
    node = new AudioWorkletNode(context, "duplex-audio", {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    node.connect(context.destination);
    node.port.onmessage = ({ data }) => {
      if (data.type === "capture" && !closed) {
        lastCaptureAt = performance.now();
        capturedSamples += data.pcm.length;
        for (const sample of data.pcm) {
          const value = sample / 32768;
          captureEnergy += value * value;
          capturePeak = Math.max(capturePeak, Math.abs(value));
        }
        if (Number.isFinite(data.captureTime))
          captureOrigin = data.captureTime - data.captureSamples / 24000;
        try {
          captureQueue.push(data.pcm);
          systemQueue.push(data.playback || new Int16Array(data.pcm.length));
          drainCapture();
        } catch (error) {
          fail(error);
        }
      } else if (
        data.type === "position" &&
        data.playback &&
        !suppressPlayback
      ) {
        latestPlayback = data.playback;
        if (data.playback.playedSamples > 0) {
          const latency =
            Number.isFinite(captureOrigin) &&
            Number.isFinite(lastSpeechMs) &&
            Number.isFinite(data.startedAt)
              ? (data.startedAt - captureOrigin) * 1000 - lastSpeechMs
              : undefined;
          record("played", { ...data.playback, firstSoundLatencyMs: latency });
        }
        reportPlayback();
      } else if (data.type === "playback_gap") {
        playbackGaps++;
        playbackGapSamples += data.missingSamples;
        maxPlaybackGapSamples = Math.max(
          maxPlaybackGapSamples,
          data.missingSamples,
        );
        record("playback_gap", {
          itemId: data.itemId,
          playedSamples: data.playedSamples,
          gapMs: data.missingSamples / 24,
        });
      } else if (data.type === "stopped" && stops.has(data.requestId)) {
        const item = stops.get(data.requestId);
        stops.delete(data.requestId);
        clearTimeout(item.timer);
        item.resolve(data.playback);
      } else if (data.type === "error") fail(Error(data.message));
    };
    {
      tracks = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      if (closed) {
        tracks.getTracks().forEach((t) => {
          t.stop();
        });
        throw Error("Session closed during microphone setup");
      }
      microphone = tracks.getAudioTracks()[0];
      record("microphone", { sampleRate: microphone.getSettings().sampleRate });
      microphone.addEventListener("ended", () => {
        if (!closed)
          fail(
            Error("The microphone disconnected. Start again to reconnect it."),
          );
      });
      microphone.addEventListener("mute", () => record("microphone_muted"));
      microphone.addEventListener("unmute", () => record("microphone_unmuted"));
      const source = context.createMediaStreamSource(tracks);
      source.connect(node);
      const mic = context.createAnalyser(),
        voice = context.createAnalyser();
      mic.fftSize = voice.fftSize = 256;
      source.connect(mic);
      node.connect(voice);
      ui.analyzers(mic, voice);
    }
    const text = ui.context || "";
    request(
      "session/prompt",
      {
        sessionId: sid,
        prompt: text ? [{ type: "text", text }] : [],
        _meta: { buzz: { realtimeAudio: 1 } },
      },
      60 * 60 * 1000,
    )
      .then((result) => record("prompt_done", result))
      .catch(fail);
    status("Preparing Frankie's conversation…");
    const deadline = performance.now() + 120000;
    while (!active) {
      if (closed || performance.now() > deadline)
        throw Error("media readiness deadline");
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    node.port.postMessage({ type: "capture", enabled: true });
  }
  async function decide(kind) {
    if (!permission) return;
    const event = permission;
    permission = null;
    ui.permission(null);
    const option = event.params.options.find((o) => o.kind === kind);
    if (!option) throw Error("missing permission option");
    await rpc({
      jsonrpc: "2.0",
      id: event.id,
      result: { outcome: { outcome: "selected", optionId: option.optionId } },
    });
  }

  const idle = setInterval(() => {
    if (!active) return;
    const now = performance.now();
    if (now - lastDiagnostic >= 5000) {
      const detail = {
        capturedSamples,
        sentSamples,
        queuedSamples: captureQueue.length,
        captureAgeMs: lastCaptureAt ? now - lastCaptureAt : null,
        rms: Math.sqrt(
          captureEnergy / Math.max(1, capturedSamples - diagnosticSamples),
        ),
        peak: capturePeak,
        contextState: context?.state,
        muted: microphone?.muted,
        trackState: microphone?.readyState,
        sampleRate: context?.sampleRate,
        outputLatency: context?.outputLatency,
        playbackGaps,
        playbackGapMs: playbackGapSamples / 24,
        maxPlaybackGapMs: maxPlaybackGapSamples / 24,
      };
      record("capture_health", detail);
      lastDiagnostic = now;
      diagnosticSamples = capturedSamples;
      captureEnergy = 0;
      capturePeak = 0;
    }
    if (now - lastEvent > 45000) fail(Error("media idle deadline"));
  }, 1000);
  const cancel = () => {
    stop(false).catch(() => {});
  };
  options.signal?.addEventListener("abort", cancel, { once: true });
  const end = ui.ended;
  ui.ended = () => {
    clearInterval(idle);
    options.signal?.removeEventListener("abort", cancel);
    end();
  };
  if (options.signal?.aborted) {
    await stop(false);
    throw Error("Connection cancelled");
  }
  try {
    await connect();
    await start();
  } catch (error) {
    fail(error);
    throw error;
  }
  return { stop, interrupt: () => interrupt(false), decide };
}
