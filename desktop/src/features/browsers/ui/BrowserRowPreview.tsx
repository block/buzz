import { Globe } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

import {
  BROWSER_PREVIEW_REFRESH_MS,
  browserPreviewFaviconHost,
  captureBrowserPreview,
} from "../lib/browserPreview";

/**
 * Left-rail thumbnail for a Browsers list row.
 * Polls `playground_webview_screenshot` (viewport-native, not fullPage) about
 * every 4s. Displays with `object-contain` so the capture scales uniformly to
 * fit (letterboxed on `bg-muted/40`); never `object-cover` crop. Placeholder
 * (site favicon or Globe) while loading / when capture fails (webview closed,
 * cold session, non-macOS empty stub, etc.).
 */
export function BrowserRowPreview({
  onOpen,
  sid,
  title,
  url,
  windowLabel,
}: {
  onOpen: () => void;
  sid: string;
  title: string;
  url: string;
  windowLabel: string;
}) {
  const [src, setSrc] = React.useState<string | null>(null);
  const host = browserPreviewFaviconHost(url);

  React.useEffect(() => {
    let cancelled = false;
    setSrc(null);

    async function refresh() {
      const next = await captureBrowserPreview({ sid, windowLabel });
      if (cancelled) return;
      setSrc(next);
    }

    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, BROWSER_PREVIEW_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [sid, windowLabel]);

  return (
    <BrowserRowThumb
      host={host}
      onOpen={onOpen}
      sid={sid}
      src={src}
      title={title}
    />
  );
}

/** Presentational thumbnail shared by the desktop Browsers list and the web (bridge-backed) one. */
export function BrowserRowThumb({
  host,
  onOpen,
  sid,
  src,
  title,
}: {
  host: string | null;
  onOpen: () => void;
  sid: string;
  src: string | null;
  title: string;
}) {
  const [faviconFailed, setFaviconFailed] = React.useState(false);
  return (
    <button
      aria-label={`Open ${title}`}
      className={cn(
        "relative h-16 w-[5.5rem] shrink-0 overflow-hidden rounded-md border border-border/70 bg-muted/40",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      )}
      data-testid={`browser-row-preview-${sid}`}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onOpen();
      }}
      type="button"
    >
      {src ? (
        <img
          alt=""
          className="h-full w-full object-contain"
          data-testid={`browser-row-preview-img-${sid}`}
          draggable={false}
          src={src}
        />
      ) : (
        <span
          className="flex h-full w-full items-center justify-center text-muted-foreground"
          data-testid={`browser-row-preview-placeholder-${sid}`}
        >
          {host && !faviconFailed ? (
            <img
              alt=""
              className="h-5 w-5 opacity-70"
              draggable={false}
              key={host}
              onError={() => setFaviconFailed(true)}
              src={`https://www.google.com/s2/favicons?domain=${encodeURIComponent(host)}&sz=64`}
            />
          ) : (
            <Globe className="h-5 w-5" />
          )}
        </span>
      )}
    </button>
  );
}
