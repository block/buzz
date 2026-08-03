import { listen } from "@tauri-apps/api/event";
import {
  Captions,
  LoaderCircle,
  Mic,
  MicOff,
  PhoneOff,
  RotateCcw,
  X,
} from "lucide-react";
import * as React from "react";

import { voiceRoomAudio } from "@/features/agents/voiceRoomAudio";
import {
  appendVoiceRoomTranscript,
  releaseVoiceRoomSpeaker,
  routeVoiceRoomTurn,
  setVoiceTargetMuted,
  type CodexVoiceTarget,
  updateVoiceSessionState,
  useCodexVoiceTargets,
  useVoiceRoomOutputMuted,
  useVoiceRoomDirectedTurns,
  useVoiceRoomSpeakerLease,
} from "@/features/agents/voiceSessionRegistry";
import { getThreadReference } from "@/features/messages/lib/threading";
import {
  type CodexVoiceEvent,
  requestMicrophoneAccess,
  setCodexVoiceMuted,
  speakCodexVoice,
  startCodexVoice,
  stopCodexVoice,
} from "@/shared/api/codexVoice";
import { relayClient } from "@/shared/api/relayClient";
import { sendChannelMessage } from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type VoicePhase = "starting" | "listening" | "ending" | "error";

type MeterBar = {
  id: string;
  value: number;
};

type VoiceTranscriptEntry = {
  id: number;
  role: "assistant" | "user";
  text: string;
};

type CodexVoiceDockProps = {
  onEnded: (threadId: string) => void;
  target: CodexVoiceTarget;
};

const DEFAULT_LEVELS: MeterBar[] = [
  { id: "outer-left", value: 0.18 },
  { id: "inner-left", value: 0.28 },
  { id: "center", value: 0.2 },
  { id: "inner-right", value: 0.34 },
  { id: "outer-right", value: 0.16 },
];

const NATIVE_RESPONSE_TIMEOUT_MS = 45_000;

