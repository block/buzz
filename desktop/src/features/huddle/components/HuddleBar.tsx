import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Bot,
  Captions,
  PhoneOff,
  PictureInPicture,
  PictureInPicture2,
  SmilePlus,
} from "lucide-react";
import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { useProfileQuery, useSelfProfileCache } from "@/features/profile/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_HUDDLE_REACTION } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useDocumentVisible } from "@/shared/lib/useDocumentVisible";
import { Button } from "@/shared/ui/button";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddle } from "../HuddleContext";
import { AddAgentDialog, type AgentAddResult } from "./AddAgentDialog";
import type { HuddleAgentVoiceSettings } from "./AgentVoiceMenu";
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
  huddle_thread_event_id: string | null;
  participants: string[]; // pubkey hex strings
  agent_pubkeys: string[];
  agent_voice_settings: Record<string, HuddleAgentVoiceSettings>;
  tts_enabled: boolean;
  transcription_enabled: boolean;
  is_creator: boolean;
  voice_input_mode: "push_to_talk" | "voice_activity";
};

type HuddleBarProps = {
  className?: string;
  /** The companion window owns the full controls; the main app only opens it. */
  mode?: "main" | "room";
  /** Called after the companion window has been opened successfully. */
  onOpenHuddleWindow?: () => void;
  onOpenThread?: (channelId: string, messageId: string) => void;
  onVisibilityChange?: (visible: boolean) => void;
};

const HUDDLE_DRAWER_EXIT_MS = 260;
const HUDDLE_STATE_FALLBACK_INTERVAL_MS = 30_000;
const HUDDLE_MODEL_STATUS_INTERVAL_MS = 10_000;
const HUDDLE_REACTION_NAME_MAX = 48;
const HEADPHONES_HINT_SEEN_STORAGE_KEY = "buzz.huddle.headphones-hint-seen";

function hasSeenHeadphonesHint() {
  return window.localStorage.getItem(HEADPHONES_HINT_SEEN_STORAGE_KEY) === "1";
}

function isVisibleHuddleState(state: HuddleState | null) {
  return state?.phase === "active" || state?.phase === "connected";
}

