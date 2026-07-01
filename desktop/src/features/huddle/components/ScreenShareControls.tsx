import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { MousePointer2, ScreenShare, ScreenShareOff, X } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { invokeRawBinary } from "../lib/tauriRawBinary";

type HuddleScreenFrameEvent = {
  pubkey: string;
  mime_type: string;
  data_base64: string;
};

type ScreenCursorControl = {
  type: "cursor";
  x: number;
  y: number;
  visible: boolean;
};

type ScreenShareStateControl = {
  type: "share_state";
  active: boolean;
};

type ScreenShareControl = ScreenCursorControl | ScreenShareStateControl;

type HuddleScreenControlEvent = {
  pubkey: string;
  control: ScreenShareControl | Record<string, unknown>;
};

type RemoteScreenShare = {
  pubkey: string;
  src: string | null;
  lastFrameAt: number;
  stale: boolean;
  cursor: {
    x: number;
    y: number;
    visible: boolean;
  } | null;
};

type ScreenShareControlsProps = {
  available: boolean;
  localPubkey: string | null;
};

type DisplayMediaVideoConstraints = MediaTrackConstraints & {
  cursor?: "always" | "motion" | "never";
  displaySurface?: "browser" | "monitor" | "window";
};

const SCREEN_FRAME_INTERVAL_MS = 250;
const SCREEN_FRAME_MAX_WIDTH = 1280;
const SCREEN_FRAME_MAX_HEIGHT = 800;
const SCREEN_FRAME_JPEG_QUALITY = 0.56;
const SCREEN_FRAME_MAX_BYTES = 500 * 1024;
const CURSOR_SEND_INTERVAL_MS = 80;
const REMOTE_STALE_AFTER_MS = 6_000;

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function isCursorControl(control: unknown): control is ScreenCursorControl {
  if (!control || typeof control !== "object") return false;
  const c = control as ScreenCursorControl;
  return (
    c.type === "cursor" &&
    typeof c.x === "number" &&
    typeof c.y === "number" &&
    typeof c.visible === "boolean"
  );
}

function isShareStateControl(
  control: unknown,
): control is ScreenShareStateControl {
  if (!control || typeof control !== "object") return false;
  const c = control as ScreenShareStateControl;
  return c.type === "share_state" && typeof c.active === "boolean";
}

function screenDisplayName(pubkey: string | null): string {
  return pubkey ? `Participant ${pubkey.slice(0, 8)}` : "Participant";
}

function canvasToJpegBlob(canvas: HTMLCanvasElement): Promise<Blob | null> {
  return new Promise((resolve) => {
    canvas.toBlob(resolve, "image/jpeg", SCREEN_FRAME_JPEG_QUALITY);
  });
}

