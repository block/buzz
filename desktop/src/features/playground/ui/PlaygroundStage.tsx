import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { isNativeWebviewModalParked } from "@/shared/lib/nativeWebviewModalPark";

import {
  afterPlaygroundLayout,
  playgroundStageBoundsSyncTargets,
  playgroundStageChromeElement,
  playgroundStageMeasureElement,
  readPlaygroundStageBounds,
} from "../lib/deviceBezel";
import {
  PLAYGROUND_DEVICES,
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  playgroundDeviceViewport,
  playgroundUserAgent,
  scalePlaygroundDeviceViewport,
  type PlaygroundDeviceId,
  type PlaygroundDeviceScalePercent,
} from "../lib/devices";
import {
  PLAYGROUND_RESIZE_HANDLE_CLASS,
  PLAYGROUND_RESIZE_HANDLE_GUTTER_CLASS,
} from "../lib/overlayLayout";
import { DEFAULT_RESPONSIVE_VIEWPORT } from "../lib/types";
import { PLAYGROUND_DOM_PROBE_SCRIPT } from "../lib/updates";
import {
  evalPlaygroundWebview,
  hidePlaygroundWebview,
  PLAYGROUND_WEBVIEW_RESTORE_EVENT,
  playgroundWebviewBoundsAreUsable,
  showPlaygroundWebview,
} from "../lib/webview";
import type { PlaygroundSession } from "../lib/sessions";
import { DeviceBezel } from "./DeviceBezel";
import { DeviceMuseumToolbar } from "./DeviceMuseumToolbar";

export type PlaygroundChromeMode = "desktop" | "responsive" | "mobile";

export function PlaygroundStage({
  layoutKey = "window:0",
  mode,
  session,
}: {
  layoutKey?: string;
  mode: PlaygroundChromeMode;
  session: PlaygroundSession;
}) {
  if (mode === "mobile") {
    return <MobileDeviceMuseum layoutKey={layoutKey} session={session} />;
  }
  if (mode === "responsive") {
    return <ResponsiveStage layoutKey={layoutKey} session={session} />;
  }
  return <DesktopStage layoutKey={layoutKey} session={session} />;
}

function DesktopStage({
  layoutKey,
  session,
}: {
  layoutKey: string;
  session: PlaygroundSession;
}) {
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col"
      data-testid="playground-desktop-stage"
    >
      <NativeStageHost
        hostRef={hostRef}
        layoutKey={layoutKey}
        session={session}
        userAgent={playgroundUserAgent("desktop")}
      />
    </div>
  );
}

