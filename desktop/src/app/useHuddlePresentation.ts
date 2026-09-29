import { useQueryClient } from "@tanstack/react-query";
import { useLocation } from "@tanstack/react-router";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";
import {
  loadHuddleBackingChannelIds,
  rememberHuddleBackingChannelId,
} from "@/app/huddleBackingChannelStorage";
import {
  type HuddlePresentation,
  reconcilePresentationWithNativeExists,
  shouldCoalesceHuddleCompanionOpen,
} from "@/app/huddlePresentation";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { channelsQueryKey } from "@/features/channels/hooks";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import {
  channelMessagesKey,
  channelWindowKey,
} from "@/features/messages/lib/messageQueryKeys";

import { leaveLeftNavBuzzTerm } from "@/features/terminal/terminalPanelStore";

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
  // Single source of truth: main app mounts the drawer bar XOR the companion
  // OS window — never both. "window" means the native huddle companion, not an
  // in-app expanded panel.
  const [presentation, setPresentation] =
    React.useState<HuddlePresentation>("none");
  const isHuddleDrawerOpen = presentation === "drawer";
  const isHuddleCompanionOpen = presentation === "window";
  const presentationEpochRef = React.useRef(0);
  const presentationRef = React.useRef<HuddlePresentation>(presentation);
  presentationRef.current = presentation;
  const companionExistsRef = React.useRef(false);
  const [isHuddleStartPending, setIsHuddleStartPending] = React.useState(false);
  const startPendingRef = React.useRef(false);
  const [revealedHuddleChannelIds, setRevealedHuddleChannelIds] =
    React.useState<ReadonlySet<string>>(() => new Set());
  const [huddleBackingChannelIds, setHuddleBackingChannelIds] = React.useState<
    ReadonlySet<string>
  >(loadHuddleBackingChannelIds);
  const activeHuddleChannelIdRef = React.useRef<string | null>(null);
  const [activeHuddleChannelId, setActiveHuddleChannelId] = React.useState<
    string | null
  >(null);
  const setActiveHuddleChannel = React.useCallback(
    (channelId: string | null) => {
      activeHuddleChannelIdRef.current = channelId;
      setActiveHuddleChannelId(channelId);
    },
    [],
  );
  const huddleCompanionChannelIdRef = React.useRef<string | null>(null);
  const huddleCompanionDismissedChannelIdRef = React.useRef<string | null>(
    null,
  );
  const huddleCompanionOpenPromiseRef = React.useRef<Promise<void> | null>(
    null,
  );
  /** True only while `open_huddle_window` invoke has not settled. */
  const huddleCompanionOpenPendingRef = React.useRef(false);
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
      startPendingRef.current = pending;
      setIsHuddleStartPending(pending);
      if (pending) {
        // Keep main unmounted for the whole start/join → companion open
        // transition so the dock never flashes on the main window.
        presentationEpochRef.current += 1;
        setPresentation("none");
        return;
      }
      // Companion-only: never fall back to a main-window drawer. If open is
      // still needed after start/join settles, openHuddleCompanion / state
      // listeners own that path.
    },
    [],
  );
  const handleHuddleVisibilityChange = React.useCallback((visible: boolean) => {
    // Companion-only product: visibility must never mount main drawer chrome.
    // Opening the companion is owned by start/join / indicator / PIP.
    if (!visible && presentationRef.current === "drawer") {
      setPresentation("none");
    }
  }, []);
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

  const openHuddleCompanion = React.useCallback(
    (ephemeralChannelId: string, options?: { force?: boolean }) => {
      setActiveHuddleChannel(ephemeralChannelId);
      trackHuddleBackingChannel(ephemeralChannelId);

      // Auto-open (huddle start / creating phase) must not fight an explicit
      // user close of the companion. Force (indicator / PIP) clears dismissed.
      if (
        !options?.force &&
        huddleCompanionDismissedChannelIdRef.current === ephemeralChannelId
      ) {
        // User closed the companion on purpose — stay off main chrome.
        if (presentationRef.current !== "none") {
          presentationEpochRef.current += 1;
          companionExistsRef.current = false;
          setPresentation("none");
        }
        return Promise.resolve();
      }

      huddleCompanionDismissedChannelIdRef.current = null;
      // Clear stale open promise so a closed companion always invokes native open.
      if (options?.force) {
        huddleCompanionChannelIdRef.current = null;
        huddleCompanionOpenPromiseRef.current = null;
        huddleCompanionOpenPendingRef.current = false;
      }
      hideHuddleChannel(ephemeralChannelId);

      // Coalesce only an in-flight open. A settled promise must not skip native
      // open — after the user closes the companion, reusing it flips presentation
      // to "window" with no OS window (#161).
      if (
        shouldCoalesceHuddleCompanionOpen({
          sameChannel:
            huddleCompanionChannelIdRef.current === ephemeralChannelId,
          openInFlight: huddleCompanionOpenPendingRef.current,
        }) &&
        huddleCompanionOpenPromiseRef.current
      ) {
        return huddleCompanionOpenPromiseRef.current;
      }

      // Bump epoch before flipping to window so in-flight companion-returned
      // handlers (late Destroyed after dock) cannot fight this expand.
      const openEpoch = presentationEpochRef.current + 1;
      presentationEpochRef.current = openEpoch;
      companionExistsRef.current = true;
      setPresentation("window");

      huddleCompanionChannelIdRef.current = ephemeralChannelId;
      huddleCompanionOpenPendingRef.current = true;
      const openPromise = invoke<void>("open_huddle_window")
        .then(() => {
          huddleCompanionOpenPendingRef.current = false;
          // Open settled — keep window presentation if this expand is current.
          if (presentationEpochRef.current !== openEpoch) return;
          companionExistsRef.current = true;
          setPresentation("window");
        })
        .catch(async (error) => {
          huddleCompanionOpenPendingRef.current = false;
          if (
            huddleCompanionChannelIdRef.current !== ephemeralChannelId ||
            presentationEpochRef.current !== openEpoch
          ) {
            throw error;
          }
          // Native may still hold the companion after an "already exists" race.
          // Probe before demoting — never mount main drawer beside a live window.
          const exists = await invoke<boolean>(
            "huddle_companion_window_exists",
          ).catch(() => false);
          if (exists) {
            companionExistsRef.current = true;
            setPresentation("window");
            return;
          }
          const message =
            error instanceof Error ? error.message : String(error ?? "");
          if (message.toLowerCase().includes("already exists")) {
            companionExistsRef.current = true;
            setPresentation("window");
            return;
          }
          huddleCompanionChannelIdRef.current = null;
          huddleCompanionOpenPromiseRef.current = null;
          companionExistsRef.current = false;
          // True failed expand — stay off main chrome (companion-only product).
          setPresentation("none");
          throw error;
        });
      huddleCompanionOpenPromiseRef.current = openPromise;
      return openPromise;
    },
    [hideHuddleChannel, setActiveHuddleChannel, trackHuddleBackingChannel],
  );
  const handleHuddleCompanionOpen = React.useCallback(async () => {
    const ephemeralChannelId = activeHuddleChannelIdRef.current;
    if (!ephemeralChannelId) return;
    try {
      await openHuddleCompanion(ephemeralChannelId, { force: true });
    } catch (error) {
      console.error("Failed to open huddle window:", error);
      return;
    }

    const parentChannelId = activeHuddleParentChannelIdRef.current;
    if (
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
    location.pathname,
    openHuddleCompanion,
    returnMainWindowToHuddleParent,
  ]);

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
      setActiveHuddleChannel(ephemeralChannelId);
      trackHuddleBackingChannel(ephemeralChannelId);
      viewHuddleChannel(ephemeralChannelId);
    },
    [setActiveHuddleChannel, trackHuddleBackingChannel, viewHuddleChannel],
  );
  const handleSidebarChannelSelect = React.useCallback(
    (channelId: string) => {
      leaveLeftNavBuzzTerm();
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
      setActiveHuddleChannel(null);
      activeHuddleParentChannelIdRef.current = null;
      huddleCompanionChannelIdRef.current = null;
      huddleCompanionDismissedChannelIdRef.current = null;
      huddleCompanionOpenPromiseRef.current = null;
      huddleCompanionOpenPendingRef.current = false;
      companionExistsRef.current = false;
      presentationEpochRef.current += 1;
      setPresentation("none");
      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    },
    [
      hideHuddleChannel,
      queryClient,
      returnToHuddleParentAfterEnd,
      setActiveHuddleChannel,
    ],
  );

  // Heal React↔native desync: if the companion OS window exists, main must
  // stay on presentation "window" so AppHuddleShell unmounts the drawer bar.
  React.useEffect(() => {
    if (isHuddleRoom) return;

    let cancelled = false;
    const syncFromNative = () => {
      void invoke<boolean>("huddle_companion_window_exists")
        .catch(() => false)
        .then((exists) => {
          if (cancelled) return;
          companionExistsRef.current = exists;
          const openInFlight = huddleCompanionOpenPendingRef.current;
          const next = reconcilePresentationWithNativeExists(
            presentationRef.current,
            exists,
            {
              openInFlight,
              huddleActive: activeHuddleChannelIdRef.current != null,
            },
          );
          if (!next) return;
          presentationEpochRef.current += 1;
          if (next !== "window") {
            // Companion is gone — drop stale open promise so the next expand
            // invokes native open instead of reusing a settled no-op (#161).
            huddleCompanionChannelIdRef.current = null;
            huddleCompanionOpenPromiseRef.current = null;
            huddleCompanionOpenPendingRef.current = false;
          }
          setPresentation(next);
          if (next === "window") {
            // Keep main off the ephemeral transcript while the companion owns UI.
            hideHuddleChannel(activeHuddleChannelIdRef.current);
            void invoke<HuddleTranscriptRouteState>("get_huddle_state")
              .then((state) => {
                if (cancelled) return;
                returnMainWindowToHuddleParent(state);
              })
              .catch(() => {
                /* best-effort parent restore */
              });
          }
        });
    };

    syncFromNative();
    const intervalId = window.setInterval(syncFromNative, 750);
    const onFocus = () => syncFromNative();
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [hideHuddleChannel, isHuddleRoom, returnMainWindowToHuddleParent]);

  React.useEffect(() => {
    if (isHuddleRoom) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void listen("huddle-companion-returned", () => {
      if (cancelled) return;
      // Capture epoch at event time. An expand that starts after this bumps the
      // epoch and must win over a late Destroyed from the prior dock/recreate.
      const epochAtEvent = presentationEpochRef.current;
      const settleAfterCompanionClose = () => {
        if (cancelled) return;
        if (presentationEpochRef.current !== epochAtEvent) return;
        void invoke<boolean>("huddle_companion_window_exists")
          .catch(() => false)
          .then((stillOpen) => {
            if (cancelled || stillOpen) return;
            if (presentationEpochRef.current !== epochAtEvent) return;
            // User closed companion — mark dismissed so auto-open will not
            // fight them, clear stale open refs, and stay off main chrome.
            // Re-open via Huddle indicator / Join (force) or start again.
            huddleCompanionDismissedChannelIdRef.current =
              activeHuddleChannelIdRef.current;
            huddleCompanionChannelIdRef.current = null;
            huddleCompanionOpenPromiseRef.current = null;
            huddleCompanionOpenPendingRef.current = false;
            companionExistsRef.current = false;
            presentationEpochRef.current += 1;
            setPresentation("none");
            void invoke<HuddleTranscriptRouteState>("get_huddle_state")
              .then((state) => {
                if (!state.ephemeral_channel_id) return;
                if (state.parent_channel_id) {
                  activeHuddleParentChannelIdRef.current =
                    state.parent_channel_id;
                }
                // Keep main on the parent channel — never promote drawer UI.
                hideHuddleChannel(state.ephemeral_channel_id);
                returnMainWindowToHuddleParent(state);
              })
              .catch((error) => {
                console.error(
                  "Failed to restore parent after companion close:",
                  error,
                );
              });
          });
      };
      // If an expand is in flight, wait until open settles so exists() is not
      // checked against a half-built companion (false negative → drawer+window).
      const openPromise = huddleCompanionOpenPromiseRef.current;
      if (openPromise) {
        void openPromise.then(
          settleAfterCompanionClose,
          settleAfterCompanionClose,
        );
        return;
      }
      settleAfterCompanionClose();
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [hideHuddleChannel, isHuddleRoom, returnMainWindowToHuddleParent]);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void invoke<HuddleTranscriptRouteState>("get_huddle_state")
      .then((state) => {
        if (cancelled || !state.ephemeral_channel_id) return;
        setActiveHuddleChannel(state.ephemeral_channel_id);
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
        setActiveHuddleChannel(event.payload.ephemeral_channel_id);
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
      // Companion-only: never promote main drawer. If the session is live and
      // no companion exists / open is in flight, open the companion (unless the
      // user explicitly dismissed it).
      if (
        !isHuddleRoom &&
        (event.payload.phase === "active" ||
          event.payload.phase === "connected") &&
        event.payload.ephemeral_channel_id &&
        !companionExistsRef.current &&
        !huddleCompanionOpenPendingRef.current &&
        !startPendingRef.current &&
        huddleCompanionDismissedChannelIdRef.current !==
          event.payload.ephemeral_channel_id
      ) {
        void openHuddleCompanion(event.payload.ephemeral_channel_id).catch(
          (error) => {
            console.error(
              "Failed to open huddle companion for active session:",
              error,
            );
          },
        );
      }
      if (event.payload.phase === "idle") {
        const endedChannelId = activeHuddleChannelIdRef.current;
        const parentChannelId = activeHuddleParentChannelIdRef.current;
        returnToHuddleParentAfterEnd(endedChannelId, parentChannelId);
        hideHuddleChannel(endedChannelId);
        setActiveHuddleChannel(null);
        activeHuddleParentChannelIdRef.current = null;
        huddleCompanionChannelIdRef.current = null;
        huddleCompanionDismissedChannelIdRef.current = null;
        huddleCompanionOpenPromiseRef.current = null;
        huddleCompanionOpenPendingRef.current = false;
        companionExistsRef.current = false;
        presentationEpochRef.current += 1;
        setPresentation("none");
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
    setActiveHuddleChannel,
    trackHuddleBackingChannel,
  ]);

  return {
    activeHuddleChannelId,
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
    presentation,
    showHuddleInMainApp,
    viewHuddleChannel,
  };
}