function scaledFrameSize(width: number, height: number) {
  if (width <= 0 || height <= 0) return { width: 0, height: 0 };
  const scale = Math.min(
    1,
    SCREEN_FRAME_MAX_WIDTH / width,
    SCREEN_FRAME_MAX_HEIGHT / height,
  );
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

async function requestDisplayStream(): Promise<MediaStream> {
  const video: DisplayMediaVideoConstraints = {
    cursor: "always",
    frameRate: { ideal: 4, max: 5 },
    height: { ideal: SCREEN_FRAME_MAX_HEIGHT },
    width: { ideal: SCREEN_FRAME_MAX_WIDTH },
  };
  return navigator.mediaDevices.getDisplayMedia({
    audio: false,
    video,
  } as DisplayMediaStreamOptions);
}

export function ScreenShareControls({
  available,
  localPubkey,
}: ScreenShareControlsProps) {
  const [localStream, setLocalStream] = React.useState<MediaStream | null>(
    null,
  );
  const [isStarting, setIsStarting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [remoteShare, setRemoteShare] =
    React.useState<RemoteScreenShare | null>(null);
  const [localCursor, setLocalCursor] = React.useState<{
    x: number;
    y: number;
    visible: boolean;
  } | null>(null);

  const localVideoRef = React.useRef<HTMLVideoElement | null>(null);
  const captureVideoRef = React.useRef<HTMLVideoElement | null>(null);
  const captureCanvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const frameTimerRef = React.useRef<number | null>(null);
  const isSharingRef = React.useRef(false);
  const lastCursorSentAtRef = React.useRef(0);

  const pushControl = React.useCallback(
    (control: ScreenShareControl): Promise<unknown> =>
      invoke("push_huddle_screen_control", { control }),
    [],
  );

  const clearFrameTimer = React.useCallback(() => {
    if (frameTimerRef.current !== null) {
      window.clearTimeout(frameTimerRef.current);
      frameTimerRef.current = null;
    }
  }, []);

  const stopLocalShare = React.useCallback(
    (notify = true) => {
      clearFrameTimer();
      isSharingRef.current = false;
      captureVideoRef.current?.pause();
      captureVideoRef.current = null;
      captureCanvasRef.current = null;
      setLocalCursor(null);
      setLocalStream((stream) => {
        stream?.getTracks().forEach((track) => {
          track.stop();
        });
        return null;
      });
      if (notify) {
        void pushControl({ type: "share_state", active: false }).catch(() => {
          /* best-effort */
        });
      }
    },
    [clearFrameTimer, pushControl],
  );

  React.useEffect(() => {
    return () => stopLocalShare();
  }, [stopLocalShare]);

  React.useEffect(() => {
    const video = localVideoRef.current;
    if (!video) return;
    video.srcObject = localStream;
    if (localStream) {
      void video.play().catch(() => {
        /* preview can fail if the element is mid-unmount */
      });
    }
    return () => {
      video.srcObject = null;
    };
  }, [localStream]);

  React.useEffect(() => {
    let disposed = false;
    let unlistenFrame: (() => void) | null = null;
    let unlistenControl: (() => void) | null = null;

    void listen<HuddleScreenFrameEvent>("huddle-screen-frame", (event) => {
      if (disposed || event.payload.pubkey === localPubkey) return;
      const {
        pubkey,
        mime_type: mimeType,
        data_base64: dataBase64,
      } = event.payload;
      setRemoteShare((prev) => ({
        pubkey,
        src: `data:${mimeType};base64,${dataBase64}`,
        lastFrameAt: Date.now(),
        stale: false,
        cursor: prev?.pubkey === pubkey ? prev.cursor : null,
      }));
    }).then((fn) => {
      if (disposed) fn();
      else unlistenFrame = fn;
    });

    void listen<HuddleScreenControlEvent>("huddle-screen-control", (event) => {
      if (disposed || event.payload.pubkey === localPubkey) return;
      const { pubkey, control } = event.payload;
      if (isShareStateControl(control)) {
        setRemoteShare((prev) => {
          if (!control.active && prev?.pubkey === pubkey) return null;
          if (control.active && !prev) {
            return {
              pubkey,
              src: null,
              lastFrameAt: Date.now(),
              stale: false,
              cursor: null,
            };
          }
          return prev;
        });
        return;
      }

      if (!isCursorControl(control)) return;
      setRemoteShare((prev) => {
        if (!prev || prev.pubkey !== pubkey) return prev;
        return {
          ...prev,
          cursor: {
            x: clamp01(control.x),
            y: clamp01(control.y),
            visible: control.visible,
          },
        };
      });
    }).then((fn) => {
      if (disposed) fn();
      else unlistenControl = fn;
    });

    return () => {
      disposed = true;
      unlistenFrame?.();
      unlistenControl?.();
    };
  }, [localPubkey]);

  React.useEffect(() => {
    const id = window.setInterval(() => {
      const now = Date.now();
      setRemoteShare((prev) => {
        if (!prev) return prev;
        if (now - prev.lastFrameAt < REMOTE_STALE_AFTER_MS) return prev;
        return { ...prev, stale: true };
      });
    }, 1_000);
    return () => window.clearInterval(id);
  }, []);

  const scheduleFrame = React.useCallback(() => {
    clearFrameTimer();
    frameTimerRef.current = window.setTimeout(async () => {
      if (!isSharingRef.current) return;

      const video = captureVideoRef.current;
      if (!video || video.videoWidth === 0 || video.videoHeight === 0) {
        scheduleFrame();
        return;
      }

      const { width, height } = scaledFrameSize(
        video.videoWidth,
        video.videoHeight,
      );
      if (!width || !height) {
        scheduleFrame();
        return;
      }

      const canvas =
        captureCanvasRef.current ?? document.createElement("canvas");
      captureCanvasRef.current = canvas;
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      const ctx = canvas.getContext("2d", { alpha: false });
      if (!ctx) {
        scheduleFrame();
        return;
      }

      try {
        ctx.drawImage(video, 0, 0, width, height);
      } catch (drawError) {
        console.warn("[huddle] Screen frame draw skipped:", drawError);
        scheduleFrame();
        return;
      }
      const blob = await canvasToJpegBlob(canvas);
      if (blob && blob.size <= SCREEN_FRAME_MAX_BYTES) {
        const bytes = new Uint8Array(await blob.arrayBuffer());
        void invokeRawBinary("push_huddle_screen_frame", bytes).catch((err) => {
          console.error("[huddle] Failed to send screen frame:", err);
        });
      }

      scheduleFrame();
    }, SCREEN_FRAME_INTERVAL_MS);
  }, [clearFrameTimer]);

  const startLocalShare = React.useCallback(async () => {
    if (isStarting || localStream) return;
    if (!available) {
      setError("Screen sharing needs an updated huddle relay.");
      return;
    }
    if (!navigator.mediaDevices?.getDisplayMedia) {
      setError("Screen sharing is not available in this app window.");
      return;
    }

    setError(null);
    setIsStarting(true);
    try {
      const stream = await requestDisplayStream();
      const [track] = stream.getVideoTracks();
      if (!track) {
        stream.getTracks().forEach((t) => {
          t.stop();
        });
        throw new Error("No screen video track was selected.");
      }

      const captureVideo = document.createElement("video");
      captureVideo.autoplay = true;
      captureVideo.muted = true;
      captureVideo.playsInline = true;
      captureVideo.srcObject = stream;
      captureVideoRef.current = captureVideo;

      void captureVideo.play().catch((playError) => {
        console.warn(
          "[huddle] Screen capture preview playback was delayed:",
          playError,
        );
      });

      track.addEventListener("ended", () => {
        setError("Screen sharing stopped.");
        stopLocalShare();
      });
      isSharingRef.current = true;
      setLocalStream(stream);
      void pushControl({ type: "share_state", active: true }).catch(() => {
        /* best-effort */
      });
      scheduleFrame();
    } catch (err) {
      stopLocalShare(false);
      const message = err instanceof Error ? err.message : String(err);
      setError(message || "Screen sharing failed.");
    } finally {
      setIsStarting(false);
    }
  }, [
    available,
    isStarting,
    localStream,
    pushControl,
    scheduleFrame,
    stopLocalShare,
  ]);

  const handleCursorMove = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (!localStream) return;
      const rect = event.currentTarget.getBoundingClientRect();
      const cursor = {
        x: clamp01((event.clientX - rect.left) / rect.width),
        y: clamp01((event.clientY - rect.top) / rect.height),
        visible: true,
      };
      setLocalCursor(cursor);

      const now = Date.now();
      if (now - lastCursorSentAtRef.current < CURSOR_SEND_INTERVAL_MS) return;
      lastCursorSentAtRef.current = now;
      void pushControl({ type: "cursor", ...cursor }).catch(() => {
        /* best-effort */
      });
    },
    [localStream, pushControl],
  );

  const handleCursorLeave = React.useCallback(() => {
    if (!localStream) return;
    const cursor = { x: localCursor?.x ?? 0.5, y: localCursor?.y ?? 0.5 };
    setLocalCursor((prev) => (prev ? { ...prev, visible: false } : null));
    void pushControl({ type: "cursor", ...cursor, visible: false }).catch(
      () => {
        /* best-effort */
      },
    );
  }, [localCursor, localStream, pushControl]);

  const showingLocal = Boolean(localStream);
  const showPanel = showingLocal || Boolean(remoteShare);
  const cursor = showingLocal ? localCursor : remoteShare?.cursor;
  const cursorVisible = Boolean(cursor?.visible);
  const title = showingLocal
    ? "You are sharing"
    : remoteShare
      ? `${screenDisplayName(remoteShare.pubkey)} is sharing`
      : "Screen share";

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label={localStream ? "Stop sharing screen" : "Share screen"}
            aria-disabled={isStarting || !available}
            aria-pressed={Boolean(localStream)}
            className={cn(
              "buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md",
              localStream && "text-foreground",
              !available && "opacity-60",
            )}
            disabled={isStarting}
            onClick={() => {
              if (localStream) {
                stopLocalShare();
              } else {
                void startLocalShare();
              }
            }}
            size="icon"
            type="button"
            variant="secondary"
          >
            {localStream ? (
              <ScreenShareOff className="h-4 w-4" />
            ) : (
              <ScreenShare className="h-4 w-4" />
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent className="buzz-huddle-tooltip" side="top">
          {error ??
            (!available
              ? "Screen sharing needs an updated huddle relay"
              : localStream
                ? "Stop sharing screen"
                : "Share screen")}
        </TooltipContent>
      </Tooltip>

      {showPanel && (
        <div className="pointer-events-none fixed inset-x-4 bottom-[calc(var(--buzz-huddle-drawer-height)+1rem)] top-14 z-30 flex items-end justify-center">
          <section
            aria-label={title}
            className="buzz-huddle-popover pointer-events-auto flex max-h-full w-[min(72rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-lg border bg-background text-foreground shadow-2xl"
          >
            <header className="flex h-11 shrink-0 items-center justify-between gap-3 border-border/60 border-b px-3">
              <div className="min-w-0">
                <p className="truncate text-xs font-medium">{title}</p>
                {remoteShare?.stale && !showingLocal && (
                  <p className="text-2xs text-muted-foreground">
                    Reconnecting...
                  </p>
                )}
              </div>
              {showingLocal && (
                <Button
                  aria-label="Stop sharing screen"
                  className="h-7 w-7 border-border/60 bg-muted text-foreground hover:bg-accent"
                  onClick={() => stopLocalShare()}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              )}
            </header>

            <div
              className="relative flex min-h-0 flex-1 items-center justify-center bg-background"
              onPointerLeave={handleCursorLeave}
              onPointerMove={handleCursorMove}
            >
              <div className="relative aspect-video max-h-[calc(100vh-10rem)] w-full max-w-full overflow-hidden bg-background">
                {showingLocal ? (
                  <video
                    ref={localVideoRef}
                    aria-label="Your shared screen preview"
                    className="h-full w-full object-contain"
                    muted
                    playsInline
                  />
                ) : remoteShare?.src ? (
                  <img
                    alt={`${screenDisplayName(remoteShare.pubkey)} shared screen`}
                    className="h-full w-full object-contain"
                    draggable={false}
                    src={remoteShare.src}
                  />
                ) : (
                  <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                    Waiting for screen...
                  </div>
                )}

                {cursorVisible && cursor && (
                  <MousePointer2
                    aria-hidden="true"
                    className="pointer-events-none absolute h-5 w-5 -translate-y-px text-foreground drop-shadow-md"
                    style={{
                      left: `${clamp01(cursor.x) * 100}%`,
                      top: `${clamp01(cursor.y) * 100}%`,
                    }}
                  />
                )}
              </div>
            </div>
          </section>
        </div>
      )}

      <span aria-live="polite" className="sr-only">
        {error ? `Screen share error: ${error}` : ""}
      </span>
    </>
  );
}
