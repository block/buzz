import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Raw binary invoke — uses Tauri's internal IPC for zero-copy ArrayBuffer transfer.
 *
 * The typed @tauri-apps/api doesn't support raw binary payloads (InvokeBody::Raw).
 * This wrapper isolates the internal API dependency to a single call site.
 * Tested against Tauri v2. If this breaks on upgrade, only this function needs updating.
 */
function invokeRawBinary(cmd: string, payload: Uint8Array): Promise<unknown> {
  // biome-ignore lint/suspicious/noExplicitAny: Tauri internals have no public type definition
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    return Promise.reject(new Error("Tauri internals not available"));
  }
  return internals.invoke(cmd, payload);
}

/** Return type for setupAudioWorklet — stop + mode control. */
export type AudioWorkletHandle = {
  stop: () => void;
  /** Send PTT state to the worklet processor. */
  setTransmitting: (active: boolean) => void;
  /** Switch voice input mode. In VAD mode, always transmitting (PTT events ignored).
   *  In PTT mode, gated by Ctrl+Space. */
  setMode: (mode: "push_to_talk" | "voice_activity") => void;
  /** Set mic input gain (0–1). Adjusts the GainNode between source and worklet. */
  setGain: (value: number) => void;
};

/**
 * AudioWorklet → Rust STT pipeline:
 *
 *   MediaStreamTrack (mic, 48kHz)
 *     → AudioContext.createMediaStreamSource()
 *     → AudioWorkletNode("stt-tap-processor")
 *         worklet.js accumulates 100ms batches (4800 samples)
 *         posts Float32Array to main thread via port.postMessage
 *     → onmessage: convert to Uint8Array view (zero-copy)
 *     → invokeRawBinary("push_audio_pcm", bytes)
 *         Rust: SttPipeline::push_audio → bounded sync_channel
 *
 * PTT gating:
 *   Main thread listens for Tauri "ptt-state" events (from Rust global shortcut)
 *   and forwards them to the worklet via port.postMessage({ type: 'ptt', active }).
 *   The worklet discards audio frames when transmitting=false.
 *
 * @param audioTrack - Mic track from LiveKit
 * @param initialTransmitting - Initial PTT state. true=open mic (VAD), false=muted until PTT press.
 */
export async function setupAudioWorklet(
  audioTrack: MediaStreamTrack,
  initialTransmitting = true,
): Promise<AudioWorkletHandle> {
  const audioContext = new AudioContext({ sampleRate: 48000 });

  // Resume after user gesture (required by autoplay policy)
  if (audioContext.state === "suspended") {
    await audioContext.resume();
  }

  // Load the worklet processor (must live in public/ for Vite to serve it)
  await audioContext.audioWorklet.addModule("/worklet.js");

  const source = audioContext.createMediaStreamSource(
    new MediaStream([audioTrack]),
  );

  const gainNode = audioContext.createGain();

  const workletNode = new AudioWorkletNode(audioContext, "stt-tap-processor");

  // Connect: mic → gain → worklet (tap only — no playback)
  source.connect(gainNode);
  gainNode.connect(workletNode);

  // Set initial PTT state (worklet defaults to transmitting=true).
  // In PTT mode, immediately gate audio until the user presses the key.
  if (!initialTransmitting) {
    workletNode.port.postMessage({ type: "ptt", active: false });
  }

  // Forward PCM batches to Rust via raw binary invoke.
  // Direction: worklet→main (receives PCM data from worklet processor).
  workletNode.port.onmessage = (event: MessageEvent<Float32Array>) => {
    const float32 = event.data;
    // Fire-and-forget — Rust side uses try_send which drops on backpressure.
    // No await: prevents main-thread backpressure from slow Rust processing.
    // Create a zero-copy Uint8Array view over the same underlying buffer.
    // Rust reinterprets the bytes as f32 on the other side.
    invokeRawBinary(
      "push_audio_pcm",
      new Uint8Array(float32.buffer, float32.byteOffset, float32.byteLength),
    ).catch(() => {
      /* silently drop — Rust handles backpressure */
    });
  };

  // Track the current mode so PTT events are only forwarded in PTT mode.
  // In VAD mode, the worklet stays in transmitting=true regardless of
  // Ctrl+Space presses — prevents accidental muting. (Crossfire fix I1.)
  let currentMode: "push_to_talk" | "voice_activity" = initialTransmitting
    ? "voice_activity"
    : "push_to_talk";

  // Listen for PTT state from Rust global shortcut (Ctrl+Space press/release).
  // Direction: Rust→main→worklet. The Tauri event carries a boolean payload.
  let pttUnlisten: UnlistenFn | null = null;
  try {
    pttUnlisten = await listen<boolean>("ptt-state", (event) => {
      // Only forward PTT events to the worklet when in PTT mode.
      // In VAD mode, Ctrl+Space is ignored — the worklet stays open.
      if (currentMode === "push_to_talk") {
        workletNode.port.postMessage({ type: "ptt", active: event.payload });
      }
    });
  } catch {
    // PTT events not available — worklet stays in current transmit mode.
    // This is fine for VAD mode (always transmitting) and degrades gracefully
    // for PTT mode (user won't be able to transmit, but audio won't leak).
  }

  return {
    stop: () => {
      workletNode.port.onmessage = null;
      pttUnlisten?.();
      source.disconnect();
      gainNode.disconnect();
      workletNode.disconnect();
      void audioContext.close();
    },
    setTransmitting: (active: boolean) => {
      workletNode.port.postMessage({ type: "ptt", active });
    },
    setMode: (mode: "push_to_talk" | "voice_activity") => {
      currentMode = mode;
      // When switching to VAD, immediately open the mic.
      // When switching to PTT, immediately gate until key press.
      workletNode.port.postMessage({
        type: "ptt",
        active: mode === "voice_activity",
      });
    },
    setGain: (value: number) => {
      gainNode.gain.value = value;
    },
  };
}
