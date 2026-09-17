import * as React from "react";

import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";

import {
  closeLinkSidePanel,
  getLinkSidePanelStore,
  subscribeLinkSidePanel,
} from "../lib/linkSidePanelStore";
import { LinkSidePanelChrome } from "./LinkSidePanelChrome";
import { LinkSidePanelSurface } from "./LinkSidePanelSurface";

export type ChannelLinkSidePanelChrome = {
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
 */
export function useChannelLinkSidePanel(): ChannelLinkSidePanelChrome | null {
  const store = React.useSyncExternalStore(
    subscribeLinkSidePanel,
    getLinkSidePanelStore,
    getLinkSidePanelStore,
  );
  const panel = store.panel;

  const headerActions = React.useMemo(() => {
    if (!panel) return undefined;
    return {
      actions: (
        <LinkSidePanelChrome
          expanded={panel.expanded}
          pinId={panel.pinId}
          url={panel.url}
          viewportMode={panel.viewportMode}
        />
      ),
    } satisfies IdleAuxiliaryHeaderControls;
  }, [panel]);

  if (!panel || !headerActions) return null;

  return {
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
