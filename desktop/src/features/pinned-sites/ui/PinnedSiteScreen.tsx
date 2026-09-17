import { isTauri } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, Compass, RefreshCw } from "lucide-react";
import * as React from "react";

import { isNativeWebviewModalParked } from "@/shared/lib/nativeWebviewModalPark";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Button } from "@/shared/ui/button";

import { usePinnedSite } from "../hooks";
import { getPinnedSiteIcon } from "../lib/icons";
import {
  getPinWebviewNavState,
  hidePinWebview,
  PIN_WEBVIEW_RESTORE_EVENT,
  pinWebviewBoundsAreUsable,
  pinWebviewGoBack,
  pinWebviewGoForward,
  pinWebviewReload,
  pollPinWebview,
  showPinWebview,
  subscribePinWebviewLoad,
  subscribePinWebviewNav,
  type PinWebviewBounds,
  type PinWebviewNavState,
} from "../lib/pinWebview";
import { PINNED_SITES_POLL_INTERVAL_MS } from "../lib/types";

export function PinnedSiteScreen({ pinId }: { pinId: string }) {
  const pin = usePinnedSite(pinId);
  const Icon = pin ? getPinnedSiteIcon(pin.icon) : Compass;

  if (!pin) {
    return (
      <div
        className="flex h-full min-h-0 flex-col"
        data-testid="pinned-site-view"
      >
        <TopChromeInsetHeader data-tauri-drag-region>
          <header className="flex h-9 items-center px-5">
            <p className="text-sm text-muted-foreground">
              Pinned site not found
            </p>
          </header>
        </TopChromeInsetHeader>
      </div>
    );
  }

  return (
    <div
      className="flex h-full min-h-0 flex-col"
      data-testid="pinned-site-view"
    >
      <PinnedSiteChrome
        icon={<Icon className="h-4 w-4" />}
        pinId={pin.id}
        startUrl={pin.url}
        title={pin.name}
      />
      <PinnedSiteSurface
        pinId={pin.id}
        pollForChanges={pin.pollForChanges}
        startUrl={pin.url}
      />
    </div>
  );
}

function PinnedSiteChrome({
  icon,
  pinId,
  startUrl,
  title,
}: {
  icon: React.ReactNode;
  pinId: string;
  startUrl: string;
  title: string;
}) {
  const [nav, setNav] = React.useState<PinWebviewNavState>({
    canGoBack: false,
    canGoForward: false,
    currentUrl: startUrl,
  });
  const [loadError, setLoadError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    void getPinWebviewNavState(pinId).then((state) => {
      if (!cancelled) setNav(state);
    });
    const unlistenNav = subscribePinWebviewNav((payload) => {
      if (payload.pinId === pinId) {
        setNav(payload);
      }
    });
    const unlistenLoad = subscribePinWebviewLoad((payload) => {
      if (payload.pinId !== pinId) return;
      setLoadError(
        payload.ok ? null : (payload.message ?? "This page failed to load."),
      );
    });
    return () => {
      cancelled = true;
      void unlistenNav.then((stop) => stop());
      void unlistenLoad.then((stop) => stop());
    };
  }, [pinId]);

  return (
    <TopChromeInsetHeader data-tauri-drag-region>
      <header className="flex h-9 min-w-0 items-center gap-2 px-3">
        <div className="flex shrink-0 items-center gap-0.5">
          <Button
            aria-label="Back"
            data-testid="pinned-site-back"
            disabled={!nav.canGoBack}
            onClick={() => {
              void pinWebviewGoBack(pinId).then(setNav);
            }}
            size="icon"
            type="button"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <Button
            aria-label="Forward"
            data-testid="pinned-site-forward"
            disabled={!nav.canGoForward}
            onClick={() => {
              void pinWebviewGoForward(pinId).then(setNav);
            }}
            size="icon"
            type="button"
            variant="ghost"
          >
            <ArrowRight className="h-4 w-4" />
          </Button>
          <Button
            aria-label="Refresh"
            data-testid="pinned-site-refresh"
            onClick={() => {
              setLoadError(null);
              void pinWebviewReload(pinId);
            }}
            size="icon"
            type="button"
            variant="ghost"
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
        <div className="flex min-w-0 items-center gap-2">
          {icon}
          <h1 className="truncate text-sm font-medium">{title}</h1>
          {loadError ? (
            <p
              className="truncate text-2xs text-destructive"
              data-testid="pinned-site-chrome-error"
              title={loadError}
            >
              {loadError}
            </p>
          ) : null}
        </div>
      </header>
    </TopChromeInsetHeader>
  );
}

function readBounds(element: HTMLElement): PinWebviewBounds {
  const rect = element.getBoundingClientRect();
  return {
    x: Math.round(rect.left),
    y: Math.round(rect.top),
    width: Math.max(1, Math.round(rect.width)),
    height: Math.max(1, Math.round(rect.height)),
  };
}

function PinnedSiteSurface({
  pinId,
  pollForChanges,
  startUrl,
}: {
  pinId: string;
  pollForChanges: boolean;
  startUrl: string;
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
      // Blocking Buzz modals park native children — do not re-show under them.
      if (isNativeWebviewModalParked()) return;
      const bounds = readBounds(hostRef.current);
      if (!pinWebviewBoundsAreUsable(bounds)) return;
      // Always show (not setBounds-only). After a hide/park, setBounds left the
      // WKWebView invisible until a second interaction. showPinWebview reuses
      // the child and calls show(); hide-epoch + generation still cancel late
      // dismiss races (#53/#54/#59).
      const id = pinId;
      void showPinWebview({
        pinId: id,
        startUrl,
        bounds,
      }).catch((error) => {
        console.error("Failed to open pinned site", error);
        if (!cancelled) {
          setLoadError(
            error instanceof Error
              ? error.message
              : "Failed to open pinned site.",
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
      void hidePinWebview(pinId);
    };
  }, [native, pinId, startUrl]);

  React.useEffect(() => {
    if (!native || !pollForChanges) return;
    const timer = window.setInterval(() => {
      void pollPinWebview(pinId, startUrl)
        .then((result) => {
          if (result.changed) {
            void pinWebviewReload(pinId);
          }
        })
        .catch((error) => {
          console.warn("Pinned site poll failed", error);
        });
    }, PINNED_SITES_POLL_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [native, pinId, pollForChanges, startUrl]);

  return (
    <div
      className="relative min-h-0 min-w-0 flex-1 bg-background"
      data-testid="pinned-site-surface"
      ref={hostRef}
    >
      {native ? (
        loadError ? (
          <PinnedSiteLoadError
            message={loadError}
            onRetry={() => {
              setLoadError(null);
              void pinWebviewReload(pinId);
            }}
          />
        ) : null
      ) : (
        <div className="flex h-full items-center justify-center px-8 text-center text-sm text-muted-foreground">
          Open this pin in the Buzz desktop app to keep the page and login.
        </div>
      )}
    </div>
  );
}

export function PinnedSiteLoadError({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      className="absolute inset-0 z-10 flex items-center justify-center bg-background px-8"
      data-testid="pinned-site-load-error"
    >
      <div className="flex max-w-md flex-col items-center text-center">
        <p className="text-sm font-medium">This pinned site did not load</p>
        <p className="mt-2 text-sm text-muted-foreground">{message}</p>
        <Button
          className="mt-4"
          data-testid="pinned-site-load-error-retry"
          onClick={onRetry}
          type="button"
          variant="secondary"
        >
          Retry
        </Button>
      </div>
    </div>
  );
}