function ResponsiveStage({
  layoutKey,
  session,
}: {
  layoutKey: string;
  session: PlaygroundSession;
}) {
  const [width, setWidth] = React.useState(DEFAULT_RESPONSIVE_VIEWPORT.width);
  const [height, setHeight] = React.useState(
    DEFAULT_RESPONSIVE_VIEWPORT.height,
  );
  const hostRef = React.useRef<HTMLDivElement | null>(null);

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-testid="playground-responsive-stage"
    >
      <div className="flex flex-wrap items-center gap-2 px-3">
        <label className="flex items-center gap-1 text-2xs text-muted-foreground">
          W
          <input
            className="w-16 rounded-md border border-border bg-background px-1 py-0.5 text-2xs text-foreground"
            data-testid="playground-responsive-width"
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
            data-testid="playground-responsive-height"
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
          data-testid="playground-responsive-frame"
        >
          <div
            className="overflow-hidden rounded-md border border-border"
            data-testid="playground-responsive-page"
            style={{ width, height }}
          >
            <NativeStageHost
              hostRef={hostRef}
              layoutKey={layoutKey}
              session={session}
              userAgent={playgroundUserAgent("responsive")}
              viewport={{ width, height }}
            />
          </div>
          <StageResizeHandle
            axis="x"
            onResize={(next) => setWidth(Math.max(320, next.width))}
            size={{ width, height }}
            testId="playground-stage-resize"
          />
          <StageResizeHandle
            axis="y"
            onResize={(next) => setHeight(Math.max(320, next.height))}
            size={{ width, height }}
            testId="playground-stage-resize-y"
          />
          <StageResizeHandle
            axis="xy"
            onResize={(next) => {
              setWidth(Math.max(320, next.width));
              setHeight(Math.max(320, next.height));
            }}
            size={{ width, height }}
            testId="playground-stage-resize-xy"
          />
        </div>
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

function MobileDeviceMuseum({
  layoutKey,
  session,
}: {
  layoutKey: string;
  session: PlaygroundSession;
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
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-testid="playground-mobile-stage"
    >
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
        testIdPrefix="playground"
      />
      <div
        className="flex min-h-0 flex-1 items-start justify-center overflow-auto bg-white p-3"
        data-testid="playground-mobile-backdrop"
      >
        {device ? (
          <DeviceBezel
            device={device}
            orientation={orientation}
            scalePercent={scalePercent}
          >
            <NativeStageHost
              hostRef={hostRef}
              layoutKey={layoutKey}
              session={session}
              userAgent={playgroundUserAgent("mobile", device)}
              viewport={viewport}
            />
          </DeviceBezel>
        ) : null}
      </div>
    </div>
  );
}

function NativeStageHost({
  hostRef,
  layoutKey,
  session,
  userAgent,
  viewport,
}: {
  hostRef: React.RefObject<HTMLDivElement | null>;
  layoutKey: string;
  session: PlaygroundSession;
  userAgent: string;
  viewport?: { width: number; height: number };
}) {
  const viewportWidth = viewport?.width;
  const viewportHeight = viewport?.height;

  React.useEffect(() => {
    const sid = session.sid;
    return () => {
      // React unmount (navigate away, leave split, deactivate embed) must
      // hide the window-scoped native child. Overlay teardown alone is not
      // enough — WKWebView stays painted after the host DOM is gone.
      // Kept off the layoutKey effect so fullscreen/dock resyncs do not hide.
      void hidePlaygroundWebview(sid);
    };
  }, [session.sid]);

  React.useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
    let opened = false;
    let layoutRetry = 0;
    let cancelScheduled: (() => void) | null = null;
    let settleTimer = 0;
    // layoutKey is the position-change signal: ResizeObserver ignores
    // moves that keep the same size (fullscreen toggle, inspect restore).
    void layoutKey;

    // Chrome is shrink-0 above the stage. Look it up before the first sync so
    // the native rect never covers the mode row. Mobile museum host size is a
    // fixed CSS viewport — window resize only re-centers the bezel via flex.
    // Observe backdrop/frame/stage/overlay (+ scroll) and defer measurement
    // until after layout so the border stays aligned with the preview.
    const chrome = playgroundStageChromeElement(host);
    const syncTargets = playgroundStageBoundsSyncTargets(host);

    const hostHasLayout = (el: HTMLElement) => {
      const rect = el.getBoundingClientRect();
      return rect.width >= 1 && rect.height >= 1;
    };

    const syncNow = () => {
      if (cancelled || !hostRef.current) return;
      // Blocking Buzz modals park native children — do not re-show under them.
      if (isNativeWebviewModalParked()) return;
      // Detached/windowed opens often mount at 0×0 before flex lays out.
      // Viewport fallback used to make bounds "usable" at stale x/y=0, which
      // painted a blank hole until resize or desktop↔mobile remount.
      const measureEl = playgroundStageMeasureElement(hostRef.current);
      if (!hostHasLayout(measureEl)) return;
      const bounds = readPlaygroundStageBounds(
        measureEl,
        viewportWidth != null && viewportHeight != null
          ? { width: viewportWidth, height: viewportHeight }
          : undefined,
        chrome,
      );
      if (!playgroundWebviewBoundsAreUsable(bounds)) return;
      // Always show (not setBounds-only). A late keeper `visible: false` or
      // modal park can hide after the first open; setBounds alone left a blank
      // stage until Inspect re-applied bounds. Visible show also bumps the
      // generation so in-flight keeper hides restore via the event below.
      const sid = session.sid;
      const firstOpen = !opened;
      opened = true;
      void showPlaygroundWebview({
        sid,
        url: session.url,
        bounds,
        visible: true,
        userAgent,
      }).then(() => {
        if (cancelled) {
          void hidePlaygroundWebview(sid);
          return;
        }
        if (firstOpen) {
          return evalPlaygroundWebview(sid, PLAYGROUND_DOM_PROBE_SCRIPT);
        }
      });
    };

    const scheduleSync = (settle = false) => {
      cancelScheduled?.();
      cancelScheduled = afterPlaygroundLayout(() => {
        cancelScheduled = null;
        syncNow();
      });
      if (!settle || typeof window.setTimeout !== "function") return;
      window.clearTimeout(settleTimer);
      // Flex justify-center can keep moving for a few frames after resize.
      // Re-measure from the screen hole until layout settles so the native
      // WKWebView cannot lag a half-phone to the right of the CSS bezel.
      let pulses = 0;
      const pulse = () => {
        syncNow();
        pulses += 1;
        if (cancelled || pulses >= 10) return;
        settleTimer = window.setTimeout(pulse, 32);
      };
      settleTimer = window.setTimeout(pulse, 32);
    };

    syncNow();
    const observer = new ResizeObserver(() => scheduleSync(true));
    for (const el of syncTargets.observe) {
      observer.observe(el);
    }
    const onWindowResize = () => scheduleSync(true);
    window.addEventListener("resize", onWindowResize);
    for (const el of syncTargets.scroll) {
      el.addEventListener("scroll", onWindowResize, { passive: true });
    }
    // Modal park (and pin restore) hides playground children without
    // unmounting this host — re-open so the stage is visible again.
    const restore = () => {
      opened = false;
      scheduleSync(true);
    };
    window.addEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, restore);
    const visualViewport = window.visualViewport;
    visualViewport?.addEventListener("resize", onWindowResize);
    visualViewport?.addEventListener("scroll", onWindowResize);
    // Retry briefly while the pop-out flex tree settles — RO may not fire if
    // the host jumps from 0×0 to sized in the same commit path WebKit skips.
    // Keep a few frames after first open so justify-center can finish moving
    // the bezel before we freeze native bounds.
    let layoutRaf = 0;
    const retryLayout = () => {
      if (cancelled || layoutRetry >= 45) return;
      layoutRetry += 1;
      syncNow();
      if (!opened || layoutRetry < 12) {
        layoutRaf = window.requestAnimationFrame(retryLayout);
      }
    };
    if (typeof window.requestAnimationFrame === "function") {
      layoutRaf = window.requestAnimationFrame(retryLayout);
    }
    return () => {
      cancelled = true;
      cancelScheduled?.();
      if (typeof window.clearTimeout === "function") {
        window.clearTimeout(settleTimer);
      }
      observer.disconnect();
      window.removeEventListener("resize", onWindowResize);
      for (const el of syncTargets.scroll) {
        el.removeEventListener("scroll", onWindowResize);
      }
      window.removeEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, restore);
      visualViewport?.removeEventListener("resize", onWindowResize);
      visualViewport?.removeEventListener("scroll", onWindowResize);
      if (typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(layoutRaf);
      }
    };
  }, [
    hostRef,
    layoutKey,
    session.sid,
    session.url,
    userAgent,
    viewportWidth,
    viewportHeight,
  ]);

  return (
    <div
      className={cn("h-full min-h-0 w-full bg-background")}
      data-layout-key={layoutKey}
      data-testid="playground-webview-host"
      data-user-agent={userAgent}
      data-viewport-width={viewport?.width}
      data-viewport-height={viewport?.height}
      ref={hostRef}
      style={
        viewport
          ? { width: viewport.width, height: viewport.height }
          : undefined
      }
    />
  );
}
