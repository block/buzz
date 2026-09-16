import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { deriveShellRoute } from "@/app/AppShell.helpers";

import { usePlaygroundSessions } from "../hooks";
import { playgroundConversationFromRoute } from "../lib/conversation";
import { usePlaygroundRuntime } from "../lib/runtime";
import { usePopoutLayoutPayload } from "@/features/popout/lib/popoutLayout";
import { PlaygroundOverlay } from "./PlaygroundOverlay";

export function PlaygroundHost() {
  const { sessions, overlaySid } = usePlaygroundSessions();
  usePlaygroundRuntime();
  const location = useLocation();
  const conversation = React.useMemo(() => {
    const route = deriveShellRoute(location.pathname);
    const search = location.search as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const thread = search.threadRootId ?? search.thread;
    return playgroundConversationFromRoute({
      selectedView: route.selectedView,
      selectedChannelId: route.selectedChannelId,
      threadId: typeof thread === "string" ? thread : null,
    });
  }, [location.pathname, location.search]);
  const popout = usePopoutLayoutPayload();
  const popoutSession = popout?.playground
    ? {
        sid: popout.playground.sid,
        name: popout.playground.name,
        url: popout.playground.url,
        pin: popout.playground.pin,
        stack: popout.playground.stack,
        expires:
          popout.playground.expires != null
            ? String(popout.playground.expires)
            : undefined,
        hasUpdate: false,
      }
    : null;
  // Pop-outs only host a playground when the payload includes one. Otherwise
  // a persisted overlaySid from the main window would cover a thread pop-out.
  const session = popout
    ? popoutSession
    : overlaySid
      ? (sessions.get(overlaySid) ?? null)
      : null;
  if (!session) return null;
  const lockPlacement =
    popout?.kind === "split"
      ? "dock"
      : popout?.kind === "playground"
        ? "window"
        : undefined;
  return (
    <PlaygroundOverlay
      conversation={conversation}
      lockPlacement={lockPlacement}
      session={session}
    />
  );
}
