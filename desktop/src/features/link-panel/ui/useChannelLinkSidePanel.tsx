import { Maximize2, Minimize2 } from "lucide-react";
import * as React from "react";

import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import {
  closeLinkSidePanel,
  getLinkSidePanelStore,
  subscribeLinkSidePanel,
  toggleLinkSidePanelExpanded,
} from "../lib/linkSidePanelStore";
import { LinkSidePanelSurface } from "./LinkSidePanelSurface";

export type ChannelLinkSidePanelChrome = {
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
    const expandLabel = panel.expanded
      ? "Exit full screen"
      : "Expand link panel";
    return {
      actions: (
        <Tooltip disableHoverableContent>
          <TooltipTrigger asChild>
            <Button
              aria-label={expandLabel}
              className="shrink-0"
              data-testid="link-side-panel-expand"
              onClick={() => toggleLinkSidePanelExpanded()}
              size="icon"
              title={expandLabel}
              type="button"
              variant="ghost"
            >
              {panel.expanded ? <Minimize2 /> : <Maximize2 />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{expandLabel}</TooltipContent>
        </Tooltip>
      ),
    } satisfies IdleAuxiliaryHeaderControls;
  }, [panel]);

  if (!panel || !headerActions) return null;

  return {
    idleAuxiliaryHeaderActions: headerActions,
    idleAuxiliaryOverridesThread: panel.expanded,
    idleAuxiliaryPanel: (
      <LinkSidePanelSurface
        keepAlive={panel.keepAlive}
        pinId={panel.pinId}
        url={panel.url}
      />
    ),
    idleAuxiliaryTitle: panel.title,
    onCloseIdleAuxiliaryPanel: closeLinkSidePanel,
    open: true,
  };
}
