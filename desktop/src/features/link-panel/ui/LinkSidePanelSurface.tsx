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
import {
  PLAYGROUND_DEVICES,
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  playgroundDeviceViewport,
  scalePlaygroundDeviceViewport,
  type PlaygroundDeviceId,
  type PlaygroundDeviceScalePercent,
} from "@/features/playground/lib/devices";
import {
  PLAYGROUND_RESIZE_HANDLE_CLASS,
  PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS,
} from "@/features/playground/lib/overlayLayout";
import { DEFAULT_RESPONSIVE_VIEWPORT } from "@/features/playground/lib/types";
import { DeviceBezel } from "@/features/playground/ui/DeviceBezel";
import { DeviceMuseumToolbar } from "@/features/playground/ui/DeviceMuseumToolbar";
import { cn } from "@/shared/lib/cn";
import { isNativeWebviewModalParked } from "@/shared/lib/nativeWebviewModalPark";
import { Button } from "@/shared/ui/button";

import {
  LINK_SIDE_PANEL_PIN_ID,
  type LinkSidePanelViewportMode,
} from "../lib/linkSidePanelStore";

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
 * Desktop fills the panel; Responsive/Mobile reuse playground stage chrome
 * (W×H + drag handles, device museum + bezel).
 */
export function LinkSidePanelSurface({
  keepAlive = false,
  pinId = LINK_SIDE_PANEL_PIN_ID,
  url,
  viewportMode = "desktop",
}: {
  keepAlive?: boolean;
  pinId?: string;
  url: string;
  viewportMode?: LinkSidePanelViewportMode;
}) {
  if (viewportMode === "mobile") {
    return (
      <MobileStage keepAlive={keepAlive} pinId={pinId} url={url} />
    );
  }
  if (viewportMode === "responsive") {
    return (
      <ResponsiveStage keepAlive={keepAlive} pinId={pinId} url={url} />
    );
  }
  return <DesktopStage keepAlive={keepAlive} pinId={pinId} url={url} />;
}

function DesktopStage({
  keepAlive,
  pinId,
  url,
}: {
  keepAlive: boolean;
  pinId: string;
  url: string;
}) {
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col"
      data-testid="link-side-panel-surface"
      data-viewport-mode="desktop"
    >
      <PinStageHost
        hostRef={hostRef}
        keepAlive={keepAlive}
        pinId={pinId}
        url={url}
      />
    </div>
  );
}

function ResponsiveStage({
  keepAlive,
  pinId,
  url,
}: {
  keepAlive: boolean;
  pinId: string;
  url: string;
}) {
  const [width, setWidth] = React.useState(DEFAULT_RESPONSIVE_VIEWPORT.width);
  const [height, setHeight] = React.useState(
    DEFAULT_RESPONSIVE_VIEWPORT.height,
  );
  const hostRef = React.useRef<HTMLDivElement | null>(null);

  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-testid="link-side-panel-surface"
      data-viewport-mode="responsive"
    >
      <div className="flex flex-wrap items-center gap-2 px-3 pt-2">
        <label className="flex items-center gap-1 text-2xs text-muted-foreground">
          W
          <input
            className="w-16 rounded-md border border-border bg-background px-1 py-0.5 text-2xs text-foreground"
            data-testid="link-side-panel-responsive-width"
            min={320}
            onChange={(event) =>
              setWidth(Math.max(320, Number(event.target.value) || 320))
            }
            type="number"
            value={width}
          />
        </label>
        <span className="text-2xs text-muted-foreground">×</span>
        <label className="flex items-center gap-1 text-2xs text-muted-foreground">
          H
          <input
            className="w-16 rounded-md border border-border bg-background px-1 py-0.5 text-2xs text-foreground"
            data-testid="link-side-panel-responsive-height"
            min={320}
            onChange={(event) =>
              setHeight(Math.max(320, Number(event.target.value) || 320))
            }
            type="number"
            value={height}
          />
        </label>
      </div>
      <div className="relative min-h-0 min-w-0 flex-1 overflow-auto p-3">
        <div
          className={cn(
            "relative inline-block",
            PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS,
          )}
          data-testid="link-side-panel-responsive-frame"
        >
          <div
            className="overflow-hidden rounded-md border border-border"
            data-testid="link-side-panel-responsive-page"
            style={{ width, height }}
          >
            <PinStageHost
              hostRef={hostRef}
              keepAlive={keepAlive}
              pinId={pinId}
              url={url}
              viewport={{ width, height }}
            />
          </div>
          <StageResizeHandle
            axis="x"
            onResize={(next) => setWidth(Math.max(320, next.width))}
            size={{ width, height }}
            testId="link-side-panel-resize"
          />
          <StageResizeHandle
            axis="y"
            onResize={(next) => setHeight(Math.max(320, next.height))}
            size={{ width, height }}
            testId="link-side-panel-resize-y"
          />
          <StageResizeHandle
            axis="xy"
            onResize={(next) => {
              setWidth(Math.max(320, next.width));
              setHeight(Math.max(320, next.height));
            }}
            size={{ width, height }}
            testId="link-side-panel-resize-xy"
          />
        </div>
      </div>
    </div>
  );
}

