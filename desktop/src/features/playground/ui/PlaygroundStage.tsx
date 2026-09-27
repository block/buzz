import * as React from "react";

import {
  AGENT_DRIVING_CHROME_TOOLTIP,
  isAgentDrivingChromeLocked,
} from "@/features/browser-agent/lib/chromeLock";
import {
  getBrowserAgentGrant,
  subscribeBrowserAgentGrant,
} from "@/features/browser-agent/lib/api";
import { browserWebviewLabel } from "@/features/browser-agent/lib/labels";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
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
import {
  getPlaygroundViewport,
  setPlaygroundViewport,
  subscribePlaygroundViewport,
  type PlaygroundChromeMode,
} from "../lib/playgroundViewport";
import { DEFAULT_RESPONSIVE_VIEWPORT } from "../lib/types";
import { PLAYGROUND_DOM_PROBE_SCRIPT } from "../lib/updates";
import {
  evalPlaygroundWebview,
  hidePlaygroundWebview,
  PLAYGROUND_WEBVIEW_RESTORE_EVENT,
  playgroundWebviewBoundsAreUsable,
  showPlaygroundWebview,
  currentWindowLabel,
} from "../lib/webview";
import type { PlaygroundSession } from "../lib/sessions";
import { DeviceBezel } from "./DeviceBezel";
import { DeviceMuseumToolbar } from "./DeviceMuseumToolbar";

export type { PlaygroundChromeMode };

function useViewportControlsLocked(
  sessionSid: string,
  override?: boolean,
): boolean {
  const webviewLabel = browserWebviewLabel({
    surface: "playground",
    surfaceId: sessionSid,
    windowLabel: currentWindowLabel(),
  });
  const [grant, setGrant] = React.useState<BrowserAgentGrant | null>(null);
  React.useEffect(() => {
    if (override !== undefined) return;
    let cancelled = false;
    void getBrowserAgentGrant(webviewLabel)
      .then((next) => {
        if (!cancelled) setGrant(next);
      })
      .catch(() => {
        if (!cancelled) setGrant(null);
      });
    let unlisten: Promise<() => void> | null = null;
    try {
      unlisten = subscribeBrowserAgentGrant((payload) => {
        if (payload.webviewLabel !== webviewLabel) return;
        setGrant(payload.grant);
      });
      void unlisten.catch(() => undefined);
    } catch {
      unlisten = null;
    }
    return () => {
      cancelled = true;
      if (unlisten) void unlisten.then((stop) => stop()).catch(() => undefined);
    };
  }, [override, webviewLabel]);
  if (override !== undefined) return override;
  return isAgentDrivingChromeLocked(grant);
}

export function PlaygroundStage({
  layoutKey = "window:0",
  mode,
  session,
  controlsLocked,
}: {
  layoutKey?: string;
  mode: PlaygroundChromeMode;
  session: PlaygroundSession;
  /** When set, forces viewport control lock (layout/unit tests). */
  controlsLocked?: boolean;
}) {
  const locked = useViewportControlsLocked(session.sid, controlsLocked);
  if (mode === "mobile") {
    return (
      <MobileDeviceMuseum
        controlsLocked={locked}
        layoutKey={layoutKey}
        session={session}
      />
    );
  }
  if (mode === "responsive") {
    return (
      <ResponsiveStage
        controlsLocked={locked}
        layoutKey={layoutKey}
        session={session}
      />
    );
  }
  return (
    <DesktopStage
      controlsLocked={locked}
      layoutKey={layoutKey}
      session={session}
    />
  );
}

