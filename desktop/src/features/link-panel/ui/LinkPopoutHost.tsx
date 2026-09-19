import * as React from "react";

import { currentPopoutPayload } from "@/features/popout/lib/popoutWindow";
import { usePopoutLayoutPayload } from "@/features/popout/lib/popoutLayout";

import type { LinkSidePanelViewportMode } from "../lib/linkSidePanelStore";
import { LinkSidePanelChrome } from "./LinkSidePanelChrome";
import { LinkSidePanelSurface } from "./LinkSidePanelSurface";

/**
 * Full-window host for a detached link/pin browser (`popout` kind `"link"`).
 * Reuses slide-out chrome + surface; pin webviews are already window-scoped.
 */
export function LinkPopoutHost() {
  const popout = usePopoutLayoutPayload() ?? currentPopoutPayload();
  const link = popout?.kind === "link" ? popout.link : null;
  const [viewportMode, setViewportMode] =
    React.useState<LinkSidePanelViewportMode>(
      link?.viewportMode ?? "desktop",
    );

  React.useEffect(() => {
    if (link?.viewportMode) setViewportMode(link.viewportMode);
  }, [link?.viewportMode]);

  if (!link) return null;

  return (
    <div
      className="flex h-full min-h-0 min-w-0 w-full flex-1 flex-col bg-background"
      data-testid="link-popout-host"
    >
      <div className="shrink-0 border-b border-border px-3 py-2">
        <LinkSidePanelChrome
          detached
          onViewportModeChange={setViewportMode}
          pinId={link.pinId}
          url={link.url}
          viewportMode={viewportMode}
        />
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <LinkSidePanelSurface
          keepAlive={link.keepAlive}
          pinId={link.pinId}
          url={link.url}
          viewportMode={viewportMode}
        />
      </div>
    </div>
  );
}
