import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import type { Editor } from "@tiptap/react";
import { toast } from "sonner";

import { buildDictationInsertion } from "@/features/messages/lib/dictationInsertion";
import { invokeTauri } from "@/shared/api/tauri";
import {
  type PcmCaptureHandle,
  setupPcmCapture,
} from "@/shared/lib/pcmCapture";

export type ComposerDictationStatus =
  | "idle"
  | "starting"
  | "recording"
  | "stopping";

type DictationTranscript = {
  sessionId: number;
  text: string;
};

type ActiveOwner = {
  id: symbol;
  release: () => void;
};

let activeOwner: ActiveOwner | null = null;

// The STT worker flushes after ~300 ms of silence. Keep capture open briefly
// after Stop so clicking immediately after the last word does not drop it.
const FINAL_TRANSCRIPT_GRACE_MS = 450;

function stopNativeDictation(sessionId: number): void {
  void invokeTauri("stop_dictation", { sessionId }).catch(() => {
    // Session cleanup is best-effort; native generation guards prevent a stale
    // session from writing into another composer.
  });
}

function dictationErrorMessage(error: unknown): string {
  if (error instanceof DOMException && error.name === "NotAllowedError") {
    return "Microphone access was denied. Allow Buzz to use the microphone in system settings, then try again.";
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  return "Voice input could not start.";
}

function insertTranscript(editor: Editor, transcript: string): boolean {
  const { from, to } = editor.state.selection;
  const previousCharacter =
    from > 1 ? editor.state.doc.textBetween(from - 1, from, "\n", "\n") : "";
  const insertion = buildDictationInsertion(previousCharacter, transcript);
  if (!insertion) return false;
  return editor
    .chain()
    .focus()
    .insertContentAt({ from, to }, insertion)
    .scrollIntoView()
    .run();
}

export function useComposerDictation({
  disabled,
  editor,
  sessionKey,
}: {
  disabled: boolean;
  editor: Editor | null;
  sessionKey?: string | null;
}) {
  const [status, setStatus] = React.useState<ComposerDictationStatus>("idle");
  const activeSessionRef = React.useRef<number | null>(null);
  const captureRef = React.useRef<PcmCaptureHandle | null>(null);
  const streamRef = React.useRef<MediaStream | null>(null);
  const operationRef = React.useRef(0);
  const instanceIdRef = React.useRef(Symbol("composer-dictation"));
  const sessionKeyRef = React.useRef(sessionKey);
  const editorRef = React.useRef(editor);
  editorRef.current = editor;

  const stopCapture = React.useCallback(() => {
    captureRef.current?.stop();
    captureRef.current = null;
    for (const track of streamRef.current?.getTracks() ?? []) {
      track.stop();
    }
    streamRef.current = null;
  }, []);

  const releaseImmediately = React.useCallback(
    (updateState: boolean) => {
      operationRef.current += 1;
      stopCapture();
      const sessionId = activeSessionRef.current;
      activeSessionRef.current = null;
      if (sessionId !== null) {
        stopNativeDictation(sessionId);
      }
      if (activeOwner?.id === instanceIdRef.current) {
        activeOwner = null;
      }
      if (updateState) setStatus("idle");
    },
    [stopCapture],
  );

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<DictationTranscript>("dictation-transcript", (event) => {
      if (event.payload.sessionId !== activeSessionRef.current) return;
      const activeEditor = editorRef.current;
      if (activeEditor) insertTranscript(activeEditor, event.payload.text);
    })
      .then((dispose) => {
        if (disposed) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch((error) => {
        console.error("Failed to listen for dictation transcripts:", error);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    if (disabled) releaseImmediately(true);
  }, [disabled, releaseImmediately]);

  // A MessageComposer can survive channel/thread navigation. Never let a
  // transcript started in one draft land in the next draft key.
  React.useEffect(() => {
    if (sessionKeyRef.current === sessionKey) return;
    sessionKeyRef.current = sessionKey;
    releaseImmediately(true);
  }, [sessionKey, releaseImmediately]);

  React.useEffect(
    () => () => {
      releaseImmediately(false);
    },
    [releaseImmediately],
  );

  const start = React.useCallback(async () => {
    if (disabled || !editorRef.current) return;

    if (activeOwner?.id !== instanceIdRef.current) {
      activeOwner?.release();
    }
    releaseImmediately(false);
    activeOwner = {
      id: instanceIdRef.current,
      release: () => releaseImmediately(true),
    };
    const operation = operationRef.current + 1;
    operationRef.current = operation;
    setStatus("starting");

    let sessionId: number | null = null;
    let stream: MediaStream | null = null;
    try {
      sessionId = await invokeTauri<number>("start_dictation");
      if (operation !== operationRef.current) {
        stopNativeDictation(sessionId);
        return;
      }
      activeSessionRef.current = sessionId;

      if (!navigator.mediaDevices?.getUserMedia) {
        throw new Error("Microphone capture is unavailable on this system.");
      }
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          sampleRate: 48000,
        },
      });
      const [audioTrack] = stream.getAudioTracks();
      if (!audioTrack) {
        throw new Error("No microphone input is available.");
      }
      const capture = await setupPcmCapture(
        audioTrack,
        "push_dictation_audio_pcm",
      );
      if (operation !== operationRef.current) {
        capture.stop();
        for (const track of stream.getTracks()) track.stop();
        stopNativeDictation(sessionId);
        return;
      }
      streamRef.current = stream;
      captureRef.current = capture;
      setStatus("recording");
    } catch (error) {
      for (const track of stream?.getTracks() ?? []) track.stop();
      if (sessionId !== null) {
        stopNativeDictation(sessionId);
      }
      if (operation === operationRef.current) {
        activeSessionRef.current = null;
        if (activeOwner?.id === instanceIdRef.current) activeOwner = null;
        setStatus("idle");
        toast.error(dictationErrorMessage(error));
      }
    }
  }, [disabled, releaseImmediately]);

  const stop = React.useCallback(async () => {
    const sessionId = activeSessionRef.current;
    if (sessionId === null) {
      releaseImmediately(true);
      return;
    }

    operationRef.current += 1;
    setStatus("stopping");
    await new Promise((resolve) =>
      window.setTimeout(resolve, FINAL_TRANSCRIPT_GRACE_MS),
    );
    if (activeSessionRef.current !== sessionId) return;

    stopCapture();
    try {
      await invokeTauri("stop_dictation", { sessionId });
    } catch (error) {
      console.error("Failed to stop dictation:", error);
    } finally {
      if (activeSessionRef.current === sessionId) {
        activeSessionRef.current = null;
        if (activeOwner?.id === instanceIdRef.current) activeOwner = null;
        setStatus("idle");
        editorRef.current?.commands.focus();
      }
    }
  }, [releaseImmediately, stopCapture]);

  const toggle = React.useCallback(() => {
    if (status === "idle") {
      void start();
    } else if (status === "recording") {
      void stop();
    }
  }, [start, status, stop]);

  return {
    isActive: status !== "idle",
    status,
    toggle,
  };
}
