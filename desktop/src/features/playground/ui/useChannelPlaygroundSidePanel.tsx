import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";

/** Match link side panel: native webview fill, no scroll gutter. */
const PLAYGROUND_SIDE_PANEL_BODY_CLASS =
  "overflow-hidden px-0 pb-0 flex min-h-0 flex-col";

import {
  playgroundConversationFromRoute,
  type PlaygroundConversation,
} from "../lib/conversation";
import {
  dismissPlayground,
  getPlaygroundStore,
  isPlaygroundSidePanelHost,
  subscribePlayground,
} from "../lib/sessions";
import { PlaygroundSidePanelBody } from "./PlaygroundSidePanelBody";

export type ChannelPlaygroundSidePanelChrome = {
  idleAuxiliaryBodyClassName: string;
  idleAuxiliaryCoverAppChrome: boolean;
  idleAuxiliaryExpanded: boolean;
  idleAuxiliaryHeaderActions: IdleAuxiliaryHeaderControls | undefined;
  idleAuxiliaryOverridesThread: boolean;
  idleAuxiliaryPanel: React.ReactNode;
  idleAuxiliaryTitle: string;
  onCloseIdleAuxiliaryPanel: () => void;
  open: boolean;
};

/**
 * Hosts an active playground session in the Projects-style right-hand idle
 * auxiliary slide-out (same shell as openLinkSidePanel) so pen / Watch / Drive
 * open keep Agent chrome + grants without left dock or inset-0 chat cover.
 */
export function useChannelPlaygroundSidePanel(): ChannelPlaygroundSidePanelChrome | null {
  const store = React.useSyncExternalStore(
    subscribePlayground,
    getPlaygroundStore,
    getPlaygroundStore,
  );
  const location = useLocation();

  const conversation = React.useMemo((): PlaygroundConversation | null => {
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

  const sidePanel = isPlaygroundSidePanelHost();
  const sid = store.overlaySid;
  const session = sidePanel && sid ? (store.sessions.get(sid) ?? null) : null;

  React.useEffect(() => {
    // Leaving the channel route parks the side-panel host so a webview cannot
    // cover Projects / other shells (same class of teardown as link panel).
    void location.pathname;
    return () => {
      if (isPlaygroundSidePanelHost()) {
        dismissPlayground();
      }
    };
  }, [location.pathname]);

  if (!session) return null;

  // Hard invariant: playground RHS never expands to full-bleed (link/pin may).
  // expanded/coverAppChrome true → FocusThreadDrawer leftPx=0 / fixed inset-0.
  // Enforced by playgroundSidePanelChromeIsCapped + unit tests.
  return {
    idleAuxiliaryBodyClassName: PLAYGROUND_SIDE_PANEL_BODY_CLASS,
    idleAuxiliaryCoverAppChrome: false,
    idleAuxiliaryExpanded: false,
    // PlaygroundChrome is the full tool row; IdleAux only needs title + X.
    idleAuxiliaryHeaderActions: undefined,
    idleAuxiliaryOverridesThread: true,
    idleAuxiliaryPanel: (
      <PlaygroundSidePanelBody conversation={conversation} session={session} />
    ),
    idleAuxiliaryTitle: session.name,
    onCloseIdleAuxiliaryPanel: dismissPlayground,
    open: true,
  };
}
