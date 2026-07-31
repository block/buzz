import * as React from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { setupAudioWorklet } from "@/features/huddle/lib/audioWorklet";
import type { AudioWorkletHandle } from "@/features/huddle/lib/audioWorklet";
import {
  isDictationAvailable,
  startDictation,
  stopDictation,
} from "@/shared/api/dictation";

/**
 * Composer dictation.
 *
 *   mic button → getUserMedia → setupAudioWorklet(push_dictation_pcm)
 *     → Rust SttPipeline (on-device Parakeet)
 *     → "dictation-text" event → onTranscript(text)
 *
 * The audio path is the huddle worklet, reused rather than reimplemented, so
 * dictation and huddles resample and VAD-gate identically. Nothing is sent
 * over the network.
 */

export type DictationStatus = "idle" | "starting" | "recording" | "error";

/** How often to re-check for the background model download to finish. */
const MODEL_POLL_INTERVAL_MS = 15_000;

export type UseDictationResult = {
  /** True once the speech model has finished its background download. */
  isAvailable: boolean;
  status: DictationStatus;
  /** Human-readable reason the last start failed, or null. */
  error: string | null;
  toggle: () => void;
};

/**
 * @param onTranscript - Receives each finalized segment. Called on the main
 *   thread; the caller is responsible for inserting it into the editor.
 * @param disabled - Mirrors the composer's disabled state. Flipping this
 *   during a session stops it.
 */
export function useDictation(
  onTranscript: (text: string) => void,
  disabled = false,
): UseDictationResult {
  const [isAvailable, setIsAvailable] = React.useState(false);
  const [status, setStatus] = React.useState<DictationStatus>("idle");
  const [error, setError] = React.useState<string | null>(null);

  const workletRef = React.useRef<AudioWorkletHandle | null>(null);
  const streamRef = React.useRef<MediaStream | null>(null);
  const unlistenRef = React.useRef<UnlistenFn | null>(null);
  // Guards against an in-flight start() resolving after the user has already
  // toggled off — without it the worklet would be adopted with no way to stop.
  const sessionRef = React.useRef(0);

  const onTranscriptRef = React.useRef(onTranscript);
  onTranscriptRef.current = onTranscript;

  // A fresh install downloads the model in the background, so an initial
  // `false` is usually temporary. Re-poll until it flips, otherwise the mic
  // button would stay disabled for the rest of the session even after the
  // download lands. Polling stops for good once available.
  React.useEffect(() => {
    if (isAvailable) return;
    let cancelled = false;

    const poll = () => {
      void isDictationAvailable()
        .then((available) => {
          if (!cancelled && available) setIsAvailable(true);
        })
        .catch(() => {
          /* backend not ready — the next tick retries */
        });
    };

    poll();
    const timer = window.setInterval(poll, MODEL_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [isAvailable]);

  const teardown = React.useCallback(() => {
    sessionRef.current += 1;
    workletRef.current?.stop();
    workletRef.current = null;
    for (const track of streamRef.current?.getTracks() ?? []) track.stop();
    streamRef.current = null;
    unlistenRef.current?.();
    unlistenRef.current = null;
    void stopDictation().catch(() => {
      /* already stopped, or the backend is gone — nothing to recover */
    });
    setStatus("idle");
  }, []);

  const start = React.useCallback(async () => {
    const session = sessionRef.current + 1;
    sessionRef.current = session;
    setStatus("starting");
    setError(null);

    try {
      // Start the Rust pipeline before opening the mic so the very first PCM
      // batch has somewhere to land.
      await startDictation();

      const unlisten = await listen<string>("dictation-text", (event) => {
        const text = event.payload.trim();
        if (text) onTranscriptRef.current(text);
      });

      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const [track] = stream.getAudioTracks();
      if (!track) throw new Error("no audio track available");

      const worklet = await setupAudioWorklet(track, true, {
        command: "push_dictation_pcm",
        enablePtt: false,
      });

      // The user toggled off (or the composer was disabled) while we were
      // awaiting — discard everything this session created.
      if (sessionRef.current !== session) {
        unlisten();
        worklet.stop();
        for (const t of stream.getTracks()) t.stop();
        void stopDictation().catch(() => {});
        return;
      }

      unlistenRef.current = unlisten;
      streamRef.current = stream;
      workletRef.current = worklet;
      setStatus("recording");
    } catch (cause) {
      // A denied mic permission surfaces here as a DOMException.
      const message =
        cause instanceof Error ? cause.message : "could not start dictation";
      const isCurrent = sessionRef.current === session;
      // Tear down FIRST — it resets status to "idle", so setting the error
      // state before it would be immediately overwritten and the user would
      // get no feedback at all on a denied mic.
      teardown();
      if (isCurrent) {
        setError(message);
        setStatus("error");
      }
    }
  }, [teardown]);

  const toggle = React.useCallback(() => {
    if (disabled) return;
    if (status === "recording" || status === "starting") {
      teardown();
      return;
    }
    void start();
  }, [disabled, start, status, teardown]);

  // Stop when the composer is disabled mid-session (channel switch, timeout).
  React.useEffect(() => {
    if (disabled && workletRef.current) teardown();
  }, [disabled, teardown]);

  // Release the mic and the STT worker if the composer unmounts mid-session.
  React.useEffect(() => teardown, [teardown]);

  return { isAvailable, status, error, toggle };
}
