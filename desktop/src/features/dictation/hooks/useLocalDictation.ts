import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

/**
 * Raw binary invoke — uses Tauri's internal IPC for zero-copy ArrayBuffer transfer.
 * Same pattern as huddle's audioWorklet.ts.
 */
function invokeRawBinary(cmd: string, payload: Uint8Array): Promise<unknown> {
  // biome-ignore lint/suspicious/noExplicitAny: Tauri internals have no public type definition
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    return Promise.reject(new Error("Tauri internals not available"));
  }
  return internals.invoke(cmd, payload);
}

interface UseLocalDictationOptions {
  disabled?: boolean;
  onRecordingStart?: () => void;
  onTranscriptText: (text: string) => void;
}

interface DictationStatus {
  available: boolean;
  active: boolean;
}

const DICTATION_TRANSCRIPT_EVENT = "dictation-transcript";
const DICTATION_STATE_EVENT = "dictation-state";

/** Interval (ms) to poll model availability after initial unavailability. */
const MODEL_POLL_INTERVAL_MS = 5_000;

/**
 * Batching interval for audio IPC (ms). The worklet posts every render quantum
 * (~2.67ms at 48kHz/128 samples). Sending each one individually overloads IPC.
 * We accumulate ~100ms of audio before sending to reduce IPC overhead from
 * ~375 calls/s to ~10 calls/s.
 */
const AUDIO_BATCH_MS = 100;

/**
 * Max samples per `push_dictation_audio` IPC call. The native command rejects
 * any raw audio payload over 100 KB (`MAX_AUDIO_BATCH_BYTES` in `dictation.rs`);
 * at 48 kHz f32 mono that is 25,600 samples (~0.53s). We chunk under that cap
 * (24,000 samples / 96 KB, leaving headroom) so a stalled main thread that
 * lets the batch grow past ~0.5s can't produce a single oversized buffer that
 * native rejects and we silently drop. Chunks are sent in order.
 */
const MAX_IPC_SAMPLES = 24_000;

/**
 * Size (bytes) of the little-endian `u64` session header prepended to each
 * `push_dictation_audio` payload. Native reads this header and only feeds audio
 * whose session matches the currently-active one, so late chunks from a
 * just-stopped session can't leak into a newer session's transcript.
 */
const SESSION_HEADER_BYTES = 8;

/**
 * Local STT dictation hook using the Parakeet model via Tauri native commands.
 *
 * Works fully offline — no relay or OpenAI API key needed. Uses the same
 * sherpa-onnx Parakeet TDT-CTC 110M model as huddle transcription.
 *
 * Audio capture uses the Web Audio API (AudioWorklet) on the frontend side,
 * then sends batched raw PCM bytes to the native STT engine via
 * `push_dictation_audio`.
 */
