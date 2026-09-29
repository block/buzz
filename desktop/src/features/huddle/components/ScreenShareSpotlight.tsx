import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

type ScreenShareSpotlightProps = {
  stream: MediaStream | null;
  label?: string;
  className?: string;
  muted?: boolean;
  /**
   * Local publish is reconnecting after a LiveKit blip — keep preview visible
   * and show a spinner overlay so the user does not re-click Share.
   */
  republishing?: boolean;
};

/** Spotlight <video> for a remote (or local preview) screen share track. */
export function ScreenShareSpotlight({
  stream,
  label,
  className,
  muted = true,
  republishing = false,
}: ScreenShareSpotlightProps) {
  const videoRef = React.useRef<HTMLVideoElement | null>(null);

  React.useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    el.srcObject = stream;
    if (stream) {
      void el.play().catch(() => {
        /* autoplay may be blocked until gesture; muted helps */
      });
    }
    return () => {
      el.srcObject = null;
    };
  }, [stream]);

  if (!stream) return null;

  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-md border border-border bg-black",
        className,
      )}
    >
      <video
        ref={videoRef}
        className="h-full w-full object-contain"
        autoPlay
        playsInline
        muted={muted}
      />
      {label ? (
        <div className="pointer-events-none absolute bottom-1 left-1 rounded bg-black/60 px-1.5 py-0.5 text-[10px] text-white">
          {label}
        </div>
      ) : null}
      {republishing ? (
        <div
          aria-live="polite"
          className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/45"
          data-testid="huddle-share-republishing"
          role="status"
        >
          <Spinner
            aria-label="Reconnecting screen share"
            className="h-8 w-8 border-2 text-white"
          />
          <span className="rounded bg-black/50 px-2 py-0.5 text-[11px] text-white">
            Reconnecting…
          </span>
        </div>
      ) : null}
    </div>
  );
}
