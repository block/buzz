import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Headphones } from "lucide-react";
import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { useHuddle } from "../HuddleContext";
import { MicControls } from "./MicControls";

type HuddleProfileState = {
  phase:
    | "idle"
    | "creating"
    | "connecting"
    | "connected"
    | "active"
    | "leaving";
  parent_channel_id: string | null;
  ephemeral_channel_id: string | null;
};

function isVisible(
  state: HuddleProfileState | null,
): state is HuddleProfileState {
  return state?.phase === "connected" || state?.phase === "active";
}

/** A small huddle presence attached to the profile area in the main app. */
export function HuddleProfileControl({
  channels,
  onHuddleEnded,
  visible,
}: {
  channels: Channel[];
  onHuddleEnded?: (ephemeralChannelId: string | null) => void;
  visible: boolean;
}) {
  const {
    audioDevices,
    isMuted,
    leaveHuddle,
    micConnected,
    micGain,
    micLevel,
    selectedDeviceId,
    setMicGain,
    setSelectedDeviceId,
    setVoiceInputMode,
    toggleMute,
    voiceInputMode,
  } = useHuddle();
  const [isLeaving, setIsLeaving] = React.useState(false);
  const [state, setState] = React.useState<HuddleProfileState | null>(null);
  const lastHuddleChannelIdRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void invoke<HuddleProfileState>("get_huddle_state")
      .then((next) => {
        if (!disposed) setState(next);
      })
      .catch(() => {
        if (!disposed) setState(null);
      });

    void listen<HuddleProfileState>("huddle-state-changed", (event) => {
      if (!disposed) setState(event.payload);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    if (state?.ephemeral_channel_id) {
      lastHuddleChannelIdRef.current = state.ephemeral_channel_id;
    }
    if (state?.phase !== "idle" || !lastHuddleChannelIdRef.current) return;
    onHuddleEnded?.(lastHuddleChannelIdRef.current);
    lastHuddleChannelIdRef.current = null;
  }, [onHuddleEnded, state]);

  if (!visible || !isVisible(state)) return null;

  const activeState = state;
  const channelName = activeState.parent_channel_id
    ? channels.find((channel) => channel.id === activeState.parent_channel_id)
        ?.name
    : null;

  async function handleLeave(event: React.MouseEvent<HTMLButtonElement>) {
    event.stopPropagation();
    if (isLeaving) return;
    setIsLeaving(true);
    try {
      await leaveHuddle();
    } finally {
      setIsLeaving(false);
    }
  }

  async function handleOpenHuddleWindow() {
    try {
      await invoke("open_huddle_window");
    } catch (error) {
      console.error("Failed to open huddle window:", error);
    }
  }

  return (
    <div
      className="mb-2 flex min-w-0 items-center gap-2 rounded-xl border border-border/70 bg-background px-4 py-4 text-sidebar-foreground"
      data-buzz-content-surface
      data-buzz-content-unframed
      data-testid="profile-huddle-control"
    >
      <Button
        aria-label="Open huddle window"
        className="h-auto min-w-0 flex-1 justify-start gap-2 px-0 py-0 text-left hover:bg-transparent"
        onClick={() => void handleOpenHuddleWindow()}
        type="button"
        variant="ghost"
      >
        <Headphones
          aria-hidden="true"
          className="h-4 w-4 shrink-0 text-emerald-500"
        />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium leading-tight">
            In a huddle
          </span>
          <span className="block truncate text-xs text-sidebar-foreground/65">
            {channelName ? `#${channelName}` : "Huddle"}
          </span>
        </span>
      </Button>
      <div className="flex shrink-0 items-center gap-1.5">
        <MicControls
          audioDevices={audioDevices}
          compact
          isMuted={isMuted}
          isPttMode={voiceInputMode === "push_to_talk"}
          micConnected={micConnected}
          micGain={micGain}
          micLevel={micLevel}
          onGainChange={setMicGain}
          onSelectDevice={setSelectedDeviceId}
          onSelectVoiceInputMode={setVoiceInputMode}
          onToggleMute={toggleMute}
          selectedDeviceId={selectedDeviceId}
        />
        <Button
          aria-busy={isLeaving}
          aria-label="Leave huddle"
          className="h-8 px-2 text-sm text-sidebar-foreground/70 hover:bg-destructive/15 hover:text-destructive"
          disabled={isLeaving}
          onClick={(event) => void handleLeave(event)}
          type="button"
          variant="ghost"
        >
          Leave
        </Button>
      </div>
    </div>
  );
}
