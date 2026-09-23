import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";

import {
  closeLinkSidePanel,
  getLinkSidePanelStore,
  subscribeLinkSidePanel,
} from "../lib/linkSidePanelStore";
import { LinkSidePanelChrome } from "./LinkSidePanelChrome";
import { LinkSidePanelSurface } from "./LinkSidePanelSurface";

/** Body fill for native webview — no scroll gutter under Desktop mode. */
export const LINK_SIDE_PANEL_BODY_CLASS =
  "overflow-hidden px-0 pb-0 flex flex-col";

export type ChannelLinkSidePanelChrome = {
  idleAuxiliaryBodyClassName: string;
  idleAuxiliaryCoverAppChrome: boolean;
  idleAuxiliaryExpanded: boolean;
  idleAuxiliaryHeaderActions: IdleAuxiliaryHeaderControls;
  idleAuxiliaryOverridesThread: boolean;
  idleAuxiliaryPanel: React.ReactNode;
  idleAuxiliaryTitle: string;
  onCloseIdleAuxiliaryPanel: () => void;
  open: boolean;
};

/**
 * Projects Tasks / Reviews sheet chrome reused for http(s) link opens:
 * right-hand idle auxiliary panel with independent expand + close.
 * Both states slide in via the focus drawer; default open matches Projects
 * width (channel sliver); expand goes true full-bleed. Thread override
 * stays always-on (#63).
 *
 * Leaving the channel route (Projects, etc.) clears the panel so a
 * fullscreen webview cannot cover the destination.
 */
export function useChannelLinkSidePanel(): ChannelLinkSidePanelChrome | null {
  const store = React.useSyncExternalStore(
    subscribeLinkSidePanel,
    getLinkSidePanelStore,
    getLinkSidePanelStore,
  );
  const panel = store.panel;
  const _location = useLocation();

  React.useEffect(() => {
    // Pathname change (or ChannelScreen unmount) tears down fullscreen /
    // side web so Projects and other shells are not covered.
    return () => {
      closeLinkSidePanel();
    };
  }, []);

  const headerActions = React.useMemo(() => {
    if (!panel) return undefined;
    return {
      actions: (
        <LinkSidePanelChrome
          pinId={panel.pinId}
          url={panel.url}
          viewportMode={panel.viewportMode}
        />
      ),
    } satisfies IdleAuxiliaryHeaderControls;
  }, [panel]);

  if (!panel || !headerActions) return null;

  return {
    idleAuxiliaryBodyClassName: LINK_SIDE_PANEL_BODY_CLASS,
    idleAuxiliaryCoverAppChrome: panel.expanded,
    idleAuxiliaryExpanded: panel.expanded,
    idleAuxiliaryHeaderActions: headerActions,
    // Match Project workspace sheets: any open link panel covers the
    // thread slot, not only the expanded/fullscreen state.
    idleAuxiliaryOverridesThread: true,
    idleAuxiliaryPanel: (
      <LinkSidePanelSurface
        keepAlive={panel.keepAlive}
        pinId={panel.pinId}
        url={panel.url}
        viewportMode={panel.viewportMode}
      />
    ),
    idleAuxiliaryTitle: panel.title,
    onCloseIdleAuxiliaryPanel: closeLinkSidePanel,
    open: true,
  };
}
