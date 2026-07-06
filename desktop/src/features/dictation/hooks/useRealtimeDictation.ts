import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import {
  createTranscribeSession,
  getTranscribeStatus,
} from "../api/transcribeSession";
import {
  type AudioBufferCapture,
  type TranscriptEvent,
  type TranscriptSegmentState,
  BUFFER_COMMITTED_EVENT,
  TRANSCRIPT_COMPLETED_EVENT,
  TRANSCRIPT_DELTA_EVENT,
  commitAudioBuffer,
  connectPeerConnection,
  createAudioBufferCapture,
  createPeerConnection,
  createTranscriptSegmentState,
  flushAudioBuffer,
  getTranscriptText,
  mergeTranscriptEvent,
  requiresManualCommit,
} from "../lib/realtimeAudio";

interface UseRealtimeDictationOptions {
  disabled?: boolean;
  onRecordingStart?: () => void;
  onTranscriptText: (text: string) => void;
}

function closeResources(resources: {
  audioCapture?: AudioBufferCapture | null;
  dataChannel?: RTCDataChannel | null;
  peerConnection?: RTCPeerConnection | null;
  stream?: MediaStream | null;
}) {
  resources.audioCapture?.close();
  resources.dataChannel?.close();
  resources.peerConnection?.close();
  for (const track of resources.stream?.getTracks() ?? []) {
    track.stop();
  }
}