function firstTagValue(event: RelayEvent, name: string): string | null {
  return event.tags.find((tag) => tag[0] === name)?.[1] ?? null;
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
  mode = "main",
  onOpenHuddleWindow,
  onVisibilityChange,
}: HuddleBarProps) {
  const documentVisible = useDocumentVisible();
  const {
    leaveHuddle,
    micConnected,
    isMuted,
    toggleMute,
    micLevel,
    voiceInputMode,
    setVoiceInputMode,
    activeSpeakers,
    speakerLevels,
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
  const selfProfileCache = useSelfProfileCache();
  const { burstHuddleReaction } = useEmojiBurst();

  const isPttMode = voiceInputMode === "push_to_talk";
  const [state, setState] = React.useState<HuddleState | null>(null);
  const [renderedState, setRenderedState] = React.useState<HuddleState | null>(
    null,
  );
  const stateGenerationRef = React.useRef(0);
  const locallyLeavingChannelRef = React.useRef<string | null>(null);
  const [headphonesHintDismissed, setHeadphonesHintDismissed] = React.useState(
    hasSeenHeadphonesHint,
  );
  const [isLeaving, setIsLeaving] = React.useState(false);
  const [showAddAgent, setShowAddAgent] = React.useState(false);
  const [agentAddError, setAgentAddError] = React.useState<string | null>(null);
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const [reactionError, setReactionError] = React.useState<string | null>(null);
  const [transcriptError, setTranscriptError] = React.useState<string | null>(
    null,
  );
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
  // Huddle state: event-driven + slow fallback poll.
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

    if (documentVisible) void fetchState();

    // Primary: listen for Rust-emitted state change events
    listen<HuddleState>("huddle-state-changed", (event) => {
      if (!cancelled) {
        stateGenerationRef.current += 1;
        applyIncomingState(event.payload);
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    // Fallback in case events are missed; keep it slow so normal huddle use is
    // event-driven and does not keep a sync IPC command warm on the main thread.
    const id = documentVisible
      ? window.setInterval(
          () => void fetchState(),
          HUDDLE_STATE_FALLBACK_INTERVAL_MS,
        )
      : null;

    return () => {
      cancelled = true;
      unlisten?.();
      if (id !== null) window.clearInterval(id);
    };
  }, [applyIncomingState, documentVisible]);

  const huddlePhase = state?.phase;
  React.useEffect(() => {
    if (
      !documentVisible ||
      (huddlePhase !== "active" && huddlePhase !== "connected")
    )
      return;

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
    const id = window.setInterval(
      () => void pollModels(),
      HUDDLE_MODEL_STATUS_INTERVAL_MS,
    );

    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [documentVisible, huddlePhase]);

  React.useEffect(() => {
    if (huddlePhase === "active" || huddlePhase === "connected") return;
    setModelStatus(null);
  }, [huddlePhase]);

  const isHuddleVisible = isVisibleHuddleState(state);
  const companionHadActiveHuddleRef = React.useRef(false);
  const mainHadActiveHuddleRef = React.useRef(false);

  const dismissHeadphonesHint = React.useCallback(() => {
    window.localStorage.setItem(HEADPHONES_HINT_SEEN_STORAGE_KEY, "1");
    setHeadphonesHintDismissed(true);
  }, []);

  React.useEffect(() => {
    onVisibilityChange?.(isHuddleVisible);
  }, [isHuddleVisible, onVisibilityChange]);

  React.useEffect(() => {
    if (mode !== "room") return;
    if (isHuddleVisible) {
      companionHadActiveHuddleRef.current = true;
      return;
    }
    if (companionHadActiveHuddleRef.current) {
      void getCurrentWindow().close();
    }
  }, [isHuddleVisible, mode]);

  React.useEffect(() => {
    if (mode !== "main") return;
    if (isHuddleVisible) {
      mainHadActiveHuddleRef.current = true;
      return;
    }
    if (mainHadActiveHuddleRef.current) {
      mainHadActiveHuddleRef.current = false;
      dismissHeadphonesHint();
    }
  }, [dismissHeadphonesHint, isHuddleVisible, mode]);

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
  const currentPubkey = identityQuery.data?.pubkey ?? null;
  const participantSpeakerLevels = React.useMemo(() => {
    const levels = { ...speakerLevels };
    if (currentPubkey) {
      levels[currentPubkey.toLowerCase()] =
        micConnected && !isMuted ? micLevel : 0;
    }
    return levels;
  }, [currentPubkey, isMuted, micConnected, micLevel, speakerLevels]);
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
  const hasAvailableMic = micConnected;
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
        // A companion is the active huddle room, not a persistent transcript
        // viewer. Close it immediately after it ends instead of depending on
        // an asynchronous state event to arrive first.
        if (mode === "room") {
          void getCurrentWindow().close();
        }
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

  async function handleOpenHuddleWindow() {
    try {
      await invoke("open_huddle_window");
      dismissHeadphonesHint();
      onOpenHuddleWindow?.();
    } catch (error) {
      console.error("Failed to open huddle window:", error);
    }
  }

  async function handleReturnToDrawer() {
    try {
      await invoke("close_huddle_companion");
    } catch (error) {
      console.error("Failed to return huddle to drawer:", error);
    }
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
          open={showAddAgent}
        />

        <div className="flex shrink-0 items-center gap-2">
          <MicControls
            isMuted={isMuted}
            onToggleMute={toggleMute}
            isPttMode={isPttMode}
            micConnected={hasAvailableMic}
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
              mode === "main" &&
              aecMissing &&
              !headphonesHintDismissed &&
              !isDrawerClosing
            }
            onHeadphonesHintDismiss={dismissHeadphonesHint}
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

          {mode === "main" ? (
            <HuddleParticipantsControl
              participants={barState.participants}
              activeSpeakers={activeSpeakers}
              speakerLevels={participantSpeakerLevels}
              agentPubkeys={barState.agent_pubkeys}
              agentVoiceSettings={barState.agent_voice_settings}
              selfProfile={{
                avatarUrl:
                  profileQuery.data?.avatarUrl ??
                  selfProfileCache?.avatarDataUrl ??
                  null,
                displayName:
                  profileQuery.data?.displayName ??
                  identityQuery.data?.displayName ??
                  null,
                pubkey: currentPubkey,
              }}
              onRemoveAgent={async (pubkey) => {
                const confirmed = window.confirm(
                  "Remove this agent from the huddle?",
                );
                if (!confirmed) return;
                try {
                  await invoke("remove_agent_from_huddle", {
                    agentPubkey: pubkey,
                  });
                  setState((prev) => {
                    if (!prev) return prev;
                    return {
                      ...prev,
                      participants: prev.participants.filter(
                        (p) => p !== pubkey,
                      ),
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
          ) : null}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 justify-self-center">
        <div className="flex items-center gap-2">
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
                className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
                onClick={() => void handleToggleTranscript()}
                size="icon"
                type="button"
                variant="ghost"
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
      </div>

      <div className="flex shrink-0 items-center gap-2 justify-self-end">
        {mode === "main" ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Open huddle in a new window"
                className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
                onClick={() => void handleOpenHuddleWindow()}
                size="icon"
                type="button"
                variant="secondary"
              >
                <PictureInPicture2 className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              Open huddle window
            </TooltipContent>
          </Tooltip>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Return huddle to drawer"
                className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
                onClick={() => void handleReturnToDrawer()}
                size="icon"
                type="button"
                variant="secondary"
              >
                <PictureInPicture className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent className="buzz-huddle-tooltip" side="top">
              Return huddle to drawer
            </TooltipContent>
          </Tooltip>
        )}

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
      </div>

      {/* Screen reader announcements for huddle state changes */}
      <output aria-live="polite" className="sr-only">
        {hasAvailableMic
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
