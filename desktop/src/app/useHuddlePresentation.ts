import { useQueryClient } from "@tanstack/react-query";
import { useLocation } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";
import {
  loadHuddleBackingChannelIds,
  rememberHuddleBackingChannelId,
} from "@/app/huddleBackingChannelStorage";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { channelsQueryKey } from "@/features/channels/hooks";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import {
  channelMessagesKey,
  channelWindowKey,
} from "@/features/messages/lib/messageQueryKeys";

type HuddleTranscriptRouteState = {
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
};

export function useHuddlePresentation() {
  const huddleRoomChannelId = huddleWindowChannelId();
  const isHuddleRoom = huddleRoomChannelId !== null;
  const [isHuddleDrawerOpen, setIsHuddleDrawerOpen] = React.useState(false);
  const [isHuddleCompanionOpen, setIsHuddleCompanionOpen] =
    React.useState(false);
  const [isHuddleStartPending, setIsHuddleStartPending] = React.useState(false);
  const [revealedHuddleChannelIds, setRevealedHuddleChannelIds] =
    React.useState<ReadonlySet<string>>(() => new Set());
  const [huddleBackingChannelIds, setHuddleBackingChannelIds] = React.useState<
    ReadonlySet<string>
  >(loadHuddleBackingChannelIds);
  const activeHuddleChannelIdRef = React.useRef<string | null>(null);
  const huddleCompanionChannelIdRef = React.useRef<string | null>(null);
  const huddleCompanionDismissedChannelIdRef = React.useRef<string | null>(
    null,
  );
  const huddleCompanionOpenPromiseRef = React.useRef<Promise<void> | null>(
    null,
  );
  const activeHuddleParentChannelIdRef = React.useRef<string | null>(null);
  const [huddleTranscriptRoute, setHuddleTranscriptRoute] =
    React.useState<HuddleTranscriptRouteState | null>(null);
  const location = useLocation();
  const queryClient = useQueryClient();
  const { goChannel } = useAppNavigation();

  React.useEffect(() => {
    if (!isHuddleRoom) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const syncRoute = (state: HuddleTranscriptRouteState) => {
      if (!cancelled) setHuddleTranscriptRoute(state);
    };

    void invoke<HuddleTranscriptRouteState>("get_huddle_state")
      .then(syncRoute)
      .catch((error) => {
        console.error("Failed to resolve huddle transcript route:", error);
        if (!cancelled) {
          setHuddleTranscriptRoute({
            ephemeral_channel_id: huddleRoomChannelId,
            huddle_thread_event_id: null,
            parent_channel_id: null,
            phase: "active",
          });
        }
      });
    void listen<HuddleTranscriptRouteState>("huddle-state-changed", (event) =>
      syncRoute(event.payload),
    ).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [huddleRoomChannelId, isHuddleRoom]);

  const huddleRouteResolved = huddleTranscriptRoute !== null;
  const huddleRouteEphemeralChannelId =
    huddleTranscriptRoute?.ephemeral_channel_id ?? null;
  const huddleRouteIsActive = huddleTranscriptRoute?.phase === "active";
  const huddleRouteDestinationChannelId =
    huddleRouteEphemeralChannelId ?? huddleRoomChannelId;
  const huddleRouteMatchesLocation = Boolean(
    huddleRouteDestinationChannelId &&
      location.pathname === `/channels/${huddleRouteDestinationChannelId}`,
  );
  const isHuddleRoomStarting =
    isHuddleRoom &&
    (!huddleRouteResolved ||
      !huddleRouteIsActive ||
      !huddleRouteMatchesLocation);

  React.useEffect(() => {
    if (!huddleRoomChannelId || !huddleRouteResolved || !huddleRouteIsActive) {
      return;
    }

    let cancelled = false;
    const channelId = huddleRouteEphemeralChannelId ?? huddleRoomChannelId;
    void Promise.all([
      queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
      queryClient.invalidateQueries({
        queryKey: channelMessagesKey(channelId),
      }),
      queryClient.invalidateQueries({ queryKey: channelWindowKey(channelId) }),
    ]).then(() => {
      if (!cancelled) void goChannel(channelId, { replace: true });
    });

    return () => {
      cancelled = true;
    };
  }, [
    goChannel,
    huddleRoomChannelId,
    huddleRouteEphemeralChannelId,
    huddleRouteIsActive,
    huddleRouteResolved,
    queryClient,
  ]);

  const handleHuddleStartPendingChange = React.useCallback(
    (pending: boolean) => {
      setIsHuddleStartPending(pending);
      if (pending) setIsHuddleDrawerOpen(false);
    },
    [],
  );
  const handleHuddleVisibilityChange = React.useCallback(
    (visible: boolean) => {
      setIsHuddleDrawerOpen(
        visible && !isHuddleStartPending && !isHuddleCompanionOpen,
      );
    },
    [isHuddleCompanionOpen, isHuddleStartPending],
  );
  const hideHuddleChannel = React.useCallback(
    (ephemeralChannelId: string | null | undefined) => {
      if (!ephemeralChannelId) return;
      setRevealedHuddleChannelIds((current) => {
        if (!current.has(ephemeralChannelId)) return current;
        const next = new Set(current);
        next.delete(ephemeralChannelId);
        return next;
      });
    },
    [],
  );
  const trackHuddleBackingChannel = React.useCallback(
    (ephemeralChannelId: string) => {
      rememberHuddleBackingChannelId(ephemeralChannelId);
      setHuddleBackingChannelIds((current) => {
        if (current.has(ephemeralChannelId)) return current;
        const next = new Set(current);
        next.add(ephemeralChannelId);
        return next;
      });
    },
    [],
  );
  const revealHuddleChannel = React.useCallback(
    (ephemeralChannelId: string) => {
      setRevealedHuddleChannelIds((current) => {
        if (current.has(ephemeralChannelId)) return current;
        const next = new Set(current);
        next.add(ephemeralChannelId);
        return next;
      });
    },
    [],
  );
  const returnMainWindowToHuddleParent = React.useCallback(
    (state: HuddleTranscriptRouteState) => {
      const ephemeralChannelId = state.ephemeral_channel_id;
      const parentChannelId = state.parent_channel_id;
      if (parentChannelId) {
        activeHuddleParentChannelIdRef.current = parentChannelId;
      }
      if (
        ephemeralChannelId &&
        parentChannelId &&
        location.pathname === `/channels/${ephemeralChannelId}`
      ) {
        void goChannel(parentChannelId, { replace: true });
      }
    },
    [goChannel, location.pathname],
  );
  const returnToHuddleParentAfterEnd = React.useCallback(
    (ephemeralChannelId: string | null, parentChannelId: string | null) => {
      if (
        ephemeralChannelId &&
        parentChannelId &&
        location.pathname === `/channels/${ephemeralChannelId}`
      ) {
        void goChannel(parentChannelId, { replace: true });
      }
    },
    [goChannel, location.pathname],
  );
  const handleHuddleCompanionOpen = React.useCallback(() => {
    const ephemeralChannelId = activeHuddleChannelIdRef.current;
    huddleCompanionDismissedChannelIdRef.current = null;
    hideHuddleChannel(ephemeralChannelId);
    setIsHuddleDrawerOpen(false);
    setIsHuddleCompanionOpen(true);

    const parentChannelId = activeHuddleParentChannelIdRef.current;
    if (
      ephemeralChannelId &&
      parentChannelId &&
      location.pathname === `/channels/${ephemeralChannelId}`
    ) {
      void goChannel(parentChannelId, { replace: true });
      return;
    }

    void invoke<HuddleTranscriptRouteState>("get_huddle_state")
      .then(returnMainWindowToHuddleParent)
      .catch((error) => {
        console.error("Failed to restore the huddle parent channel:", error);
      });
  }, [
    goChannel,
    hideHuddleChannel,
    location.pathname,
    returnMainWindowToHuddleParent,
  ]);
  const openHuddleCompanion = React.useCallback(
    (ephemeralChannelId: string) => {
      activeHuddleChannelIdRef.current = ephemeralChannelId;
      trackHuddleBackingChannel(ephemeralChannelId);

      if (huddleCompanionDismissedChannelIdRef.current === ephemeralChannelId) {
        return Promise.resolve();
      }

      huddleCompanionDismissedChannelIdRef.current = null;
      hideHuddleChannel(ephemeralChannelId);
      setIsHuddleDrawerOpen(false);
      setIsHuddleCompanionOpen(true);

      if (
        huddleCompanionChannelIdRef.current === ephemeralChannelId &&
        huddleCompanionOpenPromiseRef.current
      ) {
        return huddleCompanionOpenPromiseRef.current;
      }

      huddleCompanionChannelIdRef.current = ephemeralChannelId;
      const openPromise = invoke<void>("open_huddle_window").catch((error) => {
        if (huddleCompanionChannelIdRef.current === ephemeralChannelId) {
          huddleCompanionChannelIdRef.current = null;
          huddleCompanionOpenPromiseRef.current = null;
          setIsHuddleCompanionOpen(false);
        }
        throw error;
      });
      huddleCompanionOpenPromiseRef.current = openPromise;
      return openPromise;
    },
    [hideHuddleChannel, trackHuddleBackingChannel],
  );
  const handleHuddleStarted = React.useCallback(
    async (ephemeralChannelId: string) => {
      try {
        await openHuddleCompanion(ephemeralChannelId);
      } catch (error) {
        revealHuddleChannel(ephemeralChannelId);
        throw error;
      }
    },
    [openHuddleCompanion, revealHuddleChannel],
  );
  const viewHuddleChannel = React.useCallback(
    (ephemeralChannelId: string) => {
      revealHuddleChannel(ephemeralChannelId);
      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: channelMessagesKey(ephemeralChannelId),
      });
      void queryClient.invalidateQueries({
        queryKey: channelWindowKey(ephemeralChannelId),
      });
      void goChannel(ephemeralChannelId);
    },
    [goChannel, queryClient, revealHuddleChannel],
  );
  const showHuddleInMainApp = React.useCallback(
    (ephemeralChannelId: string) => {
      activeHuddleChannelIdRef.current = ephemeralChannelId;
      trackHuddleBackingChannel(ephemeralChannelId);
      viewHuddleChannel(ephemeralChannelId);
    },
    [trackHuddleBackingChannel, viewHuddleChannel],
  );
  const handleSidebarChannelSelect = React.useCallback(
    (channelId: string) => {
      if (
        isHuddleDrawerOpen &&
        channelId === activeHuddleChannelIdRef.current
      ) {
        showHuddleInMainApp(channelId);
        return;
      }
      void goChannel(channelId);
    },
    [goChannel, isHuddleDrawerOpen, showHuddleInMainApp],
  );
  const handleHuddleEnded = React.useCallback(
    (ephemeralChannelId: string | null) => {
      const endedChannelId =
        ephemeralChannelId ?? activeHuddleChannelIdRef.current;
      returnToHuddleParentAfterEnd(
        endedChannelId,
        activeHuddleParentChannelIdRef.current,
      );
      hideHuddleChannel(endedChannelId);
      activeHuddleChannelIdRef.current = null;
      activeHuddleParentChannelIdRef.current = null;
      huddleCompanionChannelIdRef.current = null;
      huddleCompanionDismissedChannelIdRef.current = null;
      huddleCompanionOpenPromiseRef.current = null;
      setIsHuddleCompanionOpen(false);
      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    },
    [hideHuddleChannel, queryClient, returnToHuddleParentAfterEnd],
  );

  React.useEffect(() => {
    if (isHuddleRoom) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void listen("huddle-companion-returned", () => {
      if (cancelled) return;
      huddleCompanionDismissedChannelIdRef.current =
        activeHuddleChannelIdRef.current;
      huddleCompanionChannelIdRef.current = null;
      huddleCompanionOpenPromiseRef.current = null;
      setIsHuddleCompanionOpen(false);
      setIsHuddleDrawerOpen(true);
      void invoke<HuddleTranscriptRouteState>("get_huddle_state")
        .then((state) => {
          if (!state.ephemeral_channel_id) return;
          if (state.parent_channel_id) {
            activeHuddleParentChannelIdRef.current = state.parent_channel_id;
          }
          showHuddleInMainApp(state.ephemeral_channel_id);
        })
        .catch((error) => {
          console.error("Failed to open huddle in the main app:", error);
        });
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [isHuddleRoom, showHuddleInMainApp]);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void invoke<HuddleTranscriptRouteState>("get_huddle_state")
      .then((state) => {
        if (cancelled || !state.ephemeral_channel_id) return;
        activeHuddleChannelIdRef.current = state.ephemeral_channel_id;
        trackHuddleBackingChannel(state.ephemeral_channel_id);
        if (state.parent_channel_id) {
          activeHuddleParentChannelIdRef.current = state.parent_channel_id;
        }
      })
      .catch(() => {
        /* lifecycle events remain authoritative */
      });
    listen<HuddleTranscriptRouteState>("huddle-state-changed", (event) => {
      if (cancelled) return;
      if (event.payload.ephemeral_channel_id) {
        activeHuddleChannelIdRef.current = event.payload.ephemeral_channel_id;
        trackHuddleBackingChannel(event.payload.ephemeral_channel_id);
      }
      if (event.payload.parent_channel_id) {
        activeHuddleParentChannelIdRef.current =
          event.payload.parent_channel_id;
      }
      if (
        !isHuddleRoom &&
        event.payload.phase === "creating" &&
        event.payload.ephemeral_channel_id
      ) {
        void openHuddleCompanion(event.payload.ephemeral_channel_id).catch(
          (error) => {
            console.error("Failed to open starting huddle window:", error);
          },
        );
      }
      if (event.payload.phase === "idle") {
        const endedChannelId = activeHuddleChannelIdRef.current;
        const parentChannelId = activeHuddleParentChannelIdRef.current;
        returnToHuddleParentAfterEnd(endedChannelId, parentChannelId);
        hideHuddleChannel(endedChannelId);
        activeHuddleChannelIdRef.current = null;
        activeHuddleParentChannelIdRef.current = null;
        huddleCompanionChannelIdRef.current = null;
        huddleCompanionDismissedChannelIdRef.current = null;
        huddleCompanionOpenPromiseRef.current = null;
        setIsHuddleDrawerOpen(false);
        setIsHuddleCompanionOpen(false);
        setIsHuddleStartPending(false);
        void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      }
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [
    hideHuddleChannel,
    isHuddleRoom,
    openHuddleCompanion,
    queryClient,
    returnToHuddleParentAfterEnd,
    trackHuddleBackingChannel,
  ]);

  return {
    handleHuddleCompanionOpen,
    handleHuddleEnded,
    handleHuddleStartPendingChange,
    handleHuddleStarted,
    handleHuddleVisibilityChange,
    handleSidebarChannelSelect,
    huddleBackingChannelIds,
    revealedHuddleChannelIds,
    isHuddleCompanionOpen,
    isHuddleDrawerOpen,
    isHuddleRoom,
    isHuddleRoomStarting,
    isHuddleStartPending,
    showHuddleInMainApp,
    viewHuddleChannel,
  };
}
