// Full-screen lightbox for a browser's latest screenshot (web Browsers page). Fits the viewport at any width; closes on
// Close, Esc or a tap outside the panel. "New screenshot" updates the large image in place (the caller owns the command).
import { Camera, Maximize2, X } from "lucide-react";
import * as React from "react";
import { createPortal } from "react-dom";

import { Button } from "@/shared/ui/button";

import { ScreenshotFullscreen } from "./ScreenshotFullscreen";

export function BrowserScreenshotViewer({
  busy,
  disabled,
  onClose,
  onNew,
  src,
  statusText,
  statusTone,
  takenAt,
  title,
  url,
}: {
  busy: boolean;
  disabled: boolean;
  onClose: () => void;
  onNew: () => void;
  src: string | null;
  statusText: string | null;
  statusTone: "info" | "error";
  /** Unix seconds when the screenshot was taken. */
  takenAt: number | null;
  title: string;
  url: string;
}) {
  const [fullscreen, setFullscreen] = React.useState(false);
  React.useEffect(() => {
    if (fullscreen) return; // Esc belongs to the full-screen view (it goes Back to this viewer)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose, fullscreen]);

  return createPortal(
    <>
    {/* biome-ignore lint/a11y/useKeyWithClickEvents: Esc is handled on the document. */}
    <div
      aria-label={`Screenshot of ${title}`}
      aria-modal="true"
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/70 p-2 backdrop-blur-sm sm:p-6"
      data-testid="shot-viewer"
      onClick={onClose}
      role="dialog"
    >
      <div
        className="flex max-h-full w-full max-w-5xl flex-col gap-3 overflow-hidden rounded-xl border border-border/70 bg-background p-3 shadow-xl sm:p-4"
        data-testid="shot-viewer-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex min-w-0 items-start gap-2">
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium" data-testid="shot-viewer-title">{title}</p>
            <p className="truncate text-2xs text-muted-foreground" data-testid="shot-viewer-url">{url}</p>
            <p className="text-2xs text-muted-foreground" data-testid="shot-viewer-time">
              {takenAt ? `Screenshot taken ${new Date(takenAt * 1000).toLocaleTimeString()}` : "No screenshot yet"}
            </p>
          </div>
          <Button aria-label="Full screen" data-testid="shot-viewer-fullscreen" disabled={!src} onClick={() => setFullscreen(true)} size="xs" type="button" variant="secondary">
            <Maximize2 className="mr-1 h-3 w-3" />
            <span className="hidden sm:inline">Full screen</span>
          </Button>
          <Button data-testid="shot-viewer-new" disabled={disabled || busy} onClick={onNew} size="xs" type="button" variant="secondary">
            <Camera className="mr-1 h-3 w-3" />
            New screenshot
          </Button>
          <Button aria-label="Close" data-testid="shot-viewer-close" onClick={onClose} size="icon-xs" type="button" variant="outline">
            <X />
          </Button>
        </div>
        {statusText ? (
          <p
            aria-live="polite"
            className={`text-2xs ${statusTone === "error" ? "text-destructive" : "text-muted-foreground"}`}
            data-testid="shot-viewer-status"
          >
            {statusText}
          </p>
        ) : null}
        <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto rounded-md bg-muted/40">
          {src ? (
            <img alt={`Screenshot of ${title}`} className="max-h-[70vh] w-full object-contain" data-testid="shot-viewer-img" draggable={false} src={src} />
          ) : (
            <p className="px-6 py-16 text-sm text-muted-foreground">Press New screenshot to capture this browser.</p>
          )}
        </div>
      </div>
    </div>
    {fullscreen && src ? (
        <ScreenshotFullscreen
          busy={busy}
          disabled={disabled}
          onBack={() => setFullscreen(false)}
          onNew={onNew}
          src={src}
          statusText={statusText}
          takenAt={takenAt}
          title={title}
        />
      ) : null}
    </>,
    document.body,
  );
}