export function useLocalDictation({
  disabled = false,
  onRecordingStart,
  onTranscriptText,
}: UseLocalDictationOptions) {
  const [isRecording, setIsRecording] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [isAvailable, setIsAvailable] = useState(false);

  const streamRef = useRef<MediaStream | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const workletRef = useRef<AudioWorkletNode | null>(null);
  const outputGainRef = useRef<GainNode | null>(null);
  const batchTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const audioBatchRef = useRef<Float32Array[]>([]);
  // Tail of the serialized flush chain. Each flush appends its IPC work to
  // this promise so flushes run strictly one-after-another; awaiting the tail
  // guarantees every already-started flush's chunks are enqueued native-side.
  const flushChainRef = useRef<Promise<void>>(Promise.resolve());
  const unlistenTranscriptRef = useRef<UnlistenFn | null>(null);
  const unlistenStateRef = useRef<UnlistenFn | null>(null);
  const onRecordingStartRef = useRef(onRecordingStart);
  const onTranscriptTextRef = useRef(onTranscriptText);
  // Native session ID — set after `start_dictation` returns. Transcript and
  // state events include this ID so we can definitively ignore stale events
  // from a previous session's forwarder.
  const nativeSessionRef = useRef<number>(0);
  // Abort flag — set when stop/cancel is called while startRecording is still
  // awaiting async setup. The start resumes and bails before activating.
  const startAbortedRef = useRef(false);

  onRecordingStartRef.current = onRecordingStart;
  onTranscriptTextRef.current = onTranscriptText;

  const isEnabled = !disabled && isAvailable;

  // Check availability on mount and poll until available (model may be downloading).
  useEffect(() => {
    if (disabled) {
      setIsAvailable(false);
      return;
    }

    let cancelled = false;
    let pollTimer: ReturnType<typeof setInterval> | null = null;

    function checkAvailability() {
      invoke<DictationStatus>("get_dictation_status")
        .then((status) => {
          if (cancelled) return;
          setIsAvailable(status.available);
          // Stop polling once the model is ready.
          if (status.available && pollTimer) {
            clearInterval(pollTimer);
            pollTimer = null;
          }
        })
        .catch(() => {
          if (!cancelled) setIsAvailable(false);
        });
    }

    checkAvailability();

    // Poll periodically until available (handles background model download).
    pollTimer = setInterval(() => {
      if (cancelled) return;
      checkAvailability();
    }, MODEL_POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      if (pollTimer) clearInterval(pollTimer);
    };
  }, [disabled]);

  /** Flush accumulated audio batch to the native STT engine. Returns a promise
   *  that resolves once the IPC call completes (or immediately if nothing to flush).
   *
   *  Flushes are serialized through `flushChainRef`: the batch is drained
   *  synchronously here (so each flush owns a distinct set of samples in FIFO
   *  order), but the actual IPC send is chained after any prior in-flight
   *  flush. This keeps chunks strictly ordered and lets `stopRecording`/
   *  `cleanup` await the chain tail so an earlier timer-tick flush can't still
   *  be enqueuing chunks when `stop_dictation` fires. */
  const flushAudioBatch = useCallback((): Promise<void> => {
    const batch = audioBatchRef.current;
    if (batch.length === 0) return flushChainRef.current;

    // Tag this flush with the session that owns the buffered audio. Captured
    // once here so every chunk of this batch carries the same session; native
    // drops chunks whose session no longer matches the active one, so a late
    // flush from a just-stopped session can't leak into a newer session.
    const session = nativeSessionRef.current;

    // Calculate total byte length and merge into a single buffer. Drain the
    // batch synchronously so a concurrent timer tick can't grab the same
    // samples — the merged buffer captured here is this flush's exclusive
    // payload.
    let totalSamples = 0;
    for (const chunk of batch) {
      totalSamples += chunk.length;
    }
    const merged = new Float32Array(totalSamples);
    let offset = 0;
    for (const chunk of batch) {
      merged.set(chunk, offset);
      offset += chunk.length;
    }
    audioBatchRef.current = [];

    // Split into chunks under the native IPC cap and send them in order.
    // A single unbounded buffer can exceed the 100 KB native limit if the
    // batch timer was delayed (e.g. main-thread stall), in which case native
    // rejects it and the whole chunk is silently lost.
    const sendChunks = async (): Promise<void> => {
      for (let start = 0; start < merged.length; start += MAX_IPC_SAMPLES) {
        const slice = merged.subarray(
          start,
          Math.min(start + MAX_IPC_SAMPLES, merged.length),
        );
        // Build the IPC payload: an 8-byte LE u64 session header followed by
        // the audio bytes. Copy the slice into the header-prefixed buffer so
        // the raw payload is exactly this chunk (a subarray view shares the
        // parent's ArrayBuffer).
        const payload = new Uint8Array(SESSION_HEADER_BYTES + slice.byteLength);
        new DataView(payload.buffer).setBigUint64(
          0,
          BigInt(session),
          true, // little-endian
        );
        payload.set(
          new Uint8Array(
            slice.buffer.slice(
              slice.byteOffset,
              slice.byteOffset + slice.byteLength,
            ),
          ),
          SESSION_HEADER_BYTES,
        );
        await invokeRawBinary("push_dictation_audio", payload).catch(() => {});
      }
    };

    // Chain this flush's IPC work after any prior in-flight flush so chunks
    // stay strictly ordered and awaiting the tail drains everything.
    const chained = flushChainRef.current.then(sendChunks);
    flushChainRef.current = chained;
    return chained;
  }, []);

  const cleanup = useCallback(() => {
    // Abort any in-flight startRecording so it bails after its next await
    // instead of resuming and opening the mic/worklet or leaving the native
    // session/listeners running after teardown. `cleanup` is the unmount
    // handler (and the catch-path teardown); without this, unmounting while
    // startRecording awaits `start_dictation`/`listen`/`getUserMedia`/
    // `addModule` would let the async start finish against a torn-down
    // instance. A fresh startRecording clears this flag before its first
    // await, so it never wrongly aborts a subsequent start.
    startAbortedRef.current = true;
    // Flush any remaining audio before teardown, then stop the native engine.
    // Scope the stop to THIS hook instance's session. `cleanup` runs as every
    // instance's unmount handler, so an unscoped stop here would let a
    // non-recording composer (e.g. a thread reply composer closing) tear down
    // the singleton engine owned by another composer that is actively
    // recording. `nativeSessionRef.current` is 0 for an instance that never
    // started a session (native IDs start at 1), so its stop can never match
    // the live session and correctly no-ops.
    const stoppingSession = nativeSessionRef.current;
    void flushAudioBatch().then(() => {
      invoke("stop_dictation", { session: stoppingSession }).catch(() => {});
    });
    // Stop batch timer.
    if (batchTimerRef.current) {
      clearInterval(batchTimerRef.current);
      batchTimerRef.current = null;
    }
    // Stop mic.
    if (streamRef.current) {
      for (const track of streamRef.current.getTracks()) {
        track.stop();
      }
      streamRef.current = null;
    }
    // Disconnect audio worklet. Clear the port handler first so any PCM
    // messages still queued on the main thread are dropped instead of
    // appended to the (reused) audio batch.
    if (workletRef.current) {
      workletRef.current.port.onmessage = null;
      workletRef.current.disconnect();
      workletRef.current = null;
    }
    if (outputGainRef.current) {
      outputGainRef.current.disconnect();
      outputGainRef.current = null;
    }
    // Drop any audio still buffered so it can't leak into the next session.
    audioBatchRef.current = [];
    // Close audio context.
    if (audioContextRef.current) {
      void audioContextRef.current.close();
      audioContextRef.current = null;
    }
    // Unlisten events.
    if (unlistenTranscriptRef.current) {
      unlistenTranscriptRef.current();
      unlistenTranscriptRef.current = null;
    }
    if (unlistenStateRef.current) {
      unlistenStateRef.current();
      unlistenStateRef.current = null;
    }
  }, [flushAudioBatch]);

  // Cleanup on unmount.
  useEffect(() => cleanup, [cleanup]);

  const startRecording = useCallback(async () => {
    // Also guard on `isTranscribing`: after `stopRecording()` the previous
    // session has cleared `isRecording` but is still awaiting its native
    // `stopped` event, which delivers the final transcript before its
    // listeners are unregistered. Starting a new session in that window would
    // unlisten the old session's handlers (below) before its last words
    // arrived, dropping them. Wait for the prior session to fully stop.
    if (!isEnabled || isStarting || isRecording || isTranscribing) return;

    // Clear abort flag for this new start attempt.
    startAbortedRef.current = false;
    // Reset any leftover audio buffer so a stale batch from a prior session
    // can't be flushed into this new session/draft.
    audioBatchRef.current = [];
    // Reset the flush chain so this session's flushes don't chain behind a
    // prior session's (already-settled) tail.
    flushChainRef.current = Promise.resolve();

    setIsStarting(true);
    onRecordingStartRef.current?.();

    try {
      // 1. Start the native STT engine — returns the session ID used to tag events.
      const sessionId = await invoke<number>("start_dictation");
      nativeSessionRef.current = sessionId;

      // Bail if aborted during engine start.
      if (startAbortedRef.current) {
        invoke("stop_dictation", { session: sessionId }).catch(() => {});
        return;
      }

      // Unregister any lingering listeners from a previous session so they
      // can't match the new session ID via the shared ref.
      if (unlistenTranscriptRef.current) {
        unlistenTranscriptRef.current();
        unlistenTranscriptRef.current = null;
      }
      if (unlistenStateRef.current) {
        unlistenStateRef.current();
        unlistenStateRef.current = null;
      }

      // 2. Listen for transcript events from the native layer.
      // Each listener captures `sessionId` by value (closure) so it only
      // matches events from this specific session — immune to ref mutation.
      const unlistenTranscript = await listen<{
        text: string;
        session: number;
      }>(DICTATION_TRANSCRIPT_EVENT, (event) => {
        const { text, session } = event.payload;
        if (session !== sessionId) return;
        if (text) {
          onTranscriptTextRef.current(text);
        }
      });
      // Bail if stop/cancel was called while we were awaiting.
      if (startAbortedRef.current) {
        unlistenTranscript();
        invoke("stop_dictation", { session: sessionId }).catch(() => {});
        return;
      }
      unlistenTranscriptRef.current = unlistenTranscript;

      const unlistenState = await listen<{
        state: string;
        session: number;
      }>(DICTATION_STATE_EVENT, (event) => {
        const { state, session } = event.payload;
        if (session !== sessionId) return;
        if (state === "stopped") {
          setIsRecording(false);
          setIsTranscribing(false);
          // Tear down this instance's local capture pipeline. The native
          // engine is a singleton: when another mounted composer calls
          // `start_dictation`, it stops this session's engine, so we can
          // receive `stopped` without our own `stopRecording()`/`cleanup()`
          // having run. Without tearing down here, `streamRef`/`workletRef`/
          // `batchTimerRef` stay alive and this composer keeps the mic open,
          // pushing stale audio, until it unmounts. Don't re-invoke
          // `stop_dictation` — the native side already stopped (that's why
          // this event fired).
          if (batchTimerRef.current) {
            clearInterval(batchTimerRef.current);
            batchTimerRef.current = null;
          }
          if (streamRef.current) {
            for (const track of streamRef.current.getTracks()) {
              track.stop();
            }
            streamRef.current = null;
          }
          if (workletRef.current) {
            workletRef.current.port.onmessage = null;
            workletRef.current.disconnect();
            workletRef.current = null;
          }
          if (outputGainRef.current) {
            outputGainRef.current.disconnect();
            outputGainRef.current = null;
          }
          audioBatchRef.current = [];
          if (audioContextRef.current) {
            void audioContextRef.current.close();
            audioContextRef.current = null;
          }
          // Clean up event listeners now that the session is fully done.
          if (unlistenTranscriptRef.current) {
            unlistenTranscriptRef.current();
            unlistenTranscriptRef.current = null;
          }
          if (unlistenStateRef.current) {
            unlistenStateRef.current();
            unlistenStateRef.current = null;
          }
        }
      });
      // Bail if stop/cancel was called while we were awaiting.
      if (startAbortedRef.current) {
        unlistenTranscript();
        unlistenState();
        invoke("stop_dictation", { session: sessionId }).catch(() => {});
        return;
      }
      unlistenStateRef.current = unlistenState;

      // 3. Capture mic audio.
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      streamRef.current = stream;

      // Bail if aborted during mic permission prompt.
      if (startAbortedRef.current) {
        for (const track of stream.getTracks()) track.stop();
        streamRef.current = null;
        invoke("stop_dictation", { session: sessionId }).catch(() => {});
        return;
      }

      // 4. Set up AudioWorklet to send PCM to native layer.
      const audioContext = new AudioContext({ sampleRate: 48000 });
      audioContextRef.current = audioContext;

      // Resume if the WebView created the context suspended (autoplay policy).
      // A suspended context never pulls the worklet's `process()`, so no PCM
      // would reach `push_dictation_audio` while the UI shows an active
      // recording. Mirrors the huddle capture path (`huddle/lib/audioWorklet.ts`).
      if (audioContext.state === "suspended") {
        await audioContext.resume();
      }

      // Create a processor that accumulates audio frames and posts them
      // to the main thread. Batching happens on the main thread side via
      // a timer to reduce IPC overhead.
      const processorCode = `
        class DictationProcessor extends AudioWorkletProcessor {
          process(inputs) {
            const input = inputs[0];
            if (input && input[0] && input[0].length > 0) {
              this.port.postMessage(input[0].buffer);
            }
            return true;
          }
        }
        registerProcessor('dictation-processor', DictationProcessor);
      `;
      const blob = new Blob([processorCode], {
        type: "application/javascript",
      });
      const blobUrl = URL.createObjectURL(blob);
      try {
        await audioContext.audioWorklet.addModule(blobUrl);
      } finally {
        URL.revokeObjectURL(blobUrl);
      }

      // Bail if stop/cancel was called while the worklet module was loading.
      // Without this the worklet/flush timer would start and `isRecording`
      // would be set true, leaving dictation running after it was stopped.
      if (startAbortedRef.current) {
        for (const track of stream.getTracks()) track.stop();
        streamRef.current = null;
        void audioContext.close();
        audioContextRef.current = null;
        invoke("stop_dictation", { session: sessionId }).catch(() => {});
        return;
      }

      const source = audioContext.createMediaStreamSource(stream);
      const worklet = new AudioWorkletNode(audioContext, "dictation-processor");
      workletRef.current = worklet;

      // Accumulate audio frames in a batch array; a timer flushes to native.
      worklet.port.onmessage = (event: MessageEvent<ArrayBuffer>) => {
        audioBatchRef.current.push(new Float32Array(event.data));
      };

      // Start the batch flush timer (~10 IPC calls/s instead of ~375).
      batchTimerRef.current = setInterval(flushAudioBatch, AUDIO_BATCH_MS);

      const outputGain = audioContext.createGain();
      outputGain.gain.value = 0;
      outputGainRef.current = outputGain;

      source.connect(worklet);
      worklet.connect(outputGain);
      outputGain.connect(audioContext.destination);

      setIsRecording(true);
      setIsTranscribing(true);
    } catch (error) {
      // Tear down and stop the native engine if it was started but a later
      // step failed (e.g. mic permission denied, AudioWorklet setup error).
      // `cleanup()` performs the session-scoped stop, so it only tears down
      // this instance's own session — never another composer's.
      cleanup();
      setIsRecording(false);
      setIsTranscribing(false);

      const message =
        error instanceof Error ? error.message : "Local dictation failed";
      if (/not allowed|denied|permission/i.test(message)) {
        toast.error("Microphone access denied", {
          description:
            "Allow microphone access in System Settings to use dictation.",
        });
      } else if (/not found|no audio/i.test(message)) {
        toast.error("No microphone found", {
          description: "Connect a microphone and try again.",
        });
      } else if (/model not ready/i.test(message)) {
        toast.error("Voice model downloading", {
          description:
            "The speech model is still downloading. Try again shortly.",
        });
      } else {
        toast.error("Dictation failed", { description: message });
      }
    } finally {
      setIsStarting(false);
    }
  }, [
    cleanup,
    flushAudioBatch,
    isEnabled,
    isRecording,
    isStarting,
    isTranscribing,
  ]);

  const stopRecording = useCallback(() => {
    // Signal any in-flight startRecording to bail after its next await.
    startAbortedRef.current = true;
    // Stop batch timer immediately.
    if (batchTimerRef.current) {
      clearInterval(batchTimerRef.current);
      batchTimerRef.current = null;
    }
    // Stop mic and audio pipeline immediately so the user gets visual feedback.
    if (streamRef.current) {
      for (const track of streamRef.current.getTracks()) {
        track.stop();
      }
      streamRef.current = null;
    }
    if (workletRef.current) {
      // Clear the port handler so PCM messages still queued on the main
      // thread are dropped rather than appended to the batch after the
      // final flush below (and leaking into the next session).
      workletRef.current.port.onmessage = null;
      workletRef.current.disconnect();
      workletRef.current = null;
    }
    if (outputGainRef.current) {
      outputGainRef.current.disconnect();
      outputGainRef.current = null;
    }
    if (audioContextRef.current) {
      void audioContextRef.current.close();
      audioContextRef.current = null;
    }
    setIsRecording(false);
    // Flush remaining audio and THEN stop the native engine, ensuring the
    // final batch arrives before the engine shuts down and flushes its buffer.
    // isTranscribing stays true — cleared when `dictation-state: stopped` arrives.
    // Scope the stop to THIS session: if the user restarts during the flush
    // window, a new session may already be stored by the time this resolves —
    // passing the session keeps this stop from tearing down the new engine.
    const stoppingSession = nativeSessionRef.current;
    void flushAudioBatch().then(() => {
      invoke("stop_dictation", { session: stoppingSession }).catch(() => {});
    });
  }, [flushAudioBatch]);

  const cancelRecording = useCallback(() => {
    // Signal any in-flight startRecording to bail after its next await.
    startAbortedRef.current = true;
    // `cleanup()` performs the session-scoped native stop, so cancelling a
    // composer that isn't the active recorder can't tear down another
    // composer's session.
    cleanup();
    setIsRecording(false);
    setIsTranscribing(false);
  }, [cleanup]);

  const toggleRecording = useCallback(() => {
    if (isRecording || isStarting) {
      stopRecording();
      return;
    }
    void startRecording();
  }, [isRecording, isStarting, startRecording, stopRecording]);

  return {
    isEnabled,
    isRecording,
    isStarting,
    isTranscribing,
    startRecording,
    stopRecording,
    cancelRecording,
    toggleRecording,
  };
}
