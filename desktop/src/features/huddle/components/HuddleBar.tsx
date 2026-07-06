import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Bot,
  Captions,
  MessageSquareText,
  PhoneOff,
  SmilePlus,
} from "lucide-react";
import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { useProfileQuery } from "@/features/profile/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_REACTION,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddle } from "../HuddleContext";
import { AddAgentDialog, type AgentAddResult } from "./AddAgentDialog";
import { MicControls, SpeakerControls } from "./MicControls";
import { HuddleParticipantsControl } from "./ParticipantList";
import { truncatePubkey } from "@/shared/lib/pubkey";

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
  transcription_enabled: boolean;
  is_creator: boolean;
  voice_input_mode: "push_to_talk" | "voice_activity";
};

type HuddleBarProps = {
  className?: string;
  onOpenThread?: (channelId: string, messageId: string) => void;
  onVisibilityChange?: (visible: boolean) => void;
};

const HUDDLE_DRAWER_EXIT_MS = 260;
const HUDDLE_REACTION_NAME_MAX = 48;

function isVisibleHuddleState(state: HuddleState | null) {
  return state?.phase === "active" || state?.phase === "connected";
}

function firstTagValue(event: RelayEvent, name: string): string | null {
  return event.tags.find((tag) => tag[0] === name)?.[1] ?? null;
}