function DesktopStage({
  layoutKey,
  session,
  controlsLocked,
}: {
  layoutKey: string;
  session: PlaygroundSession;
  controlsLocked: boolean;
}) {
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  React.useEffect(() => {
    setPlaygroundViewport(session.sid, { mode: "desktop" });
  }, [session.sid]);
  React.useEffect(() => {
    const host = hostRef.current;
    if (!host || typeof ResizeObserver !== "function") return;
    const publish = () => {
      const rect = host.getBoundingClientRect();
      const width = Math.round(rect.width);
      const height = Math.round(rect.height);
      if (width < 1 || height < 1) return;
      setPlaygroundViewport(session.sid, {
        mode: "desktop",
        width,
        height,
      });
    };
    publish();
    const ro = new ResizeObserver(publish);
    ro.observe(host);
    return () => ro.disconnect();
  }, [session.sid, layoutKey]);
  void controlsLocked;
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
  controlsLocked,
}: {
  layoutKey: string;
  session: PlaygroundSession;
  controlsLocked: boolean;
}) {
  const stored = React.useSyncExternalStore(
    subscribePlaygroundViewport,
    () => getPlaygroundViewport(session.sid),
    () => getPlaygroundViewport(session.sid),
  );
  const width =
    stored.mode === "responsive" && stored.width > 0
      ? stored.width
      : DEFAULT_RESPONSIVE_VIEWPORT.width;
  const height =
    stored.mode === "responsive" && stored.height > 0
      ? stored.height
      : DEFAULT_RESPONSIVE_VIEWPORT.height;
  const setWidth = React.useCallback(
    (next: number) => {
      setPlaygroundViewport(session.sid, {
        mode: "responsive",
        width: Math.max(320, next),
        height,
      });
    },
    [session.sid, height],
  );
  const setHeight = React.useCallback(
    (next: number) => {
      setPlaygroundViewport(session.sid, {
        mode: "responsive",
        width,
        height: Math.max(320, next),
      });
    },
    [session.sid, width],
  );
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  const lockTitle = controlsLocked
    ? AGENT_DRIVING_CHROME_TOOLTIP
    : undefined;

  React.useEffect(() => {
    if (stored.mode === "responsive" && stored.width > 0 && stored.height > 0) {
      return;
    }
    setPlaygroundViewport(session.sid, {
      mode: "responsive",
      width,
      height,
    });
  }, [session.sid, stored.mode, stored.width, stored.height, width, height]);

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-agent-driving={controlsLocked ? "true" : undefined}
      data-testid="playground-responsive-stage"
    >
      <div className="flex flex-wrap items-center gap-2 px-3">
        <label className="flex items-center gap-1 text-2xs text-muted-foreground">
          W
          <input
            className="w-16 rounded-md border border-border bg-background px-1 py-0.5 text-2xs text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="playground-responsive-width"
            disabled={controlsLocked}
            min={320}
            onChange={(event) =>
              setWidth(Math.max(320, Number(event.target.value) || 320))
            }
            title={lockTitle}
            type="number"
            value={width}
          />
        </label>
        <span className="text-2xs text-muted-foreground">×</span>
        <label className="flex items-center gap-1 text-2xs text-muted-foreground">
          H
          <input
            className="w-16 rounded-md border border-border bg-background px-1 py-0.5 text-2xs text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="playground-responsive-height"
            disabled={controlsLocked}
            min={320}
            onChange={(event) =>
              setHeight(Math.max(320, Number(event.target.value) || 320))
            }
            title={lockTitle}
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
            disabled={controlsLocked}
            onResize={(next) => setWidth(Math.max(320, next.width))}
            size={{ width, height }}
            testId="playground-stage-resize"
          />
          <StageResizeHandle
            axis="y"
            disabled={controlsLocked}
            onResize={(next) => setHeight(Math.max(320, next.height))}
            size={{ width, height }}
            testId="playground-stage-resize-y"
          />
          <StageResizeHandle
            axis="xy"
            disabled={controlsLocked}
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
  disabled = false,
  onResize,
  size,
  testId,
}: {
  axis: "x" | "y" | "xy";
  disabled?: boolean;
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
      className={
        disabled
          ? `${PLAYGROUND_RESIZE_HANDLE_CLASS[axis]} pointer-events-none opacity-40`
          : PLAYGROUND_RESIZE_HANDLE_CLASS[axis]
      }
      data-testid={testId}
      disabled={disabled}
      onPointerDown={(event) => {
        if (disabled) return;
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
      title={disabled ? AGENT_DRIVING_CHROME_TOOLTIP : undefined}
      type="button"
    />
  );
}

function MobileDeviceMuseum({
  layoutKey,
  session,
  controlsLocked,
}: {
  layoutKey: string;
  session: PlaygroundSession;
  controlsLocked: boolean;
}) {
  const stored = React.useSyncExternalStore(
    subscribePlaygroundViewport,
    () => getPlaygroundViewport(session.sid),
    () => getPlaygroundViewport(session.sid),
  );
  const deviceId: PlaygroundDeviceId =
    stored.mode === "mobile" && stored.deviceId
      ? stored.deviceId
      : "iphone-16";
  const orientation: "portrait" | "landscape" =
    stored.mode === "mobile" && stored.orientation
      ? stored.orientation
      : "portrait";
  const scalePercent: PlaygroundDeviceScalePercent =
    stored.mode === "mobile" && stored.scalePercent
      ? (stored.scalePercent as PlaygroundDeviceScalePercent)
      : PLAYGROUND_DEVICE_SCALE_DEFAULT;
  const setDeviceId = React.useCallback(
    (next: PlaygroundDeviceId) => {
      setPlaygroundViewport(session.sid, {
        mode: "mobile",
        deviceId: next,
        orientation,
        scalePercent,
      });
    },
    [session.sid, orientation, scalePercent],
  );
  const setOrientation = React.useCallback(
    (next: "portrait" | "landscape") => {
      setPlaygroundViewport(session.sid, {
        mode: "mobile",
        deviceId,
        orientation: next,
        scalePercent,
      });
    },
    [session.sid, deviceId, scalePercent],
  );
  const setScalePercent = React.useCallback(
    (next: PlaygroundDeviceScalePercent) => {
      setPlaygroundViewport(session.sid, {
        mode: "mobile",
        deviceId,
        orientation,
        scalePercent: next,
      });
    },
    [session.sid, deviceId, orientation],
  );
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  const device = PLAYGROUND_DEVICES.find((item) => item.id === deviceId);
  const viewport = scalePlaygroundDeviceViewport(
    device
      ? playgroundDeviceViewport(device, orientation)
      : { width: 393, height: 852 },
    scalePercent,
  );

  React.useEffect(() => {
    setPlaygroundViewport(session.sid, {
      mode: "mobile",
      width: viewport.width,
      height: viewport.height,
      scalePercent,
      deviceId,
      orientation,
    });
  }, [
    session.sid,
    viewport.width,
    viewport.height,
    scalePercent,
    deviceId,
    orientation,
  ]);

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-2"
      data-agent-driving={controlsLocked ? "true" : undefined}
      data-testid="playground-mobile-stage"
    >
      <DeviceMuseumToolbar
        deviceId={deviceId}
        disabled={controlsLocked}
        disabledTitle={AGENT_DRIVING_CHROME_TOOLTIP}
        onDeviceIdChange={setDeviceId}
        onOrientationToggle={() =>
          setOrientation(
            orientation === "portrait" ? "landscape" : "portrait",
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