export function CodexVoiceDock({ onEnded, target }: CodexVoiceDockProps) {
  const { agentName, agentPubkey, channelId, mode, relayUrl, threadId, voice } =
    target;
  const [model, setModel] = React.useState("gpt-live-1-codex");
  const roomOutputMuted = useVoiceRoomOutputMuted();
  const activeTargets = useCodexVoiceTargets();
  const directedTurns = useVoiceRoomDirectedTurns();
  const speakerLease = useVoiceRoomSpeakerLease();
  const isOrchestrator = agentName.trim().toLowerCase() === "orchestrator";
  const capturesRoomInput =
    isOrchestrator ||
    (activeTargets.length === 1 && activeTargets[0]?.threadId === threadId);
  const [phase, setPhase] = React.useState<VoicePhase>("starting");
  const [error, setError] = React.useState<string | null>(null);
  const [muted, setMuted] = React.useState(false);
  const [transcript, setTranscript] = React.useState<string | null>(null);
  const [transcriptExpanded, setTranscriptExpanded] = React.useState(false);
  const [transcriptHistory, setTranscriptHistory] = React.useState<
    VoiceTranscriptEntry[]
  >([]);
  const [levels, setLevels] = React.useState(DEFAULT_LEVELS);
  const streamRef = React.useRef<MediaStream | null>(null);
  const peerRef = React.useRef<RTCPeerConnection | null>(null);
  const remoteAudioRef = React.useRef<HTMLAudioElement | null>(null);
  const analyserContextRef = React.useRef<AudioContext | null>(null);
  const analyserFrameRef = React.useRef<number | null>(null);
  const phaseRef = React.useRef<VoicePhase>(phase);
  const startedRef = React.useRef(false);
  const proxyRootIdsRef = React.useRef(new Set<string>());
  const spokenProxyMessagesRef = React.useRef(new Set<string>());
  const proxySpeechQueueRef = React.useRef(Promise.resolve());
  const pendingDirectedReplyRef = React.useRef(false);
  const recoveringRef = React.useRef(false);
  const responseWatchdogRef = React.useRef<number | null>(null);
  const restartVoiceRef = React.useRef<() => void>(() => undefined);
  const transcriptSequenceRef = React.useRef(0);
  const handledDirectedTurnRef = React.useRef(0);
  const speakerLeaseRef = React.useRef(speakerLease);
  const transportMutedRef = React.useRef<boolean | null>(null);
  phaseRef.current = phase;
  speakerLeaseRef.current = speakerLease;

  React.useEffect(() => {
    updateVoiceSessionState(threadId, {
      error,
      muted,
      phase,
      transcript,
    });
  }, [error, muted, phase, threadId, transcript]);

  const clearResponseWatchdog = React.useCallback(() => {
    if (responseWatchdogRef.current === null) return;
    window.clearTimeout(responseWatchdogRef.current);
    responseWatchdogRef.current = null;
  }, []);

  const releaseMedia = React.useCallback(() => {
    clearResponseWatchdog();
    if (analyserFrameRef.current !== null) {
      cancelAnimationFrame(analyserFrameRef.current);
      analyserFrameRef.current = null;
    }
    if (analyserContextRef.current) {
      void analyserContextRef.current.close();
      analyserContextRef.current = null;
    }
    peerRef.current?.close();
    peerRef.current = null;
    voiceRoomAudio.leave(threadId);
    releaseVoiceRoomSpeaker(threadId);
    streamRef.current = null;
    if (remoteAudioRef.current) remoteAudioRef.current.srcObject = null;
    setMuted(false);
    setTranscript(null);
    setTranscriptExpanded(false);
    setTranscriptHistory([]);
    setLevels(DEFAULT_LEVELS);
  }, [clearResponseWatchdog, threadId]);

  React.useEffect(() => {
    if (mode !== "proxy") return;
    let disposed = false;
    let unsubscribe: (() => Promise<void>) | undefined;
    void relayClient
      .subscribeToChannelLive(channelId, (event) => {
        if (
          disposed ||
          event.pubkey.toLowerCase() !== agentPubkey.toLowerCase() ||
          spokenProxyMessagesRef.current.has(event.id)
        ) {
          return;
        }
        const reference = getThreadReference(event.tags);
        const belongsToVoiceTurn =
          (reference.rootId && proxyRootIdsRef.current.has(reference.rootId)) ||
          (reference.parentId &&
            proxyRootIdsRef.current.has(reference.parentId));
        if (
          (!belongsToVoiceTurn && !pendingDirectedReplyRef.current) ||
          !event.content.trim()
        ) {
          return;
        }

        pendingDirectedReplyRef.current = false;
        spokenProxyMessagesRef.current.add(event.id);
        proxySpeechQueueRef.current = proxySpeechQueueRef.current
          .catch(() => undefined)
          .then(() => speakCodexVoice(threadId, event.content))
          .catch((speechError) => {
            if (!disposed) {
              releaseVoiceRoomSpeaker(threadId);
              setError(
                formatError(
                  speechError,
                  `${agentName}'s voice reply could not play.`,
                ),
              );
            }
          });
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
        } else {
          unsubscribe = dispose;
        }
      })
      .catch((subscriptionError) => {
        if (!disposed) {
          setError(
            formatError(
              subscriptionError,
              `Buzz could not listen for ${agentName}'s replies.`,
            ),
          );
        }
      });
    return () => {
      disposed = true;
      if (unsubscribe) void unsubscribe();
    };
  }, [agentName, agentPubkey, channelId, mode, threadId]);

  const sendProxyTurn = React.useCallback(
    (text: string) => {
      pendingDirectedReplyRef.current = true;
      return sendChannelMessage(channelId, text, null, undefined, [agentPubkey])
        .then((result) => {
          proxyRootIdsRef.current.add(result.rootEventId ?? result.eventId);
        })
        .catch((sendError) => {
          pendingDirectedReplyRef.current = false;
          releaseVoiceRoomSpeaker(threadId);
          setError(
            formatError(
              sendError,
              `${agentName} could not receive the voice turn.`,
            ),
          );
        });
    },
    [agentName, agentPubkey, channelId, threadId],
  );

  React.useEffect(() => {
    if (mode !== "proxy") return;
    const turn = directedTurns.at(-1);
    if (
      !turn ||
      turn.id <= handledDirectedTurnRef.current ||
      turn.recipientThreadId !== threadId
    ) {
      return;
    }
    handledDirectedTurnRef.current = turn.id;
    void sendProxyTurn(turn.text);
  }, [directedTurns, mode, sendProxyTurn, threadId]);

  const removeDock = React.useCallback(() => {
    phaseRef.current = "ending";
    releaseMedia();
    onEnded(threadId);
  }, [onEnded, releaseMedia, threadId]);

  const finishVoice = React.useCallback(async () => {
    if (phaseRef.current === "ending") return;
    phaseRef.current = "ending";
    setPhase("ending");
    try {
      await stopCodexVoice(threadId);
    } finally {
      releaseMedia();
      onEnded(threadId);
    }
  }, [onEnded, releaseMedia, threadId]);

  React.useEffect(() => {
    let cancelled = false;
    const unlisten = listen<CodexVoiceEvent>("codex-voice-event", (event) => {
      if (
        cancelled ||
        (event.payload.params.threadId &&
          event.payload.params.threadId !== threadId)
      ) {
        return;
      }
      const { method, params } = event.payload;
      if (method === "thread/realtime/sdp" && params.sdp) {
        void peerRef.current
          ?.setRemoteDescription({ type: "answer", sdp: params.sdp })
          .catch((rtcError) => {
            setPhase("error");
            setError(formatError(rtcError, "Voice audio could not connect."));
          });
      } else if (method === "thread/realtime/started") {
        recoveringRef.current = false;
        setPhase("listening");
      } else if (method === "thread/realtime/transcript/delta") {
        setTranscript((current) => `${current ?? ""}${params.delta ?? ""}`);
      } else if (method === "thread/realtime/transcript/done") {
        const completedTranscript = params.text?.trim() || null;
        setTranscript(completedTranscript);
        if (completedTranscript) {
          const role = params.role === "assistant" ? "assistant" : "user";
          const ownsFloor =
            role !== "assistant" ||
            !speakerLeaseRef.current ||
            speakerLeaseRef.current.threadId === threadId;
          if (
            (role === "user" && isOrchestrator) ||
            (role === "assistant" && ownsFloor)
          ) {
            appendVoiceRoomTranscript({
              speakerName: role === "assistant" ? agentName : "You",
              speakerType: role === "assistant" ? "agent" : "human",
              text: completedTranscript,
            });
          }
          transcriptSequenceRef.current += 1;
          setTranscriptHistory((current) => [
            ...current.slice(-49),
            {
              id: transcriptSequenceRef.current,
              role,
              text: completedTranscript,
            },
          ]);
        }
        if (params.role === "assistant") {
          clearResponseWatchdog();
          if (speakerLeaseRef.current?.threadId === threadId) {
            window.setTimeout(() => releaseVoiceRoomSpeaker(threadId), 250);
          }
        } else if (params.text?.trim()) {
          if (isOrchestrator) {
            routeVoiceRoomTurn(params.text.trim());
          } else if (capturesRoomInput) {
            void sendProxyTurn(params.text.trim());
          }
          if (mode === "native" && isOrchestrator) {
            clearResponseWatchdog();
            responseWatchdogRef.current = window.setTimeout(() => {
              responseWatchdogRef.current = null;
              if (phaseRef.current === "ending") return;
              recoveringRef.current = true;
              setPhase("starting");
              setError("Orchestrator took too long. Reconnecting voice…");
              releaseVoiceRoomSpeaker(threadId);
              void stopCodexVoice(threadId).finally(() => {
                if (phaseRef.current !== "ending") restartVoiceRef.current();
              });
            }, NATIVE_RESPONSE_TIMEOUT_MS);
          }
        }
      } else if (method === "thread/realtime/error") {
        recoveringRef.current = false;
        setPhase("error");
        setError(params.message || "Codex Voice encountered an error.");
        releaseMedia();
        void stopCodexVoice(threadId);
      } else if (method === "thread/realtime/closed") {
        if (recoveringRef.current) return;
        void stopCodexVoice(threadId).finally(removeDock);
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((dispose) => dispose());
    };
  }, [
    agentName,
    capturesRoomInput,
    clearResponseWatchdog,
    isOrchestrator,
    mode,
    releaseMedia,
    removeDock,
    sendProxyTurn,
    threadId,
  ]);

  React.useEffect(
    () => () => {
      if (phaseRef.current !== "ending") void stopCodexVoice(threadId);
      releaseMedia();
    },
    [releaseMedia, threadId],
  );

  const startLevelMeter = React.useCallback((stream: MediaStream) => {
    const context = new AudioContext();
    const source = context.createMediaStreamSource(stream);
    const analyser = context.createAnalyser();
    analyser.fftSize = 64;
    source.connect(analyser);
    const samples = new Uint8Array(analyser.frequencyBinCount);
    analyserContextRef.current = context;
    const draw = () => {
      analyser.getByteFrequencyData(samples);
      const average =
        samples.reduce((total, sample) => total + sample, 0) /
        Math.max(1, samples.length) /
        255;
      const weights = [0.45, 0.72, 1, 0.66, 0.4];
      setLevels((current) =>
        current.map((bar, index) => ({
          ...bar,
          value: 0.14 + average * (weights[index] ?? 0.4),
        })),
      );
      analyserFrameRef.current = requestAnimationFrame(draw);
    };
    draw();
  }, []);

  const beginVoice = React.useCallback(async () => {
    setError(null);
    setTranscript(null);
    setPhase("starting");
    phaseRef.current = "starting";
    try {
      const microphoneAllowed = await requestMicrophoneAccess();
      if (!microphoneAllowed) {
        throw new Error(
          "Enable Buzz in System Settings → Privacy & Security → Microphone.",
        );
      }
      const stream = await voiceRoomAudio.join(threadId);
      streamRef.current = stream;
      voiceRoomAudio.setMuted(threadId, !capturesRoomInput);
      startLevelMeter(stream);

      const peer = new RTCPeerConnection();
      peerRef.current = peer;
      for (const track of stream.getAudioTracks()) peer.addTrack(track, stream);
      peer.createDataChannel("oai-events");
      peer.ontrack = ({ streams }) => {
        const [remoteStream] = streams;
        if (!remoteStream || !remoteAudioRef.current) return;
        voiceRoomAudio.setRemoteStream(threadId, remoteStream);
        remoteAudioRef.current.srcObject = remoteStream;
        void remoteAudioRef.current.play();
      };
      peer.onconnectionstatechange = () => {
        if (peer.connectionState === "failed") {
          setPhase("error");
          setError("The live voice connection failed.");
          releaseMedia();
          void stopCodexVoice(threadId);
        }
      };
      const offer = await peer.createOffer();
      await peer.setLocalDescription(offer);
      await waitForIceGathering(peer);
      const sdp = peer.localDescription?.sdp;
      if (!sdp) throw new Error("Buzz could not create the voice connection.");

      const response = await startCodexVoice({
        threadId,
        pubkey: agentPubkey,
        agentName,
        relayUrl,
        voice,
        sdp,
      });
      setMuted(response.muted);
      setModel(response.model);
    } catch (startError) {
      recoveringRef.current = false;
      releaseMedia();
      void stopCodexVoice(threadId);
      setPhase("error");
      setError(formatError(startError, "Codex Voice could not start."));
    }
  }, [
    agentName,
    agentPubkey,
    capturesRoomInput,
    relayUrl,
    releaseMedia,
    startLevelMeter,
    threadId,
    voice,
  ]);
  restartVoiceRef.current = () => {
    releaseMedia();
    void beginVoice();
  };

  React.useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    void beginVoice();
  }, [beginVoice]);

  React.useEffect(() => {
    if (phase !== "listening") return;
    const userMuted = target.muted ?? false;
    const transportMuted =
      userMuted ||
      !capturesRoomInput ||
      Boolean(speakerLease && speakerLease.threadId !== threadId);
    if (transportMutedRef.current === transportMuted) return;
    transportMutedRef.current = transportMuted;
    voiceRoomAudio.setMuted(threadId, transportMuted);
    setMuted(userMuted);
    void setCodexVoiceMuted(threadId, transportMuted).catch((muteError) => {
      transportMutedRef.current = null;
      voiceRoomAudio.setMuted(threadId, false);
      setMuted(false);
      setVoiceTargetMuted(threadId, false);
      setError(formatError(muteError, "Could not control the microphone."));
    });
  }, [capturesRoomInput, phase, speakerLease, target.muted, threadId]);

  function toggleMute() {
    setVoiceTargetMuted(threadId, !muted);
  }

  async function retryVoice() {
    await stopCodexVoice(threadId);
    releaseMedia();
    await beginVoice();
  }

  return (
    <div className="pointer-events-auto overflow-hidden rounded-xl border border-border/80 bg-card shadow-sm">
      <audio
        autoPlay
        className="hidden"
        muted={
          roomOutputMuted ||
          Boolean(speakerLease && speakerLease.threadId !== threadId)
        }
        ref={remoteAudioRef}
      />
      <div className="flex items-center gap-3 px-3 py-2.5">
        <VoiceOrb levels={levels} muted={muted} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium">{agentName}</span>
            <span className="text-xs text-muted-foreground">
              {phase === "starting"
                ? "Connecting…"
                : phase === "ending"
                  ? "Ending…"
                  : phase === "error"
                    ? "Connection issue"
                    : muted
                      ? "Muted"
                      : "Listening"}
            </span>
          </div>
          <button
            aria-expanded={transcriptExpanded}
            className="flex w-full items-center gap-1 text-left text-xs text-muted-foreground disabled:cursor-default"
            disabled={transcriptHistory.length === 0}
            onClick={() => setTranscriptExpanded((current) => !current)}
            type="button"
          >
            <span className="min-w-0 flex-1 truncate">
              {error ||
                transcript ||
                `${model} · ${voice} · ${mode === "proxy" ? "proxy" : "live room"}`}
            </span>
            {transcriptHistory.length > 0 ? (
              <Captions aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
            ) : null}
          </button>
        </div>
        {phase === "error" ? (
          <>
            <Button
              aria-label={`Retry voice with ${agentName}`}
              onClick={() => void retryVoice()}
              size="icon"
              title="Retry"
              type="button"
              variant="ghost"
            >
              <RotateCcw />
            </Button>
            <Button
              aria-label={`Dismiss voice with ${agentName}`}
              onClick={() => void finishVoice()}
              size="icon"
              title="Dismiss"
              type="button"
              variant="ghost"
            >
              <X />
            </Button>
          </>
        ) : (
          <>
            <Button
              aria-label={muted ? "Unmute microphone" : "Mute microphone"}
              disabled={phase !== "listening"}
              onClick={toggleMute}
              size="icon"
              type="button"
              variant={muted ? "secondary" : "ghost"}
            >
              {muted ? <MicOff /> : <Mic />}
            </Button>
            <Button
              aria-label={`End voice conversation with ${agentName}`}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={phase === "ending"}
              onClick={() => void finishVoice()}
              size="icon"
              type="button"
            >
              {phase === "ending" ? (
                <LoaderCircle className="animate-spin" />
              ) : (
                <PhoneOff />
              )}
            </Button>
          </>
        )}
      </div>
      {transcriptExpanded && transcriptHistory.length > 0 ? (
        <div className="max-h-48 space-y-2 overflow-y-auto border-t border-border/70 px-3 py-2.5 text-xs">
          {transcriptHistory.map((entry) => (
            <p key={entry.id} className="leading-relaxed">
              <span className="mr-1.5 font-medium text-foreground">
                {entry.role === "assistant" ? agentName : "Room"}
              </span>
              <span className="text-muted-foreground">{entry.text}</span>
            </p>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function VoiceOrb({ levels, muted }: { levels: MeterBar[]; muted: boolean }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "flex h-9 w-9 shrink-0 items-center justify-center gap-0.5 rounded-full bg-primary text-primary-foreground shadow-sm",
        muted && "bg-muted text-muted-foreground",
      )}
    >
      {levels.map((level) => (
        <span
          key={level.id}
          className="w-0.5 rounded-full bg-current transition-transform duration-75 motion-reduce:transition-none"
          style={{
            height: 14,
            transform: `scaleY(${muted ? 0.18 : level.value})`,
          }}
        />
      ))}
    </div>
  );
}

function waitForIceGathering(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((resolve) => {
    const onStateChange = () => {
      if (peer.iceGatheringState !== "complete") return;
      peer.removeEventListener("icegatheringstatechange", onStateChange);
      resolve();
    };
    peer.addEventListener("icegatheringstatechange", onStateChange);
  });
}

function formatError(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return fallback;
}
