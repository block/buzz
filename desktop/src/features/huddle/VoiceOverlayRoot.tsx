import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  Captions,
  Mic,
  MicOff,
  Move,
  PhoneOff,
  Radio,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  type VoiceOverlayAction,
  type VoiceOverlayMediaState,
  type VoiceOverlayPhase,
  VOICE_OVERLAY_ACTION_EVENT,
  VOICE_OVERLAY_READY_EVENT,
  VOICE_OVERLAY_STATE_EVENT,
  voiceOverlayMediaSnapshot,
} from "./lib/voiceOverlayProtocol";

type NativeHuddleState = {
  phase: VoiceOverlayPhase;
  participants: string[];
  agent_pubkeys: string[];
  tts_enabled: boolean;
  transcription_enabled: boolean;
  voice_input_mode: "push_to_talk" | "voice_activity";
};

const defaultMediaState: VoiceOverlayMediaState = {
  version: 1,
  phase: "idle",
  participantCount: 0,
  agentCount: 0,
  ttsEnabled: true,
  transcriptionEnabled: false,
  isLeaving: false,
  error: null,
  isMuted: false,
  micConnected: false,
  micLevel: 0,
  pttActive: false,
  voiceInputMode: "voice_activity",
};

function snapshotFromNativeState(
  state: NativeHuddleState,
  current: VoiceOverlayMediaState,
): VoiceOverlayMediaState {
  return voiceOverlayMediaSnapshot({
    ...current,
    version: 1,
    phase: state.phase,
    participantCount: state.participants.length,
    agentCount: state.agent_pubkeys.length,
    ttsEnabled: state.tts_enabled,
    transcriptionEnabled: state.transcription_enabled,
    voiceInputMode: state.voice_input_mode,
    isLeaving: state.phase === "leaving",
  });
}

function participantSummary(participants: number, agents: number): string {
  const participantLabel = participants === 1 ? "participant" : "participants";
  const agentLabel = agents === 1 ? "agent" : "agents";
  return `${participants} ${participantLabel} · ${agents} ${agentLabel}`;
}

async function sendMainAction(action: VoiceOverlayAction) {
  await emitTo("main", VOICE_OVERLAY_ACTION_EVENT, action);
}