export function useRealtimeDictation({
  disabled = false,
  onRecordingStart,
  onTranscriptText,
}: UseRealtimeDictationOptions) {
  const [isRecording, setIsRecording] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [isConfigured, setIsConfigured] = useState(false);

  const peerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const dataChannelRef = useRef<RTCDataChannel | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const audioCaptureRef = useRef<AudioBufferCapture | null>(null);
  const segmentStateRef = useRef<TranscriptSegmentState>(
    createTranscriptSegmentState(),
  );
  const activeRunIdRef = useRef(0);
  const manualCommitRef = useRef(false);
  const commitIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const onRecordingStartRef = useRef(onRecordingStart);
  const onTranscriptTextRef = useRef(onTranscriptText);

  onRecordingStartRef.current = onRecordingStart;
  onTranscriptTextRef.current = onTranscriptText;

  const isEnabled = !disabled && isConfigured;

  // Check if transcription is configured on mount
  useEffect(() => {
    let cancelled = false;
    getTranscribeStatus()
      .then((status) => {
        if (!cancelled) setIsConfigured(status.configured);
      })
      .catch(() => {
        if (!cancelled) setIsConfigured(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /**
   * Tear down recording resources.
   *
   * @param invalidateRun - When true, immediately marks the run stale so
   *   late transcript events are rejected (used by send/edit-save/navigation).
   *   When false (user-initiated stop), the run stays valid during the grace
   *   window so the final commit's transcript is delivered to the composer.
   */
  const cleanupResources = useCallback((invalidateRun = true) => {
    // Clear periodic commit interval if active.
    if (commitIntervalRef.current) {
      clearInterval(commitIntervalRef.current);
      commitIntervalRef.current = null;
    }

    // For manual-commit models (no server VAD), commit any buffered audio
    // before tearing down the connection so OpenAI processes the final chunk.
    // We keep the data channel open briefly to receive the transcript response.
    const dc = dataChannelRef.current;
    const needsCommit =
      manualCommitRef.current && dc && dc.readyState === "open";
    manualCommitRef.current = false;

    // Send final commit for manual-commit models.
    if (needsCommit && dc) {
      commitAudioBuffer(dc);
    }

    // Determine whether we need a grace window before full teardown.
    // User-initiated stop (!invalidateRun) needs a grace period for BOTH:
    // - Manual-commit models: final commit's transcript response
    // - Server-VAD models: in-flight VAD completion events
    const needsGrace = !invalidateRun && dc && dc.readyState === "open";

    if (needsGrace && dc) {
      // Stop the mic immediately so no new audio is sent.
      for (const track of streamRef.current?.getTracks() ?? []) {
        track.stop();
      }
      streamRef.current = null;
      audioCaptureRef.current?.close();
      audioCaptureRef.current = null;
      // Delay WebRTC teardown briefly so final transcript events arrive.
      const pc = peerConnectionRef.current;
      peerConnectionRef.current = null;
      dataChannelRef.current = null;
      const runId = activeRunIdRef.current;

      setTimeout(() => {
        // After the grace window, invalidate the run and tear down.
        if (activeRunIdRef.current === runId) {
          activeRunIdRef.current += 1;
        }
        dc.close();
        pc?.close();
      }, 3000);
      return;
    }

    // Immediate teardown: send/navigation cancellation, or no open channel.
    activeRunIdRef.current += 1;
    closeResources({
      audioCapture: audioCaptureRef.current,
      dataChannel: dataChannelRef.current,
      peerConnection: peerConnectionRef.current,
      stream: streamRef.current,
    });
    audioCaptureRef.current = null;
    dataChannelRef.current = null;
    peerConnectionRef.current = null;
    streamRef.current = null;
  }, []);

  /** Full cleanup — invalidates the run (used by send/navigation). */
  const cleanup = useCallback(() => {
    cleanupResources(true);
    setIsRecording(false);
    setIsStarting(false);
    setIsTranscribing(false);
  }, [cleanupResources]);

  /** User-initiated stop — preserves final transcript for manual-commit models. */
  const userStop = useCallback(() => {
    cleanupResources(false);
    setIsRecording(false);
    setIsStarting(false);
    // Note: isTranscribing stays true briefly while the final transcript arrives.
  }, [cleanupResources]);

  useEffect(() => cleanupResources, [cleanupResources]);

  const handleRealtimeEvent = useCallback(
    (runId: number, event: TranscriptEvent) => {
      // Ignore events from a stale run (e.g. user sent/stopped while
      // transcripts were still in-flight from the data channel).
      if (activeRunIdRef.current !== runId) return;

      if (event.type === "error") {
        console.error("OpenAI realtime server error", event);
        toast.error(event.error?.message ?? "Voice input error");
        return;
      }

      if (
        event.type !== TRANSCRIPT_DELTA_EVENT &&
        event.type !== TRANSCRIPT_COMPLETED_EVENT &&
        event.type !== BUFFER_COMMITTED_EVENT
      ) {
        return;
      }

      const prevText = getTranscriptText(segmentStateRef.current);
      const merged = mergeTranscriptEvent(segmentStateRef.current, event);

      if (merged === prevText) return;

      onTranscriptTextRef.current(merged);
      setIsTranscribing(event.type !== TRANSCRIPT_COMPLETED_EVENT);
    },
    [],
  );

  const startRecording = useCallback(async () => {
    if (!isEnabled || isStarting || isRecording) return;

    const runId = activeRunIdRef.current + 1;
    activeRunIdRef.current = runId;
    const isStaleRun = () => activeRunIdRef.current !== runId;

    let stream: MediaStream | null = null;
    let audioCapture: AudioBufferCapture | null = null;
    let peerConnection: RTCPeerConnection | null = null;
    let dataChannel: RTCDataChannel | null = null;

    setIsStarting(true);
    segmentStateRef.current = createTranscriptSegmentState();
    onRecordingStartRef.current?.();

    try {
      // 1. Capture mic immediately for instant feedback
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          autoGainControl: true,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      if (isStaleRun()) {
        closeResources({ stream });
        return;
      }
      streamRef.current = stream;
      setIsRecording(true);

      // 2. Buffer PCM via AudioWorklet while network calls proceed
      audioCapture = await createAudioBufferCapture(stream);
      if (isStaleRun()) {
        closeResources({ audioCapture, stream });
        return;
      }
      audioCaptureRef.current = audioCapture;

      // 3. Create session via relay
      const session = await createTranscribeSession();
      if (isStaleRun()) {
        closeResources({ audioCapture, stream });
        return;
      }
      manualCommitRef.current = requiresManualCommit(session.model);

      // 4. Set up WebRTC
      peerConnection = createPeerConnection();
      peerConnectionRef.current = peerConnection;
      const activeStream = stream;
      stream.getAudioTracks().forEach((track) => {
        peerConnection?.addTrack(track, activeStream);
      });

      dataChannel = peerConnection.createDataChannel("oai-events");
      dataChannelRef.current = dataChannel;
      dataChannel.addEventListener("message", (message) => {
        try {
          handleRealtimeEvent(runId, JSON.parse(String(message.data)));
        } catch {
          // Ignore non-JSON events
        }
      });

      // Flush buffered audio once data channel opens
      const channelToFlush = dataChannel;
      const captureToFlush = audioCapture;
      const useManualCommit = manualCommitRef.current;
      dataChannel.addEventListener("open", () => {
        // If the user stopped (or restarted) recording between the SDP
        // exchange and the channel opening, drop this run's buffered audio.
        if (isStaleRun()) {
          captureToFlush.close();
          return;
        }
        flushAudioBuffer(channelToFlush, captureToFlush.chunks);
        // For manual-commit models, commit the initial buffered audio and
        // start a periodic commit interval so streaming transcripts flow
        // during recording (server VAD models commit automatically).
        if (useManualCommit) {
          commitAudioBuffer(channelToFlush);
          // Commit every 2s to produce streaming transcript segments while
          // the user is still speaking. Each commit triggers a transcription
          // of the audio accumulated since the last commit.
          commitIntervalRef.current = setInterval(() => {
            if (channelToFlush.readyState === "open") {
              commitAudioBuffer(channelToFlush);
            }
          }, 2000);
        }
        captureToFlush.close();
        audioCaptureRef.current = null;
      });

      // 5. SDP exchange
      await connectPeerConnection({
        peerConnection,
        clientSecret: session.clientSecret,
      });
      if (isStaleRun()) {
        closeResources({ audioCapture, dataChannel, peerConnection, stream });
        return;
      }
    } catch (error) {
      closeResources({ audioCapture, dataChannel, peerConnection, stream });
      if (!isStaleRun()) {
        audioCaptureRef.current = null;
        dataChannelRef.current = null;
        peerConnectionRef.current = null;
        streamRef.current = null;
        setIsRecording(false);
        setIsTranscribing(false);

        const message =
          error instanceof Error ? error.message : "Voice input failed";
        if (/not allowed|denied|permission/i.test(message)) {
          toast.error("Microphone access denied", {
            description:
              "Allow microphone access in System Settings to use dictation.",
          });
        } else if (/not found|no audio/i.test(message)) {
          toast.error("No microphone found", {
            description: "Connect a microphone and try again.",
          });
        } else {
          toast.error("Voice input failed", { description: message });
        }
      }
    } finally {
      if (!isStaleRun()) setIsStarting(false);
    }
  }, [handleRealtimeEvent, isEnabled, isRecording, isStarting]);

  /** User-initiated stop (mic button) — preserves final transcript. */
  const stopRecording = useCallback(() => userStop(), [userStop]);

  /**
   * Cancel recording and reject all pending transcripts. Used by send,
   * edit-save, and navigation to prevent late events from refilling the
   * composer after the content has been dispatched.
   */
  const cancelRecording = useCallback(() => cleanup(), [cleanup]);

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