function MobileStage({
  keepAlive,
  pinId,
  url,
}: {
  keepAlive: boolean;
  pinId: string;
  url: string;
}) {
  const [deviceId, setDeviceId] =
    React.useState<PlaygroundDeviceId>("iphone-16");
  const [orientation, setOrientation] = React.useState<
    "portrait" | "landscape"
  >("portrait");
  const [scalePercent, setScalePercent] =
    React.useState<PlaygroundDeviceScalePercent>(
      PLAYGROUND_DEVICE_SCALE_DEFAULT,
    );
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  const device = PLAYGROUND_DEVICES.find((item) => item.id === deviceId);
  const viewport = scalePlaygroundDeviceViewport(
    device
      ? playgroundDeviceViewport(device, orientation)
      : { width: 393, height: 852 },
    scalePercent,
  );

  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-testid="link-side-panel-surface"
      data-viewport-mode="mobile"
    >
      <div className="pt-2">
        <DeviceMuseumToolbar
          deviceId={deviceId}
          onDeviceIdChange={setDeviceId}
          onOrientationToggle={() =>
            setOrientation((value) =>
              value === "portrait" ? "landscape" : "portrait",
            )
          }
          onScalePercentChange={setScalePercent}
          orientation={orientation}
          scalePercent={scalePercent}
          testIdPrefix="link-side-panel"
        />
      </div>
      <div
        className="flex min-h-0 flex-1 items-start justify-center overflow-auto bg-white p-3"
        data-testid="link-side-panel-mobile-backdrop"
      >
        {device ? (
          <DeviceBezel
            device={device}
            orientation={orientation}
            scalePercent={scalePercent}
          >
            <PinStageHost
              hostRef={hostRef}
              keepAlive={keepAlive}
              pinId={pinId}
              url={url}
              viewport={viewport}
            />
          </DeviceBezel>
        ) : null}
      </div>
    </div>
  );
}

function StageResizeHandle({
  axis,
  onResize,
  size,
  testId,
}: {
  axis: "x" | "y" | "xy";
  onResize: (next: { width: number; height: number }) => void;
  size: { width: number; height: number };
  testId: string;
}) {
  const dragging = React.useRef(false);
  const origin = React.useRef({
    x: 0,
    y: 0,
    width: 0,
    height: 0,
  });

  React.useEffect(() => {
    const onMove = (event: PointerEvent) => {
      if (!dragging.current) return;
      const width =
        axis === "y"
          ? origin.current.width
          : origin.current.width + (event.clientX - origin.current.x);
      const height =
        axis === "x"
          ? origin.current.height
          : origin.current.height + (event.clientY - origin.current.y);
      onResize({ width, height });
    };
    const onUp = () => {
      dragging.current = false;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [axis, onResize]);

  const label =
    axis === "x"
      ? "Resize width"
      : axis === "y"
        ? "Resize height"
        : "Resize width and height";

  return (
    <button
      aria-label={label}
      className={PLAYGROUND_RESIZE_HANDLE_CLASS[axis]}
      data-testid={testId}
      onPointerDown={(event) => {
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.setPointerCapture?.(event.pointerId);
        origin.current = {
          x: event.clientX,
          y: event.clientY,
          width: size.width,
          height: size.height,
        };
        dragging.current = true;
      }}
      type="button"
    />
  );
}

function PinStageHost({
  hostRef,
  keepAlive,
  pinId,
  url,
  viewport,
}: {
  hostRef: React.RefObject<HTMLDivElement | null>;
  keepAlive: boolean;
  pinId: string;
  url: string;
  viewport?: { width: number; height: number };
}) {
  const native = isTauri() || import.meta.env.MODE === "e2e";
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const viewportWidth = viewport?.width;
  const viewportHeight = viewport?.height;

  React.useEffect(() => {
    if (!native) return;
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;

    const openOrResize = () => {
      if (cancelled || !hostRef.current) return;
      if (isNativeWebviewModalParked()) return;
      const measured = readBounds(hostRef.current);
      const bounds =
        viewportWidth != null && viewportHeight != null
          ? {
              ...measured,
              width: Math.max(1, Math.round(viewportWidth)),
              height: Math.max(1, Math.round(viewportHeight)),
            }
          : measured;
      if (!pinWebviewBoundsAreUsable(bounds)) return;
      void showPinWebview({
        pinId,
        startUrl: url,
        bounds,
      }).catch((error) => {
        console.error("Failed to open link side panel", error);
        if (!cancelled) {
          setLoadError(
            error instanceof Error ? error.message : "Failed to open link.",
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
      if (keepAlive) {
        void hidePinWebview(pinId);
      } else {
        void closePinWebview(pinId);
      }
    };
  }, [
    keepAlive,
    hostRef,
    native,
    pinId,
    url,
    viewportWidth,
    viewportHeight,
  ]);

  return (
    <div
      className={cn("relative h-full min-h-0 w-full bg-background")}
      data-testid="link-side-panel-webview-host"
      data-viewport-width={viewport?.width}
      data-viewport-height={viewport?.height}
      ref={hostRef}
      style={
        viewport
          ? { width: viewport.width, height: viewport.height }
          : undefined
      }
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
  );
}