export function VoiceOverlayRoot() {
  const [snapshot, setSnapshot] = React.useState(defaultMediaState);
  const [transportError, setTransportError] = React.useState<string | null>(
    null,
  );

  const refreshNativeState = React.useCallback(async () => {
    const state = await invoke<NativeHuddleState>("get_huddle_state");
    setSnapshot((current) => snapshotFromNativeState(state, current));
    setTransportError(null);
  }, []);

  const dispatchAction = React.useCallback(
    async (action: VoiceOverlayAction) => {
      try {
        await sendMainAction(action);
        setTransportError(null);
      } catch {
        setTransportError("Voice controls could not reach the main window.");
      }
    },
    [],
  );

  React.useLayoutEffect(() => {
    void invoke("voice_overlay_window", { action: "ready" }).catch(() => {
      // Browser E2E and unsupported hosts render without native reveal.
    });
  }, []);

  React.useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];

    void refreshNativeState().catch(() => {
      if (!disposed) {
        setTransportError("Voice state is unavailable.");
      }
    });

    void (async () => {
      const unlistenNative = await listen<NativeHuddleState>(
        "huddle-state-changed",
        (event) => {
          if (!disposed) {
            setSnapshot((current) =>
              snapshotFromNativeState(event.payload, current),
            );
            setTransportError(null);
          }
        },
      );
      if (disposed) {
        unlistenNative();
        return;
      }
      cleanups.push(() => void unlistenNative());

      const unlistenOwner = await listen<VoiceOverlayMediaState>(
        VOICE_OVERLAY_STATE_EVENT,
        (event) => {
          if (!disposed && event.payload.version === 1) {
            setSnapshot(voiceOverlayMediaSnapshot(event.payload));
            setTransportError(null);
          }
        },
      );
      if (disposed) {
        unlistenOwner();
        return;
      }
      cleanups.push(() => void unlistenOwner());

      await emitTo("main", VOICE_OVERLAY_READY_EVENT);
    })().catch(() => {
      if (!disposed) {
        setTransportError("Voice controls could not reach the main window.");
      }
    });

    return () => {
      disposed = true;
      for (const cleanup of cleanups) cleanup();
    };
  }, [refreshNativeState]);

  const displayedError = transportError ?? snapshot.error;
  const isActive =
    snapshot.phase === "active" || snapshot.phase === "connected";
  const isPtt = snapshot.voiceInputMode === "push_to_talk";
  const micLabel = !snapshot.micConnected
    ? "Microphone syncing"
    : snapshot.isMuted
      ? "Unmute microphone"
      : "Mute microphone";

  return (
    <main
      className="min-h-dvh bg-transparent p-2 text-foreground"
      data-testid="voice-overlay"
    >
      <section className="overflow-hidden rounded-2xl border bg-background/95 shadow-2xl backdrop-blur-xl">
        <header
          className="flex h-10 items-center gap-2 border-b px-3"
          data-tauri-drag-region
        >
          <Move className="h-3.5 w-3.5 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-semibold">Buzz Voice</p>
            <p className="truncate text-xs text-muted-foreground">
              {isActive
                ? participantSummary(
                    snapshot.participantCount,
                    snapshot.agentCount,
                  )
                : "No active huddle"}
            </p>
          </div>
          <Button
            aria-label="Close floating voice controls"
            className="h-7 w-7 rounded-full"
            onClick={() => void getCurrentWebviewWindow().close()}
            size="icon"
            variant="ghost"
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </header>

        <div className="flex items-center justify-center gap-2 px-3 py-3">
          <Button
            aria-label={micLabel}
            aria-pressed={snapshot.isMuted}
            className={cn(
              "relative h-11 w-11 rounded-full",
              snapshot.pttActive &&
                !snapshot.isMuted &&
                "ring-2 ring-green-500",
            )}
            disabled={!snapshot.micConnected || !isActive}
            onClick={() =>
              void dispatchAction({ version: 1, type: "toggle_mute" })
            }
            size="icon"
            variant={snapshot.isMuted ? "destructive" : "secondary"}
          >
            {snapshot.isMuted || !snapshot.micConnected ? (
              <MicOff className="h-4 w-4" />
            ) : (
              <Mic className="h-4 w-4" />
            )}
            {!snapshot.isMuted && snapshot.micConnected && (
              <span
                aria-hidden="true"
                className="absolute bottom-1 h-1 rounded-full bg-green-500 transition-[width]"
                style={{ width: `${Math.max(4, snapshot.micLevel * 24)}px` }}
              />
            )}
          </Button>

          <Button
            aria-label={isPtt ? "Use voice activity" : "Use push to talk"}
            aria-pressed={isPtt}
            className="h-11 w-11 rounded-full"
            disabled={!isActive}
            onClick={() =>
              void dispatchAction({
                version: 1,
                type: "set_voice_input_mode",
                mode: isPtt ? "voice_activity" : "push_to_talk",
              })
            }
            size="icon"
            variant={isPtt ? "default" : "secondary"}
          >
            <Radio className="h-4 w-4" />
          </Button>

          <Button
            aria-label={
              snapshot.transcriptionEnabled
                ? "Stop transcript"
                : "Start transcript"
            }
            aria-pressed={snapshot.transcriptionEnabled}
            className="h-11 w-11 rounded-full"
            disabled={!isActive}
            onClick={() =>
              void dispatchAction({
                version: 1,
                type: "toggle_transcription",
              })
            }
            size="icon"
            variant={snapshot.transcriptionEnabled ? "default" : "secondary"}
          >
            <Captions className="h-4 w-4" />
          </Button>

          <Button
            aria-label={
              snapshot.ttsEnabled ? "Mute agent voice" : "Hear agent voice"
            }
            aria-pressed={!snapshot.ttsEnabled}
            className="h-11 w-11 rounded-full"
            disabled={!isActive}
            onClick={() =>
              void dispatchAction({ version: 1, type: "toggle_tts" })
            }
            size="icon"
            variant="secondary"
          >
            {snapshot.ttsEnabled ? (
              <Volume2 className="h-4 w-4" />
            ) : (
              <VolumeX className="h-4 w-4" />
            )}
          </Button>

          <Button
            aria-label="Leave huddle"
            className="h-11 w-11 rounded-full"
            disabled={!isActive || snapshot.isLeaving}
            onClick={() => void dispatchAction({ version: 1, type: "leave" })}
            size="icon"
            variant="destructive"
          >
            <PhoneOff className="h-4 w-4" />
          </Button>
        </div>

        {displayedError && (
          <p
            className="border-t px-3 py-2 text-xs text-destructive"
            role="alert"
          >
            {displayedError}
          </p>
        )}
      </section>
    </main>
  );
}
