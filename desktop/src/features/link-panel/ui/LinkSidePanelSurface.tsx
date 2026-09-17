import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import {
  closePinWebview,
  hidePinWebview,
  PIN_WEBVIEW_RESTORE_EVENT,
  pinWebviewBoundsAreUsable,
  showPinWebview,
  subscribePinWebviewLoad,
  type PinWebviewBounds,
} from "@/features/pinned-sites/lib/pinWebview";
import { isNativeWebviewModalParked } from "@/shared/lib/nativeWebviewModalPark";
import { Button } from "@/shared/ui/button";

import { LINK_SIDE_PANEL_PIN_ID } from "../lib/linkSidePanelStore";

function readBounds(element: HTMLElement): PinWebviewBounds {
  const rect = element.getBoundingClientRect();
  return {
    x: Math.round(rect.left),
    y: Math.round(rect.top),
    width: Math.max(1, Math.round(rect.width)),
    height: Math.max(1, Math.round(rect.height)),
  };
}

/**
 * Hosts the native pin webview inside the Projects-style idle auxiliary panel.
 * Mirrors {@link PinnedSiteSurface} show/hide. Late dismiss is cancelled inside
 * showPinWebview (hide-epoch + per-pin show generation) so remounts do not
 * blank the first open (PRs #53/#54 patterns).
 */
export function LinkSidePanelSurface({
  keepAlive = false,
  pinId = LINK_SIDE_PANEL_PIN_ID,
  url,
}: {
  keepAlive?: boolean;
  pinId?: string;
  url: string;
}) {
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  const native = isTauri() || import.meta.env.MODE === "e2e";
  const [loadError, setLoadError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!native) return;
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
        const openOrResize = () => {
      if (cancelled || !hostRef.current) return;
      if (isNativeWebviewModalParked()) return;
      const bounds = readBounds(hostRef.current);
      if (!pinWebviewBoundsAreUsable(bounds)) return;
      // Always show — same blank-until-interaction class as pins/playground.
      void showPinWebview({
        pinId,
        startUrl: url,
        bounds,
      }).catch((error) => {
        console.error("Failed to open link side panel", error);
        if (!cancelled) {
          setLoadError(
            error instanceof Error
              ? error.message
              : "Failed to open link.",
          );
        }
      });
    };

    openOrResize();
    const observer = new ResizeObserver(openOrResize);
    observer.observe(host);
    const restore = () => {
      openOrResize();
    };
    window.addEventListener(PIN_WEBVIEW_RESTORE_EVENT, restore);

    const unlistenLoad = subscribePinWebviewLoad((payload) => {
      if (payload.pinId !== pinId) return;
      setLoadError(
        payload.ok ? null : (payload.message ?? "This page failed to load."),
      );
    });

    return () => {
      cancelled = true;
      observer.disconnect();
      window.removeEventListener(PIN_WEBVIEW_RESTORE_EVENT, restore);
      void unlistenLoad.then((stop) => stop());
      // Ordinary link opens destroy on unmount. Keep-alive playground pins
      // only hide so the session survives until the header X unpins.
      if (keepAlive) {
        void hidePinWebview(pinId);
      } else {
        void closePinWebview(pinId);
      }
    };
  }, [keepAlive, native, pinId, url]);

  return (
    <div
      className="-mx-4 -mb-8 flex min-h-[min(70vh,40rem)] flex-1 flex-col"
      data-testid="link-side-panel-surface"
    >
      <div
        className="relative min-h-0 min-w-0 flex-1 bg-background"
        ref={hostRef}
      >
        {native ? (
          loadError ? (
            <div
              className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center"
              data-testid="link-side-panel-load-error"
            >
              <p className="text-sm text-destructive">{loadError}</p>
              <Button
                data-testid="link-side-panel-load-error-retry"
                onClick={() => {
                  setLoadError(null);
                  window.dispatchEvent(new Event(PIN_WEBVIEW_RESTORE_EVENT));
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                Try again
              </Button>
            </div>
          ) : null
        ) : (
          <div className="flex h-full items-center justify-center px-8 text-center text-sm text-muted-foreground">
            Open this link in the Hula Buzz desktop app to view it beside the
            conversation.
          </div>
        )}
      </div>
    </div>
  );
}
