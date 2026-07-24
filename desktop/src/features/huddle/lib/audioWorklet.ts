import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setupPcmCapture } from "@/shared/lib/pcmCapture";

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
  const capture = await setupPcmCapture(
    audioTrack,
    "push_audio_pcm",
    initialTransmitting,
  );

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
        capture.setTransmitting(event.payload);
      }
    });
  } catch {
    // PTT events not available — worklet stays in current transmit mode.
    // This is fine for VAD mode (always transmitting) and degrades gracefully
    // for PTT mode (user won't be able to transmit, but audio won't leak).
  }

  return {
    stop: () => {
      pttUnlisten?.();
      capture.stop();
    },
    setTransmitting: (active: boolean) => {
      capture.setTransmitting(active);
    },
    setMode: (mode: "push_to_talk" | "voice_activity") => {
      currentMode = mode;
      // When switching to VAD, immediately open the mic.
      // When switching to PTT, immediately gate until key press.
      capture.setTransmitting(mode === "voice_activity");
    },
    setGain: (value: number) => {
      capture.setGain(value);
    },
  };
}
