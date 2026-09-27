import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { deriveShellRoute } from "@/app/AppShell.helpers";

import { usePlaygroundSessions } from "../hooks";
import {
  playgroundConversationFromPopout,
  playgroundConversationFromRoute,
} from "../lib/conversation";
import { usePlaygroundRuntime } from "../lib/runtime";
import {
  dismissPlayground,
  isPlaygroundSidePanelHost,
} from "../lib/sessions";
import { usePopoutLayoutPayload } from "@/features/popout/lib/popoutLayout";
import { FocusThreadDrawer } from "@/features/channels/ui/FocusThreadDrawer";
import { IdleAuxiliaryPanel } from "@/features/channels/ui/IdleAuxiliaryPanel";
import { THREAD_FOCUS_SLIVER_WIDTH_PX } from "@/features/channels/lib/threadFocusLayout";
import { PlaygroundOverlay } from "./PlaygroundOverlay";
import { PlaygroundSidePanelBody } from "./PlaygroundSidePanelBody";

/** Accessible back-target name for off-channel FocusThreadDrawer scrim. */
function offChannelDrawerBackLabel(
  selectedView: ReturnType<typeof deriveShellRoute>["selectedView"],
): string {
  switch (selectedView) {
    case "browsers":
      return "Browsers";
    case "agents":
      return "Agents";
    case "bots":
      return "Bots";
    case "home":
      return "Home";
    case "messages":
      return "Messages";
    case "projects":
      return "Projects";
    case "workflows":
      return "Workflows";
    case "pulse":
      return "Pulse";
    default:
      return "Buzz";
  }
}

export function PlaygroundHost() {
  const { sessions, overlaySid } = usePlaygroundSessions();
  usePlaygroundRuntime();
  const location = useLocation();
  const popout = usePopoutLayoutPayload();
  const conversation = React.useMemo(() => {
    const route = deriveShellRoute(location.pathname);
    const search = location.search as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const thread = search.threadRootId ?? search.thread;
    const fromRoute = playgroundConversationFromRoute({
      selectedView: route.selectedView,
      selectedChannelId: route.selectedChannelId,
      threadId: typeof thread === "string" ? thread : null,
    });
    if (fromRoute) return fromRoute;
    // Detached playground (and similar) may lack a channel route; use the
    // channel/thread that opened the window so Screenshot stages correctly.
    return playgroundConversationFromPopout({
      channelId: popout?.channelId,
      threadId: popout?.threadId,
    });
  }, [location.pathname, location.search, popout?.channelId, popout?.threadId]);
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
  const route = deriveShellRoute(location.pathname);
  // Pen / Watch / Drive host in ChannelScreen idle-auxiliary on channel routes.
  if (
    !popout &&
    isPlaygroundSidePanelHost() &&
    route.selectedView === "channel"
  ) {
    return null;
  }
  // Off-channel (Browsers, etc.): same FocusThreadDrawer slide-out as channel
  // pin Open / card Open — not a narrow docked RHS AuxiliaryPanel column.
  if (!popout && isPlaygroundSidePanelHost()) {
    return (
      <div
        className="absolute inset-0 z-40"
        data-testid="playground-offchannel-side-panel-host"
      >
        <FocusThreadDrawer
          channelName={offChannelDrawerBackLabel(route.selectedView)}
          // Never full-bleed: coverAppChrome + leftPx=0 is link/pin expand only.
          coverAppChrome={false}
          label={session.name}
          leftPx={THREAD_FOCUS_SLIVER_WIDTH_PX}
          onClose={dismissPlayground}
        >
          <IdleAuxiliaryPanel
            bodyClassName="overflow-hidden px-0 pb-0 flex min-h-0 flex-col"
            canResetWidth={false}
            isFocusDrawer
            isSinglePanelView
            onClose={dismissPlayground}
            onResetWidth={() => {}}
            onResizeStart={() => {}}
            title={session.name}
            useSplitAuxiliaryPane={false}
            widthPx={THREAD_FOCUS_SLIVER_WIDTH_PX}
          >
            <PlaygroundSidePanelBody
              conversation={conversation}
              session={session}
            />
          </IdleAuxiliaryPanel>
        </FocusThreadDrawer>
      </div>
    );
  }
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