function parseEphemeralChannelId(content: string): string | null {
  try {
    const parsed = JSON.parse(content) as { ephemeral_channel_id?: unknown };
    return typeof parsed.ephemeral_channel_id === "string"
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

function customEmojiShortcode(emoji: string): string | null {
  const trimmed = emoji.trim();
  if (!trimmed.startsWith(":") || !trimmed.endsWith(":")) return null;
  const shortcode = trimmed.slice(1, -1).trim().toLowerCase();
  return shortcode.length > 0 ? shortcode : null;
}

function clampReactionName(name: string): string {
  const trimmed = name.trim();
  if (trimmed.length <= HUDDLE_REACTION_NAME_MAX) return trimmed;
  return `${trimmed.slice(0, HUDDLE_REACTION_NAME_MAX - 1).trimEnd()}…`;
}

function fallbackNameForPubkey(pubkey?: string | null): string {
  return pubkey ? `Participant ${truncatePubkey(pubkey)}` : "Someone";
}

function parseHuddleReactionEvent(event: RelayEvent) {
  if (event.kind !== KIND_HUDDLE_REACTION) return null;

  const emoji = (firstTagValue(event, "reaction") ?? event.content).trim();
  if (!emoji) return null;

  const shortcode = customEmojiShortcode(emoji);
  const emojiUrl =
    shortcode !== null
      ? event.tags.find(
          (tag) =>
            tag[0] === "emoji" &&
            tag[1]?.toLowerCase() === shortcode &&
            typeof tag[2] === "string",
        )?.[2]
      : null;
  const senderName = clampReactionName(
    firstTagValue(event, "sender_name") ?? fallbackNameForPubkey(event.pubkey),
  );

  return {
    emoji,
    emojiUrl: emojiUrl ? rewriteRelayUrl(emojiUrl) : null,
    senderName,
  };
}

function huddleReactionTags(
  channelId: string,
  emoji: string,
  senderName: string,
  emojiUrl?: string,
): string[][] {
  const tags: string[][] = [
    ["h", channelId],
    ["reaction", emoji],
    ["sender_name", clampReactionName(senderName)],
  ];
  const shortcode = customEmojiShortcode(emoji);
  if (shortcode && emojiUrl) {
    tags.push(["emoji", shortcode, emojiUrl]);
  }
  return tags;
}

export function HuddleBar({
  className,
  onOpenThread,
  onVisibilityChange,
}: HuddleBarProps) {
  const {
    localAudioTrack,
    leaveHuddle,
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
  const customEmoji = useCustomEmoji();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const { burstHuddleReaction } = useEmojiBurst();

  const isPttMode = voiceInputMode === "push_to_talk";
  const [state, setState] = React.useState<HuddleState | null>(null);
  const [renderedState, setRenderedState] = React.useState<HuddleState | null>(
    null,
  );
  const stateGenerationRef = React.useRef(0);
  const locallyLeavingChannelRef = React.useRef<string | null>(null);
  const [isMuted, setIsMuted] = React.useState(false);
  const [headphonesHintDismissed, setHeadphonesHintDismissed] =
    React.useState(false);
  const [isLeaving, setIsLeaving] = React.useState(false);
  const [showAddAgent, setShowAddAgent] = React.useState(false);
  const [agentAddError, setAgentAddError] = React.useState<string | null>(null);
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const [reactionError, setReactionError] = React.useState<string | null>(null);
  const [transcriptError, setTranscriptError] = React.useState<string | null>(
    null,
  );
  const [huddleThreadEventId, setHuddleThreadEventId] = React.useState<
    string | null
  >(null);
  const [modelStatus, setModelStatus] = React.useState<{
    stt: string;
    tts: string;
  } | null>(null);
  const applyIncomingState = React.useCallback((nextState: HuddleState) => {
    const leavingChannelId = locallyLeavingChannelRef.current;
    if (
      leavingChannelId &&
      nextState.ephemeral_channel_id === leavingChannelId &&
      isVisibleHuddleState(nextState)
    ) {
      return;
    }

    if (!isVisibleHuddleState(nextState)) {
      locallyLeavingChannelRef.current = null;
    }
    setState(nextState);
  }, []);
  // Huddle state: event-driven + 10s fallback poll.
  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    async function fetchState() {
      const generation = stateGenerationRef.current;
      try {
        const s = await invoke<HuddleState>("get_huddle_state");
        if (!cancelled && generation === stateGenerationRef.current) {
          applyIncomingState(s);
        }
      } catch {
        // Only clear state if we never had an active huddle.
        if (!cancelled && generation === stateGenerationRef.current) {
          setState((prev) =>
            prev?.phase === "active" || prev?.phase === "connected"
              ? prev
              : null,
          );
        }
      }
    }

    void fetchState();

    // Primary: listen for Rust-emitted state change events
    listen<HuddleState>("huddle-state-changed", (event) => {
      if (!cancelled) applyIncomingState(event.payload);
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
  }, [applyIncomingState]);

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

  const isHuddleVisible = isVisibleHuddleState(state);

  React.useEffect(() => {
    onVisibilityChange?.(isHuddleVisible);
  }, [isHuddleVisible, onVisibilityChange]);

  React.useEffect(() => {
    if (isHuddleVisible && state) {
      setRenderedState(state);
      return;
    }

    const id = window.setTimeout(
      () => setRenderedState(null),
      HUDDLE_DRAWER_EXIT_MS,
    );
    return () => window.clearTimeout(id);
  }, [isHuddleVisible, state]);

  const barState = isHuddleVisible && state ? state : renderedState;
  const reactionChannelId = barState?.ephemeral_channel_id ?? null;
  const parentChannelId = barState?.parent_channel_id ?? null;
  const currentPubkey = identityQuery.data?.pubkey ?? null;
  const reactionSenderName = React.useMemo(
    () =>
      clampReactionName(
        profileQuery.data?.displayName?.trim() ||
          identityQuery.data?.displayName?.trim() ||
          fallbackNameForPubkey(currentPubkey),
      ),
    [
      currentPubkey,
      identityQuery.data?.displayName,
      profileQuery.data?.displayName,
    ],
  );

  React.useEffect(() => {
    if (!reactionChannelId) {
      setIsReactionPickerOpen(false);
      setReactionError(null);
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;
    const seenEventIds = new Set<string>();

    void relayClient
      .subscribeLive(
        {
          kinds: [KIND_HUDDLE_REACTION],
          "#h": [reactionChannelId],
          limit: 1000,
          since: Math.floor(Date.now() / 1_000),
        },
        (event) => {
          if (disposed) return;
          if (seenEventIds.has(event.id)) return;
          seenEventIds.add(event.id);
          if (event.pubkey === currentPubkey) return;

          const reaction = parseHuddleReactionEvent(event);
          if (!reaction) return;
          burstHuddleReaction(reaction);
        },
      )
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error("[huddle] Failed to subscribe to reactions:", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [burstHuddleReaction, currentPubkey, reactionChannelId]);

  React.useEffect(() => {
    if (!parentChannelId || !reactionChannelId) {
      setHuddleThreadEventId(null);
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;
    setHuddleThreadEventId(null);

    void relayClient
      .subscribeToHuddleEvents(parentChannelId, (event) => {
        if (disposed || event.kind !== KIND_HUDDLE_STARTED) return;
        if (parseEphemeralChannelId(event.content) !== reactionChannelId)
          return;
        setHuddleThreadEventId(event.id);
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error("[huddle] Failed to find huddle thread:", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [parentChannelId, reactionChannelId]);

  const handleHuddleReactionSelect = React.useCallback(
    (emoji: string) => {
      const trimmedEmoji = emoji.trim();
      if (!reactionChannelId || !trimmedEmoji) return;

      const emojiUrl = reactionEmojiUrl(trimmedEmoji, customEmoji);
      const displayEmojiUrl = emojiUrl ? rewriteRelayUrl(emojiUrl) : null;

      setIsReactionPickerOpen(false);
      setReactionError(null);
      burstHuddleReaction({
        emoji: trimmedEmoji,
        emojiUrl: displayEmojiUrl,
        senderName: reactionSenderName,
      });

      void (async () => {
        await relayClient.preconnect();
        const event = await signRelayEvent({
          kind: KIND_HUDDLE_REACTION,
          content: trimmedEmoji,
          tags: huddleReactionTags(
            reactionChannelId,
            trimmedEmoji,
            reactionSenderName,
            emojiUrl,
          ),
        });
        await relayClient.publishEvent(
          event,
          "Timed out while sending huddle reaction.",
          "Failed to send huddle reaction.",
        );
      })().catch((error) => {
        setReactionError("Reaction failed");
        console.error("[huddle] Failed to send reaction:", error);
      });
    },
    [burstHuddleReaction, customEmoji, reactionChannelId, reactionSenderName],
  );

  if (!barState) {
    return null;
  }

  const isDrawerClosing = !isHuddleVisible;
  const ttsEnabled = barState.tts_enabled;
  const transcriptionEnabled = barState.transcription_enabled;

  // Self-removing detection: remote-peer audio plays through native rodio
  // today (outside the WebView render graph), so the browser's AEC has no
  // far-end reference. The AEC follow-up PR flips this constant in the
  // same diff that moves playout into WebAudio.
  const aecMissing = true;

  async function handleLeave() {
    if (isLeaving) return;
    const leavingChannelId = barState?.ephemeral_channel_id ?? null;
    stateGenerationRef.current += 1;
    locallyLeavingChannelRef.current = leavingChannelId;
    setIsLeaving(true);
    try {
      const backendClean = await leaveHuddle();
      if (backendClean) {
        setState(null);
      } else {
        locallyLeavingChannelRef.current = null;
        stateGenerationRef.current += 1;
      }
      // If cleanup failed, keep the bar visible so the user can retry.
    } catch (e) {
      locallyLeavingChannelRef.current = null;
      stateGenerationRef.current += 1;
      console.error("Failed to leave huddle:", e);
    } finally {
      setIsLeaving(false);
    }
  }

  async function handleToggleTranscript() {
    setTranscriptError(null);
    try {
      await invoke("set_huddle_transcription_enabled", {
        enabled: !transcriptionEnabled,
      });
      const s = await invoke<HuddleState>("get_huddle_state");
      setState(s);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setTranscriptError(`Transcript failed: ${message}`);
      console.error("Failed to toggle huddle transcript:", e);
    }
  }

  function handleOpenThread() {
    if (!parentChannelId || !huddleThreadEventId) return;
    onOpenThread?.(parentChannelId, huddleThreadEventId);
  }

  return (
    <div
      aria-hidden={isDrawerClosing}
      data-state={isDrawerClosing ? "closing" : "open"}
      className={cn(
        "buzz-huddle-drawer grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3 px-5 py-3 text-foreground",
        isDrawerClosing && "pointer-events-none",
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-3 overflow-hidden">
        {/* Error banner */}
        {huddleError && (
          <div
            role="alert"
            className="flex min-w-0 items-center gap-1.5 rounded bg-destructive/10 px-2 py-1 text-xs text-destructive"
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

        {/* Model download progress */}
        {modelStatus &&
          ((transcriptionEnabled && modelStatus.stt !== "ready") ||
            modelStatus.tts !== "ready") && (
            <output className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
              <span className="truncate animate-pulse">
                {transcriptionEnabled &&
                modelStatus.stt !== "ready" &&
                modelStatus.tts !== "ready"
                  ? `Voice models: STT ${modelStatus.stt}, TTS ${modelStatus.tts}`
                  : transcriptionEnabled && modelStatus.stt !== "ready"
                    ? `STT model: ${modelStatus.stt}`
                    : `TTS model: ${modelStatus.tts}`}
              </span>
            </output>
          )}

        {agentAddError && (
          <span className="max-w-[180px] truncate rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
            {agentAddError}
          </span>
        )}

        {reactionError && (
          <span className="max-w-[160px] truncate rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
            {reactionError}
          </span>
        )}

        {transcriptError && (
          <span className="max-w-[180px] truncate rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
            {transcriptError}
          </span>
        )}

        {showAddAgent && (
          <AddAgentDialog
            currentAgentPubkeys={barState.agent_pubkeys}
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
      </div>

      <div className="flex items-center gap-5 justify-self-center">
        <div className="flex items-center gap-2">
          <MicControls
            isMuted={isMuted}
            onToggleMute={() => setIsMuted((m) => !m)}
            isPttMode={isPttMode}
            pttActive={pttActive}
            micConnected={micConnected}
            micLevel={micLevel}
            onSelectVoiceInputMode={setVoiceInputMode}
            audioDevices={audioDevices}
            selectedDeviceId={selectedDeviceId}
            onSelectDevice={setSelectedDeviceId}
            micGain={micGain}
            onGainChange={setMicGain}
          />

          <SpeakerControls
            ttsEnabled={ttsEnabled}
            showHeadphonesHint={
              aecMissing && !headphonesHintDismissed && !isDrawerClosing
            }
            onHeadphonesHintDismiss={() => setHeadphonesHintDismissed(true)}
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

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="View huddle thread"
                className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
                disabled={!parentChannelId || !huddleThreadEventId}
                onClick={handleOpenThread}
                size="icon"
                type="button"
                variant="secondary"
              >
                <MessageSquareText className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              View thread
            </TooltipContent>
          </Tooltip>
        </div>

        <div className="flex items-center gap-2">
          <HuddleParticipantsControl
            participants={barState.participants}
            activeSpeakers={activeSpeakers}
            agentPubkeys={barState.agent_pubkeys}
            onRemoveAgent={async (pubkey) => {
              if (!barState.ephemeral_channel_id) return;
              const confirmed = window.confirm(
                "Remove this agent from the huddle?",
              );
              if (!confirmed) return;
              try {
                await invoke("remove_channel_member", {
                  channelId: barState.ephemeral_channel_id,
                  pubkey,
                });
                // Optimistically remove from local state — the backend's
                // 15s membership poll will eventually converge.
                setState((prev) => {
                  if (!prev) return prev;
                  return {
                    ...prev,
                    participants: prev.participants.filter((p) => p !== pubkey),
                    agent_pubkeys: prev.agent_pubkeys.filter(
                      (p) => p !== pubkey,
                    ),
                  };
                });
              } catch (e) {
                console.error("Failed to remove agent from huddle:", e);
              }
            }}
          />

          <Popover
            onOpenChange={setIsReactionPickerOpen}
            open={isReactionPickerOpen}
          >
            <Tooltip>
              <TooltipTrigger asChild>
                <PopoverTrigger asChild>
                  <Button
                    aria-label="Emoji reactions"
                    aria-pressed={isReactionPickerOpen}
                    className={cn(
                      "buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md",
                      isReactionPickerOpen && "text-foreground",
                    )}
                    size="icon"
                    type="button"
                    variant="secondary"
                  >
                    <SmilePlus className="h-4 w-4" />
                  </Button>
                </PopoverTrigger>
              </TooltipTrigger>
              <TooltipContent className="buzz-huddle-tooltip" side="top">
                Emoji reactions
              </TooltipContent>
            </Tooltip>
            <PopoverContent
              align="center"
              className="w-auto overflow-hidden rounded-2xl border-0 bg-transparent p-0 shadow-none"
              side="top"
              sideOffset={10}
            >
              <EmojiPicker autoFocus onSelect={handleHuddleReactionSelect} />
            </PopoverContent>
          </Popover>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label={
                  transcriptionEnabled ? "Stop transcript" : "Start transcript"
                }
                aria-pressed={transcriptionEnabled}
                className={cn(
                  "buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md",
                  transcriptionEnabled && "text-foreground",
                )}
                onClick={() => void handleToggleTranscript()}
                size="icon"
                type="button"
                variant={transcriptionEnabled ? "secondary" : "ghost"}
              >
                <Captions className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              {transcriptionEnabled ? "Stop transcript" : "Start transcript"}
            </TooltipContent>
          </Tooltip>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Add agent to huddle"
                className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
                onClick={() => setShowAddAgent(true)}
                size="icon"
                type="button"
                variant="secondary"
              >
                <Bot className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              Add agent
            </TooltipContent>
          </Tooltip>
        </div>

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Leave huddle"
              className="h-12 gap-2 px-4"
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
          <TooltipContent className="buzz-huddle-tooltip">
            Leave huddle
          </TooltipContent>
        </Tooltip>
      </div>

      <div aria-hidden="true" className="min-w-0" />

      {/* Screen reader announcements for huddle state changes */}
      <output aria-live="polite" className="sr-only">
        {micConnected
          ? "In huddle, microphone connected"
          : "In huddle, no microphone"}
        {`, voice input: ${isPttMode ? "push to talk, press Ctrl+Space to transmit" : "voice activity detection"}`}
        {modelStatus &&
          transcriptionEnabled &&
          modelStatus.stt !== "ready" &&
          `, STT model ${modelStatus.stt}`}
        {modelStatus &&
          modelStatus.tts !== "ready" &&
          `, TTS model ${modelStatus.tts}`}
      </output>
    </div>
  );
}
