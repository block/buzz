import * as React from "react";

import { encodeVoiceNoteWav } from "./voiceNoteWav";

const MIME_CANDIDATES = [
  "audio/webm;codecs=opus",
  "audio/ogg;codecs=opus",
  "audio/mp4",
  "audio/webm",
] as const;

function supportedMimeType(): string | undefined {
  if (typeof MediaRecorder === "undefined") return undefined;
  return MIME_CANDIDATES.find((type) => MediaRecorder.isTypeSupported(type));
}

export type VoiceNoteRecording = {
  duration: number;
  file: File;
};

type RecordingSession = {
  cancelled: boolean;
  chunks: Blob[];
  context: AudioContext | null;
  recorder: MediaRecorder | null;
  resolveStop: ((recording: VoiceNoteRecording | null) => void) | null;
  startedAt: number;
  stream: MediaStream | null;
};

function releaseSessionAudio(session: RecordingSession) {
  session.stream?.getTracks().forEach((track) => {
    track.stop();
  });
  session.stream = null;
  const context = session.context;
  session.context = null;
  if (context) void context.close().catch(() => undefined);
}

export function useVoiceNoteRecorder() {
  const mountedRef = React.useRef(true);
  const sessionRef = React.useRef<RecordingSession | null>(null);
  const [status, setStatus] = React.useState<
    "idle" | "requesting" | "recording" | "processing"
  >("idle");
  const [elapsedSeconds, setElapsedSeconds] = React.useState(0);
  const [levels, setLevels] = React.useState<number[]>([]);
  const [error, setError] = React.useState<string | null>(null);

  const cancel = React.useCallback(() => {
    const session = sessionRef.current;
    if (!session) return;
    session.cancelled = true;
    sessionRef.current = null;
    session.resolveStop?.(null);
    session.resolveStop = null;
    const recorder = session.recorder;
    if (recorder && recorder.state !== "inactive") recorder.stop();
    releaseSessionAudio(session);
    if (mountedRef.current) {
      setStatus("idle");
      setElapsedSeconds(0);
    }
  }, []);

  const start = React.useCallback(async () => {
    if (status !== "idle" || sessionRef.current) return;
    setError(null);
    if (!navigator.mediaDevices?.getUserMedia || !window.MediaRecorder) {
      setError("Voice recording is not available in this environment.");
      return;
    }

    const session: RecordingSession = {
      cancelled: false,
      chunks: [],
      context: null,
      recorder: null,
      resolveStop: null,
      startedAt: 0,
      stream: null,
    };
    sessionRef.current = session;
    setStatus("requesting");

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      session.stream = stream;
      if (
        session.cancelled ||
        !mountedRef.current ||
        sessionRef.current !== session
      ) {
        releaseSessionAudio(session);
        return;
      }

      const mimeType = supportedMimeType();
      const recorder = mimeType
        ? new MediaRecorder(stream, { mimeType })
        : new MediaRecorder(stream);
      session.recorder = recorder;
      const context = new AudioContext();
      session.context = context;
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      analyser.smoothingTimeConstant = 0.72;
      context.createMediaStreamSource(stream).connect(analyser);
      session.startedAt = performance.now();
      setElapsedSeconds(0);
      setLevels([]);

      recorder.addEventListener("dataavailable", (event) => {
        if (event.data.size > 0) session.chunks.push(event.data);
      });
      recorder.addEventListener("stop", () => {
        void (async () => {
          const actualMime = recorder.mimeType || mimeType || "audio/webm";
          const blob = new Blob(session.chunks, { type: actualMime });
          session.chunks = [];
          let recording: VoiceNoteRecording | null = null;
          if (!session.cancelled && blob.size > 0) {
            try {
              const encoded = await blob.arrayBuffer();
              const decoded = await context.decodeAudioData(encoded.slice(0));
              if (
                !session.cancelled &&
                mountedRef.current &&
                sessionRef.current === session
              ) {
                const channels = Array.from(
                  { length: decoded.numberOfChannels },
                  (_, index) => decoded.getChannelData(index),
                );
                const wav = encodeVoiceNoteWav(channels, decoded.sampleRate);
                const wavBuffer = new ArrayBuffer(wav.byteLength);
                new Uint8Array(wavBuffer).set(wav);
                recording = {
                  duration: decoded.duration,
                  file: new File([wavBuffer], `voice-note-${Date.now()}.wav`, {
                    type: "audio/wav",
                  }),
                };
              }
            } catch {
              if (
                !session.cancelled &&
                mountedRef.current &&
                sessionRef.current === session
              ) {
                setError("Buzz could not prepare this voice note for upload.");
              }
            }
          }
          releaseSessionAudio(session);
          if (sessionRef.current === session) {
            sessionRef.current = null;
            if (mountedRef.current) {
              setStatus("idle");
              setElapsedSeconds(0);
            }
          }
          session.resolveStop?.(recording);
          session.resolveStop = null;
        })();
      });
      recorder.addEventListener("error", () => {
        if (mountedRef.current && sessionRef.current === session) {
          setError("The voice recording was interrupted.");
        }
      });
      recorder.start(250);
      setStatus("recording");

      const samples = new Uint8Array(analyser.fftSize);
      const levelTimer = window.setInterval(() => {
        if (
          recorder.state !== "recording" ||
          session.cancelled ||
          sessionRef.current !== session
        ) {
          window.clearInterval(levelTimer);
          return;
        }
        analyser.getByteTimeDomainData(samples);
        let sumSquares = 0;
        for (const sample of samples) {
          const centered = (sample - 128) / 128;
          sumSquares += centered * centered;
        }
        const rms = Math.sqrt(sumSquares / samples.length);
        const level = Math.min(1, rms * 5.5);
        if (!mountedRef.current) return;
        setLevels((previous) => [...previous, level]);
        setElapsedSeconds((performance.now() - session.startedAt) / 1000);
      }, 90);
    } catch (cause) {
      releaseSessionAudio(session);
      if (
        session.cancelled ||
        !mountedRef.current ||
        sessionRef.current !== session
      ) {
        return;
      }
      sessionRef.current = null;
      setStatus("idle");
      const denied =
        cause instanceof DOMException &&
        (cause.name === "NotAllowedError" || cause.name === "SecurityError");
      setError(
        denied
          ? "Allow Buzz to access your microphone to record a voice note."
          : "Buzz could not start the voice recorder.",
      );
    }
  }, [status]);

  const stop = React.useCallback(
    (discard = false): Promise<VoiceNoteRecording | null> => {
      if (discard) {
        cancel();
        return Promise.resolve(null);
      }
      const session = sessionRef.current;
      const recorder = session?.recorder;
      if (!session || !recorder || recorder.state === "inactive") {
        return Promise.resolve(null);
      }
      setStatus("processing");
      return new Promise((resolve) => {
        session.resolveStop = resolve;
        recorder.stop();
      });
    },
    [cancel],
  );

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      cancel();
    };
  }, [cancel]);

  return {
    cancel,
    elapsedSeconds,
    error,
    levels,
    start,
    status,
    stop,
  };
}
