// Full-screen screenshot: image height fills the page; pinch / wheel / +- zoom, double-tap toggle, drag to pan, Back/Esc.
import { ArrowLeft, Camera, Minus, Plus, RotateCcw } from "lucide-react";
import * as React from "react";
import { createPortal } from "react-dom";

import { Button } from "@/shared/ui/button";

import {
  IDENTITY,
  type Size,
  type View,
  baseSize,
  clampView,
  panBy,
  toggleZoom,
  wheelFactor,
  zoomAt,
} from "../lib/zoomPan";

const TAP_MS = 300;
const TAP_SLOP = 10;

export function ScreenshotFullscreen({
  busy,
  disabled,
  onBack,
  onNew,
  src,
  statusText,
  takenAt,
  title,
}: {
  busy: boolean;
  disabled: boolean;
  onBack: () => void;
  onNew: () => void;
  src: string;
  statusText: string | null;
  takenAt: number | null;
  title: string;
}) {
  const stageRef = React.useRef<HTMLDivElement>(null);
  const [stage, setStage] = React.useState<Size>({ w: 0, h: 0 });
  const [aspect, setAspect] = React.useState(1);
  const [view, setView] = React.useState<View>(IDENTITY);
  const viewRef = React.useRef(view);
  viewRef.current = view;
  const base = React.useMemo(() => baseSize(stage, aspect), [stage, aspect]);
  const baseRef = React.useRef(base);
  baseRef.current = base;
  const stageSizeRef = React.useRef(stage);
  stageSizeRef.current = stage;
  const pointers = React.useRef(new Map<number, { x: number; y: number }>());
  const gesture = React.useRef<{ dist: number; mid: { x: number; y: number } } | null>(null);
  const tap = React.useRef<{ t: number; x: number; y: number; moved: number } | null>(null);
  const lastTap = React.useRef<{ t: number; x: number; y: number } | null>(null);

  const apply = React.useCallback((fn: (v: View, b: Size, s: Size) => View) => {
    setView(fn(viewRef.current, baseRef.current, stageSizeRef.current));
  }, []);
  const rel = (cx: number, cy: number) => {
    const r = stageRef.current?.getBoundingClientRect();
    return r ? { x: cx - r.left - r.width / 2, y: cy - r.top - r.height / 2 } : { x: 0, y: 0 };
  };

  React.useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const measure = () => setStage({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    ro?.observe(el);
    window.addEventListener("resize", measure);
    return () => {
      ro?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);
  // Re-clamp when the stage or image shape changes (rotation, new screenshot).
  React.useEffect(() => setView((v) => clampView(v, base, stage)), [base, stage]);

  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onBack();
      else if (e.key === "+" || e.key === "=") apply((v, b, s) => zoomAt(v, 1.5, { x: 0, y: 0 }, b, s));
      else if (e.key === "-") apply((v, b, s) => zoomAt(v, 1 / 1.5, { x: 0, y: 0 }, b, s));
      else if (e.key === "0") setView(IDENTITY);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onBack, apply]);

  // Wheel / trackpad pinch must be non-passive to stop the page from scrolling or zooming.
  React.useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const at = rel(e.clientX, e.clientY);
      const f = wheelFactor(e.deltaY * (e.ctrlKey ? 2 : 1), e.deltaMode);
      apply((v, b, s) => zoomAt(v, f, at, b, s));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [apply]);

  function onPointerDown(e: React.PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size === 1) tap.current = { t: Date.now(), x: e.clientX, y: e.clientY, moved: 0 };
    else tap.current = null;
    gesture.current = null;
  }
  function onPointerMove(e: React.PointerEvent) {
    const p = pointers.current.get(e.pointerId);
    if (!p) return;
    const prev = { ...p };
    p.x = e.clientX;
    p.y = e.clientY;
    if (pointers.current.size >= 2) {
      const [a, b2] = [...pointers.current.values()];
      if (!a || !b2) return;
      const dist = Math.hypot(a.x - b2.x, a.y - b2.y) || 1;
      const mid = rel((a.x + b2.x) / 2, (a.y + b2.y) / 2);
      const g = gesture.current;
      if (g) {
        apply((v, bs, s) => panBy(zoomAt(v, dist / g.dist, mid, bs, s), mid.x - g.mid.x, mid.y - g.mid.y, bs, s));
      }
      gesture.current = { dist, mid };
      return;
    }
    if (tap.current) tap.current.moved += Math.abs(e.clientX - prev.x) + Math.abs(e.clientY - prev.y);
    apply((v, bs, s) => panBy(v, e.clientX - prev.x, e.clientY - prev.y, bs, s));
  }
  function onPointerUp(e: React.PointerEvent) {
    pointers.current.delete(e.pointerId);
    gesture.current = null;
    const t = tap.current;
    tap.current = null;
    if (!t || t.moved > TAP_SLOP || Date.now() - t.t > TAP_MS) return;
    const now = Date.now();
    const prevTap = lastTap.current;
    if (prevTap && now - prevTap.t < TAP_MS && Math.hypot(prevTap.x - e.clientX, prevTap.y - e.clientY) < 30) {
      lastTap.current = null;
      const at = rel(e.clientX, e.clientY);
      apply((v, b, s) => toggleZoom(v, at, b, s));
    } else lastTap.current = { t: now, x: e.clientX, y: e.clientY };
  }
  function onPointerCancel(e: React.PointerEvent) {
    pointers.current.delete(e.pointerId);
    gesture.current = null;
    tap.current = null;
  }

  const zoomed = view.scale > 1.01;
  return createPortal(
    <div
      aria-label={`Full screen screenshot of ${title}`}
      aria-modal="true"
      className="fixed inset-0 z-[80] flex flex-col bg-black text-white"
      data-testid="shot-fullscreen"
      role="dialog"
    >
      <div
        className="flex shrink-0 items-center gap-2 bg-black/70 px-3 pb-2"
        style={{ paddingTop: "max(0.5rem, env(safe-area-inset-top))", paddingLeft: "max(0.75rem, env(safe-area-inset-left))", paddingRight: "max(0.75rem, env(safe-area-inset-right))" }}
      >
        <Button data-testid="shot-fs-back" onClick={onBack} size="xs" type="button" variant="secondary">
          <ArrowLeft className="mr-1 h-3 w-3" />
          Back
        </Button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">{title}</p>
          <p className="truncate text-2xs text-white/70" data-testid="shot-fs-time">
            {takenAt ? `Screenshot taken ${new Date(takenAt * 1000).toLocaleTimeString()}` : ""}
            {statusText ? ` · ${statusText}` : ""}
          </p>
        </div>
        <Button aria-label="Zoom out" data-testid="shot-fs-out" disabled={!zoomed} onClick={() => apply((v, b, s) => zoomAt(v, 1 / 1.5, { x: 0, y: 0 }, b, s))} size="icon-xs" type="button" variant="secondary">
          <Minus />
        </Button>
        <span className="w-10 text-center text-2xs tabular-nums" data-testid="shot-fs-zoom">{Math.round(view.scale * 100)}%</span>
        <Button aria-label="Zoom in" data-testid="shot-fs-in" onClick={() => apply((v, b, s) => zoomAt(v, 1.5, { x: 0, y: 0 }, b, s))} size="icon-xs" type="button" variant="secondary">
          <Plus />
        </Button>
        <Button aria-label="Reset zoom" data-testid="shot-fs-reset" disabled={!zoomed && view.x === 0 && view.y === 0} onClick={() => setView(IDENTITY)} size="icon-xs" type="button" variant="secondary">
          <RotateCcw />
        </Button>
        <Button data-testid="shot-fs-new" disabled={disabled || busy} onClick={onNew} size="xs" type="button" variant="secondary">
          <Camera className="mr-1 h-3 w-3" />
          <span className="hidden sm:inline">New screenshot</span>
          <span className="sm:hidden">New</span>
        </Button>
      </div>
      <div
        className="relative min-h-0 flex-1 touch-none select-none overflow-hidden"
        data-testid="shot-fs-stage"
        data-wheel-zoom=""
        onPointerCancel={onPointerCancel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        ref={stageRef}
        style={{ cursor: zoomed ? "grab" : "zoom-in", paddingBottom: "env(safe-area-inset-bottom)" }}
      >
        <div className="absolute inset-0 flex items-center justify-center">
          <img
            alt={`Screenshot of ${title}`}
            className="max-w-none"
            data-scale={view.scale.toFixed(2)}
            data-testid="shot-fs-img"
            draggable={false}
            onLoad={(e) => {
              const i = e.currentTarget;
              if (i.naturalWidth && i.naturalHeight) setAspect(i.naturalWidth / i.naturalHeight);
            }}
            src={src}
            style={{
              height: stage.h || "100%",
              width: stage.h ? stage.h * aspect : "auto",
              transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
              transformOrigin: "center center",
              willChange: "transform",
            }}
          />
        </div>
      </div>
    </div>,
    document.body,
  );
}
