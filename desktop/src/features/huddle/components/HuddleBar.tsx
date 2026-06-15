import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Headphones, PhoneOff, Plus } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddle } from "../HuddleContext";
import { AddAgentDialog, type AgentAddResult } from "./AddAgentDialog";
import { HeadphonesNotice } from "./HeadphonesNotice";
import { MicControls, SpeakerControls } from "./MicControls";
import { ParticipantList } from "./ParticipantList";

// Mirrors HuddleState in src-tauri/src/huddle/mod.rs.
type HuddleState = {
  phase:
    | "idle"
    | "creating"
    | "connecting"
    | "connected"
    | "active"
    | "leaving";
  parent_channel_id: string | null;
  ephemeral_channel_id: string | null;
  participants: string[]; // pubkey hex strings
  agent_pubkeys: string[];
  tts_enabled: boolean;
  is_creator: boolean;
  voice_input_mode: "push_to_talk" | "voice_activity";
};

type HuddleBarProps = {
  className?: string;
};

export function HuddleBar({ className }: HuddleBarProps) {
  const {
    localAudioTrack,
    leaveHuddle,
    endHuddle,
    isStarting,
    micConnected,
    micLevel,
    pttActive,
    voiceInputMode,
    setVoiceInputMode,
    activeSpeakers,
    huddleError,
    clearHuddleError,
    audioDevices,
    selectedDeviceId,
    setSelectedDeviceId,
    micGain,
    setMicGain,
    outputDevices,
    selectedOutputDevice,
    setSelectedOutputDevice,
  } = useHuddle();

  const isPttMode = voiceInputMode === "push_to_talk";
  const [state, setState] = React.useState<HuddleState | null>(null);
  const [isMuted, setIsMuted] = React.useState(false);
  // Session-dismissable "use headphones" notice. The desktop client plays
  // remote peer audio via native rodio, outside the WebView render graph,
  // so the browser's echo canceller has no far-end reference to cancel
  // against. Until the WebAudio AEC follow-up PR lands, two people on
  // speakers in the same physical room will hear themselves echo. The
  // banner stays visible across huddle start/stop within a single page
  // load; it auto-removes when the AEC plumbing lands and the detection
  // below flips to false.
  const [headphonesNoticeDismissed, setHeadphonesNoticeDismissed] =
    React.useState(false);
  const ttsEnabled = state?.tts_enabled ?? true;
  const [isLeaving, setIsLeaving] = React.useState(false);
  const [showAddAgent, setShowAddAgent] = React.useState(false);
  const [agentAddError, setAgentAddError] = React.useState<string | null>(null);
  const [modelStatus, setModelStatus] = React.useState<{
    stt: string;
    tts: string;
  } | null>(null);
  // Huddle state: event-driven + 10s fallback poll.
  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    async function fetchState() {
      try {
        const s = await invoke<HuddleState>("get_huddle_state");
        if (!cancelled) setState(s);
      } catch {
        // Only clear state if we never had an active huddle.
        if (!cancelled) {
          setState((prev) =>
            prev?.phase === "active" || prev?.phase === "connected"
              ? prev
              : null,
          );
        }
      }
    }

    // Initial fetch
    void fetchState();

    // Primary: listen for Rust-emitted state change events
    listen<HuddleState>("huddle-state-changed", (event) => {
      if (!cancelled) setState(event.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    // Fallback: 10s poll in case events are missed
    const id = window.setInterval(() => void fetchState(), 10_000);

    return () => {
      cancelled = true;
      unlisten?.();
      window.clearInterval(id);
    };
  }, []);

  const huddlePhase = state?.phase;
  React.useEffect(() => {
    if (huddlePhase !== "active" && huddlePhase !== "connected") return;

    let cancelled = false;

    const fmt = (s: unknown): string => {
      if (typeof s === "string") return s === "ready" ? "ready" : "pending";
      if (typeof s === "object" && s !== null) {
        if ("downloading" in s) {
          const d = (s as { downloading: { progress_percent: number } })
            .downloading;
          return `${d.progress_percent}%`;
        }
        if ("error" in s) return "error";
      }
      return "pending";
    };

    async function pollModels() {
      try {
        const status = await invoke<{
          stt: unknown;
          tts: unknown;
        }>("get_model_status");
        if (cancelled) return;

        setModelStatus({
          stt: fmt(status.stt),
          tts: fmt(status.tts),
        });
      } catch {
        // best-effort
      }
    }

    void pollModels();
    const id = window.setInterval(() => void pollModels(), 3_000);

    return () => {
      cancelled = true;
      window.clearInterval(id);
      setModelStatus(null); // Clear stale status on huddle end/phase change.
    };
  }, [huddlePhase]);

  React.useEffect(() => {
    if (localAudioTrack) {
      localAudioTrack.enabled = !isMuted;
    }
  }, [isMuted, localAudioTrack]);

  if (!state || (state.phase !== "active" && state.phase !== "connected"))
    return null;

  // Self-removing detection: remote-peer audio plays through native rodio
  // today (outside the WebView render graph), so the browser's AEC has no
  // far-end reference. The AEC follow-up PR flips this constant in the
  // same diff that moves playout into WebAudio.
  const aecMissing = true;

  async function handleLeave() {
    if (isLeaving) return;
    setIsLeaving(true);
    try {
      const backendClean = await leaveHuddle();
      if (backendClean) {
        setState(null);
      }
      // If cleanup failed, keep the bar visible so the user can retry.
    } catch (e) {
      console.error("Failed to leave huddle:", e);
    } finally {
      setIsLeaving(false);
    }
  }

  async function handleEnd() {
    if (isLeaving) return;
    const confirmed = window.confirm(
      "End the huddle for everyone? This will disconnect all participants.",
    );
    if (!confirmed) return;
    setIsLeaving(true);
    try {
      const backendClean = await endHuddle();
      if (backendClean) {
        setState(null);
      }
      // If cleanup failed, keep the bar visible so the user can retry.
    } catch (e) {
      console.error("Failed to end huddle:", e);
    } finally {
      setIsLeaving(false);
    }
  }

  return (
    <div
      className={cn(
        "flex items-center gap-3 border-t bg-background px-4 py-2",
        className,
      )}
    >
      {/* Error banner */}
      {huddleError && (
        <div
          role="alert"
          className="flex items-center gap-1.5 rounded bg-destructive/10 px-2 py-1 text-xs text-destructive"
        >
          <span className="max-w-[220px] truncate">{huddleError}</span>
          <button
            aria-label="Dismiss error"
            className="ml-1 opacity-60 hover:opacity-100"
            onClick={clearHuddleError}
            type="button"
          >
            ✕
          </button>
        </div>
      )}

      {/* Echo-cancellation pre-PR notice. Removed when AEC plumbing lands. */}
      {aecMissing && !headphonesNoticeDismissed && (
        <HeadphonesNotice
          onDismiss={() => setHeadphonesNoticeDismissed(true)}
        />
      )}

      {/* Model download progress */}
      {modelStatus &&
        (modelStatus.stt !== "ready" || modelStatus.tts !== "ready") && (
          <output className="flex items-center gap-1 text-xs text-muted-foreground">
            <span className="animate-pulse">
              {modelStatus.stt !== "ready" && modelStatus.tts !== "ready"
                ? `Voice models: STT ${modelStatus.stt}, TTS ${modelStatus.tts}`
                : modelStatus.stt !== "ready"
                  ? `STT model: ${modelStatus.stt}`
                  : `TTS model: ${modelStatus.tts}`}
            </span>
          </output>
        )}

      {/* Participants */}
      <div className="flex items-center gap-2">
        <Headphones className="h-4 w-4 text-muted-foreground" />
        <ParticipantList
          participants={state.participants}
          activeSpeakers={activeSpeakers}
          agentPubkeys={state.agent_pubkeys}
          onRemoveAgent={async (pubkey) => {
            if (!state.ephemeral_channel_id) return;
            const confirmed = window.confirm(
              "Remove this agent from the huddle?",
            );
            if (!confirmed) return;
            try {
              await invoke("remove_channel_member", {
                channelId: state.ephemeral_channel_id,
                pubkey,
              });
              // Optimistically remove from local state — the backend's
              // 15s membership poll will eventually converge.
              setState((prev) => {
                if (!prev) return prev;
                return {
                  ...prev,
                  participants: prev.participants.filter((p) => p !== pubkey),
                  agent_pubkeys: prev.agent_pubkeys.filter((p) => p !== pubkey),
                };
              });
            } catch (e) {
              console.error("Failed to remove agent from huddle:", e);
            }
          }}
        />
        <Button
          aria-label="Add agent to huddle"
          className="h-7 w-7"
          onClick={() => setShowAddAgent(true)}
          size="icon"
          variant="ghost"
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>

      {/* Voice input mode — single toggle combining indicator + switch */}
      {micConnected ? (
        <>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label={
                  isPttMode
                    ? "Push to Talk mode — click to switch to Auto"
                    : "Auto mode — click to switch to Push to Talk"
                }
                className="h-7 gap-1.5 px-2 text-xs"
                onClick={() =>
                  void setVoiceInputMode(
                    isPttMode ? "voice_activity" : "push_to_talk",
                  )
                }
                size="sm"
                variant="ghost"
              >
                {isPttMode ? (
                  <>
                    <div
                      className={cn(
                        "h-2 w-2 rounded-full transition-colors",
                        pttActive && !isMuted
                          ? "animate-pulse bg-green-500"
                          : "bg-zinc-500",
                      )}
                    />
                    Push to Talk
                  </>
                ) : (
                  <>
                    <div
                      className="h-2 w-2 rounded-full transition-colors"
                      style={{
                        backgroundColor:
                          micLevel > 0.05
                            ? `rgba(34, 197, 94, ${0.4 + micLevel * 0.6})`
                            : "rgba(100, 116, 139, 0.4)",
                      }}
                    />
                    Auto
                  </>
                )}
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {isPttMode
                ? "Push to Talk — hold Ctrl+Space to transmit. Click to switch to Auto."
                : "Auto — mic is always live. Click to switch to Push to Talk."}
            </TooltipContent>
          </Tooltip>
          {isPttMode && (
            <kbd className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {navigator.platform?.includes("Mac") ? "⌃Space" : "Ctrl+Space"}
            </kbd>
          )}
        </>
      ) : (
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <div
            className={cn(
              "h-2 w-2 rounded-full",
              isStarting ? "animate-pulse bg-green-500" : "bg-destructive/70",
            )}
          />
          {isStarting ? "Connecting…" : "No mic"}
        </span>
      )}

      {agentAddError && (
        <span className="max-w-[180px] truncate rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
          {agentAddError}
        </span>
      )}

      {showAddAgent && (
        <AddAgentDialog
          currentAgentPubkeys={state?.agent_pubkeys ?? []}
          onClose={() => setShowAddAgent(false)}
          onAdd={async (pubkey: string): Promise<AgentAddResult> => {
            setAgentAddError(null);
            try {
              const result = await invoke<AgentAddResult>(
                "add_agent_to_huddle",
                { agentPubkey: pubkey },
              );
              // Refresh huddle state so the participant list updates immediately.
              const s = await invoke<HuddleState>("get_huddle_state");
              setState(s);
              return result;
            } catch (e: unknown) {
              const msg = e instanceof Error ? e.message : String(e);
              setAgentAddError(`Failed to add agent: ${msg}`);
              throw e; // Re-throw so AddAgentDialog shows its inline error.
            }
          }}
        />
      )}

      <MicControls
        isMuted={isMuted}
        onToggleMute={() => setIsMuted((m) => !m)}
        isPttMode={isPttMode}
        pttActive={pttActive}
        micConnected={micConnected}
        audioDevices={audioDevices}
        selectedDeviceId={selectedDeviceId}
        onSelectDevice={setSelectedDeviceId}
        micGain={micGain}
        onGainChange={setMicGain}
      />

      <SpeakerControls
        ttsEnabled={ttsEnabled}
        onToggleTts={async () => {
          try {
            await invoke("set_tts_enabled", { enabled: !ttsEnabled });
            const s = await invoke<HuddleState>("get_huddle_state");
            setState(s);
          } catch (e) {
            console.error("Failed to toggle TTS:", e);
          }
        }}
        outputDevices={outputDevices}
        selectedOutputDevice={selectedOutputDevice}
        onSelectOutputDevice={setSelectedOutputDevice}
      />

      {/* Leave / End buttons — pushed to the right */}
      <div className="ml-auto flex items-center gap-2">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Leave huddle"
              className="h-8 gap-1.5 px-3"
              disabled={isLeaving}
              aria-busy={isLeaving}
              onClick={() => void handleLeave()}
              size="sm"
              variant="destructive"
            >
              <PhoneOff className="h-4 w-4" />
              Leave
            </Button>
          </TooltipTrigger>
          <TooltipContent>Leave huddle</TooltipContent>
        </Tooltip>
        {state?.is_creator && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="End huddle for everyone"
                className="h-8 gap-1.5 px-3"
                disabled={isLeaving}
                onClick={() => void handleEnd()}
                size="sm"
                variant="destructive"
              >
                <PhoneOff className="h-4 w-4" />
                End for all
              </Button>
            </TooltipTrigger>
            <TooltipContent>End huddle for everyone</TooltipContent>
          </Tooltip>
        )}
      </div>

      {/* Screen reader announcements for huddle state changes */}
      <output aria-live="polite" className="sr-only">
        {micConnected
          ? "In huddle, microphone connected"
          : "In huddle, no microphone"}
        {`, voice input: ${isPttMode ? "push to talk, press Ctrl+Space to transmit" : "voice activity detection"}`}
        {modelStatus &&
          modelStatus.stt !== "ready" &&
          `, STT model ${modelStatus.stt}`}
        {modelStatus &&
          modelStatus.tts !== "ready" &&
          `, TTS model ${modelStatus.tts}`}
      </output>
    </div>
  );
}
