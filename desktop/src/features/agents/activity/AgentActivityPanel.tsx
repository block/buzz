// Ported/adapted from Berd's agent-activity-panel (branch
// zmarley/agent-activity-panel). Canvas activity panel with DVR transport:
// live file-activity map of an agent turn/session with scrub, speeds,
// keyboard control, and intent-first hover tooltips.

import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import type { ObserverEvent } from "../ui/agentSessionTypes";
import type { ActivityScene, ActivitySceneEvent } from "./activityScene";
import {
  ACTIVITY_SCRUB_STEP_MS,
  type ActivityTransportState,
  activityTransportBadge,
  createActivityTransport,
  cycleActivitySpeed,
  jumpActivityTransport,
  jumpToLiveActivityTransport,
  restartActivityTransport,
  scrubActivityTransport,
  tickActivityTransport,
  toggleActivityPlayback,
} from "./activityTransport";
import type { deriveActivityTurns } from "./activityTurns";
import { inferActivityRoots } from "./deriveActivity";
import { type ActivityHoverView, drawFrame } from "./renderScene";
import {
  type ActivityEventHitTarget,
  type ActivityRiderHitTarget,
  canvasXToTime,
  DEFAULT_ACTIVITY_THEME,
  type FrameHitTargets,
  formatActivityDuration,
  getActivityPanelLayout,
  preferredActivityContentHeight,
} from "./renderSceneLayout";
import {
  type ActivityScope,
  turnScope,
  useActivityScene,
} from "./useActivityScene";

const EVENT_VERBS: Record<string, string> = {
  r: "read",
  w: "edited",
  c: "created",
  x: "write collision",
  sp: "spawned",
  d: "done",
  s: "ran",
};

interface HoverState {
  view: ActivityHoverView;
  clientX: number;
  clientY: number;
  target: ActivityEventHitTarget | null;
  rider: ActivityRiderHitTarget | null;
  rowPath: string | null;
  gapLabel: string | null;
}

/** Auto viewport cap before the canvas starts scrolling. */
const DEFAULT_VIEWPORT_MAX_HEIGHT = 380;
const MIN_VIEWPORT_HEIGHT = 140;
const MAX_VIEWPORT_HEIGHT = 900;

function clampViewportHeight(height: number): number {
  return Math.min(Math.max(height, MIN_VIEWPORT_HEIGHT), MAX_VIEWPORT_HEIGHT);
}

export interface AgentActivityPanelProps {
  frames: readonly ObserverEvent[];
  agentPubkey: string;
  agentName?: string;
  agentIndex?: number;
  /** Canonical "agent is working" signal from the host surface. */
  isLive: boolean;
  className?: string;
  /** True when older archived history exists beyond the loaded window. */
  hasOlderHistory?: boolean;
  /** Pages older archived frames into the window (store-backed). */
  onLoadOlder?: () => Promise<unknown>;
}

export function AgentActivityPanel({
  frames,
  agentPubkey,
  agentName,
  agentIndex,
  isLive,
  className,
  hasOlderHistory,
  onLoadOlder,
}: AgentActivityPanelProps) {
  const [scope, setScope] = React.useState<ActivityScope>("session");

  const roots = React.useMemo(() => inferActivityRoots(frames), [frames]);

  const { scene, turns } = useActivityScene(frames, {
    agentId: agentPubkey,
    ...(agentName !== undefined ? { agentName } : {}),
    ...(agentIndex !== undefined ? { agentIndex } : {}),
    roots,
    scope,
  });

  // Scoping to a turn that later leaves the retention window resets cleanly.
  React.useEffect(() => {
    if (scope === "session") return;
    const turnId = scope.slice("turn:".length);
    if (!turns.some((turn) => turn.id === turnId)) setScope("session");
  }, [scope, turns]);

  const scopeIsLive = React.useMemo(() => {
    if (!isLive) return false;
    if (scope === "session") return true;
    const turnId = scope.slice("turn:".length);
    const turn = turns.find((candidate) => candidate.id === turnId);
    return turn !== undefined && turn.endT === undefined;
  }, [isLive, scope, turns]);

  if (!scene || scene.events.length === 0) {
    return (
      <div
        className={cn(
          "rounded-lg border border-border/70 bg-muted/30 px-3 py-4 text-sm text-muted-foreground",
          className,
        )}
      >
        No file activity in this window yet. Activity appears when the agent
        reads or edits files during a turn.
      </div>
    );
  }

  return (
    <ActivityPanelBody
      key={`${agentPubkey}:${scope}`}
      scene={scene}
      scopeIsLive={scopeIsLive}
      scope={scope}
      setScope={setScope}
      turns={turns}
      className={className}
      hasOlderHistory={hasOlderHistory}
      onLoadOlder={onLoadOlder}
    />
  );
}

export function ActivityPanelBody({
  scene,
  scopeIsLive,
  scope,
  setScope,
  turns,
  className,
  pickerSlot,
  onRiderClick,
  hasOlderHistory,
  onLoadOlder,
}: {
  scene: ActivityScene;
  scopeIsLive: boolean;
  scope: ActivityScope;
  setScope: (scope: ActivityScope) => void;
  turns: ReturnType<typeof deriveActivityTurns>;
  className?: string;
  /** Replaces the turn picker when set (e.g. agent chips in channel view). */
  pickerSlot?: React.ReactNode;
  /** Makes agent riders clickable (canvas dots on the playhead). */
  onRiderClick?: (agentId: string) => void;
  /** Shows a "Load older" control that pages archived history in. */
  hasOlderHistory?: boolean;
  onLoadOlder?: () => Promise<unknown>;
}) {
  const [loadingOlder, setLoadingOlder] = React.useState(false);
  const loadOlder = React.useCallback(async () => {
    if (!onLoadOlder) return;
    setLoadingOlder(true);
    try {
      await onLoadOlder();
    } finally {
      setLoadingOlder(false);
    }
  }, [onLoadOlder]);
  const containerRef = React.useRef<HTMLElement | null>(null);
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const hitTargetsRef = React.useRef<FrameHitTargets | null>(null);
  const hoverRef = React.useRef<HoverState | null>(null);
  const draggingRef = React.useRef(false);
  const [hover, setHover] = React.useState<HoverState | null>(null);
  const [size, setSize] = React.useState({ width: 0, height: 0 });
  // User-dragged viewport height; null = auto (fit content up to the cap).
  const [userHeight, setUserHeight] = React.useState<number | null>(null);
  const resizeDragRef = React.useRef<{
    startY: number;
    startHeight: number;
  } | null>(null);

  // Transport lives in a ref for the rAF loop; a state mirror drives the
  // control chrome (badge/speed/playing) without re-rendering per frame.
  const boundsRef = React.useRef({
    d0: scene.d0,
    d1: scene.d1,
    isLive: scopeIsLive,
  });
  boundsRef.current = { d0: scene.d0, d1: scene.d1, isLive: scopeIsLive };

  const transportRef = React.useRef<ActivityTransportState>(
    createActivityTransport(boundsRef.current),
  );
  const [transportChrome, setTransportChrome] = React.useState(() => ({
    ...transportRef.current,
  }));

  const syncChrome = React.useCallback(() => {
    const current = transportRef.current;
    setTransportChrome((previous) =>
      previous.mode === current.mode &&
      previous.playing === current.playing &&
      previous.speed === current.speed
        ? previous
        : { ...current },
    );
  }, []);

  const sceneRef = React.useRef(scene);
  sceneRef.current = scene;

  // Size tracking: width follows the container; height is the scene's
  // natural content height (rows at comfortable size, growing with the
  // tree) so nothing gets crushed — the viewport div scrolls when content
  // exceeds it. Height changes with node count, so live turns that reveal
  // new files grow the canvas.
  const contentHeight = preferredActivityContentHeight(scene.nodes.length);
  React.useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const { width } = entry.contentRect;
      setSize((previous) =>
        previous.width === width && previous.height === contentHeight
          ? previous
          : { width, height: contentHeight },
      );
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [contentHeight]);

  // Draw loop: paused when the document is hidden.
  React.useEffect(() => {
    let raf = 0;
    let lastTick: number | null = null;
    let running = true;

    const step = (now: number) => {
      if (!running) return;
      const dt = lastTick === null ? 0 : now - lastTick;
      lastTick = now;

      const next = tickActivityTransport(
        transportRef.current,
        dt,
        boundsRef.current,
      );
      if (next !== transportRef.current) {
        transportRef.current = next;
        syncChrome();
      }

      const canvas = canvasRef.current;
      const ctx = canvas?.getContext("2d");
      const currentScene = sceneRef.current;
      if (canvas && ctx && canvas.width > 0) {
        const dpr = window.devicePixelRatio || 1;
        hitTargetsRef.current = drawFrame(
          ctx,
          currentScene,
          {
            viewT: transportRef.current.viewT,
            liveT: boundsRef.current.d1,
            width: canvas.width / dpr,
            height: canvas.height / dpr,
            dpr,
            hover: hoverRef.current?.view ?? null,
          },
          DEFAULT_ACTIVITY_THEME,
        );
      }

      raf = requestAnimationFrame(step);
    };

    const onVisibility = () => {
      if (document.hidden) {
        cancelAnimationFrame(raf);
        lastTick = null;
      } else {
        raf = requestAnimationFrame(step);
      }
    };

    raf = requestAnimationFrame(step);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      running = false;
      cancelAnimationFrame(raf);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [syncChrome]);

  const applyTransport = React.useCallback(
    (
      update: (
        state: ActivityTransportState,
        bounds: { d0: number; d1: number; isLive: boolean },
      ) => ActivityTransportState,
    ) => {
      transportRef.current = update(transportRef.current, boundsRef.current);
      syncChrome();
    },
    [syncChrome],
  );

  const scrubToCanvasX = React.useCallback((clientX: number) => {
    const canvas = canvasRef.current;
    const targets = hitTargetsRef.current;
    if (!canvas || !targets) return;
    const rect = canvas.getBoundingClientRect();
    const x = clientX - rect.left;
    const nodeCount = sceneRef.current.nodes.length;
    const layout = getActivityPanelLayout(rect.width, rect.height, nodeCount);
    const displayT = canvasXToTime(sceneRef.current, x, layout);
    transportRef.current = scrubActivityTransport(
      transportRef.current,
      displayT,
      boundsRef.current,
    );
  }, []);

  const handlePointerMove = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      if (draggingRef.current) {
        scrubToCanvasX(event.clientX);
        return;
      }
      const canvas = canvasRef.current;
      const targets = hitTargetsRef.current;
      if (!canvas || !targets) return;
      const rect = canvas.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;

      // Riders take precedence when clickable — they sit on the playhead
      // above event markers and are the navigation affordance.
      let rider: ActivityRiderHitTarget | null = null;
      if (onRiderClick) {
        let riderDistance = Number.POSITIVE_INFINITY;
        for (const target of targets.riders) {
          const distance = Math.hypot(target.x - x, target.y - y);
          if (distance <= target.radius && distance < riderDistance) {
            rider = target;
            riderDistance = distance;
          }
        }
      }

      let nearest: ActivityEventHitTarget | null = null;
      let nearestDistance = Number.POSITIVE_INFINITY;
      if (!rider) {
        for (const target of targets.events) {
          const dx = target.x - x;
          const dy = target.y - y;
          const distance = Math.hypot(dx, dy);
          if (distance <= target.radius + 3 && distance < nearestDistance) {
            nearest = target;
            nearestDistance = distance;
          }
        }
      }

      canvas.style.cursor = rider ? "pointer" : "crosshair";

      let rowPath: string | null = null;
      if (!rider && !nearest) {
        for (const row of targets.rows) {
          if (
            x >= row.x &&
            x <= row.x + row.width &&
            y >= row.y &&
            y <= row.y + row.height
          ) {
            rowPath = row.node.path;
            break;
          }
        }
      }

      let gapLabel: string | null = null;
      let gapId: string | undefined;
      if (!rider && !nearest && !rowPath) {
        for (const gap of targets.gaps) {
          if (
            x >= gap.x &&
            x <= gap.x + gap.width &&
            y >= gap.y &&
            y <= gap.y + gap.height
          ) {
            gapLabel = `${formatActivityDuration(gap.realEnd - gap.realStart)} idle`;
            gapId = gap.id;
            break;
          }
        }
      }

      if (!rider && !nearest && !rowPath && !gapLabel) {
        hoverRef.current = null;
        setHover(null);
        return;
      }

      const next: HoverState = {
        view: {
          ...(rowPath ? { rowPath } : {}),
          ...(nearest ? { eventId: nearest.event.id } : {}),
          ...(gapId ? { gapId } : {}),
        },
        clientX: event.clientX,
        clientY: event.clientY,
        target: nearest,
        rider,
        rowPath,
        gapLabel,
      };
      hoverRef.current = next;
      setHover(next);
    },
    [onRiderClick, scrubToCanvasX],
  );

  const handlePointerLeave = React.useCallback(() => {
    hoverRef.current = null;
    draggingRef.current = false;
    setHover(null);
  }, []);

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const targets = hitTargetsRef.current;
      const canvas = canvasRef.current;
      if (!canvas || !targets) return;
      const rect = canvas.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;

      // Rider click → navigate to that agent instead of scrubbing.
      if (onRiderClick) {
        for (const target of targets.riders) {
          if (Math.hypot(target.x - x, target.y - y) <= target.radius) {
            event.preventDefault();
            onRiderClick(target.agentId);
            return;
          }
        }
      }

      const { chart } = targets;
      if (x < chart.x0 || x > chart.x1 || y < chart.y0 || y > chart.y1) {
        return;
      }
      draggingRef.current = true;
      canvas.setPointerCapture(event.pointerId);
      scrubToCanvasX(event.clientX);
      syncChrome();
    },
    [onRiderClick, scrubToCanvasX, syncChrome],
  );

  const handlePointerUp = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      canvasRef.current?.releasePointerCapture(event.pointerId);
    },
    [],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const bounds = boundsRef.current;
      switch (event.key) {
        case " ":
          event.preventDefault();
          applyTransport(toggleActivityPlayback);
          break;
        case "ArrowLeft":
          event.preventDefault();
          applyTransport((state) =>
            jumpActivityTransport(state, -ACTIVITY_SCRUB_STEP_MS, bounds),
          );
          break;
        case "ArrowRight":
          event.preventDefault();
          applyTransport((state) =>
            jumpActivityTransport(state, ACTIVITY_SCRUB_STEP_MS, bounds),
          );
          break;
        case "l":
        case "L":
          event.preventDefault();
          applyTransport(jumpToLiveActivityTransport);
          break;
        case "r":
        case "R":
          event.preventDefault();
          applyTransport(restartActivityTransport);
          break;
        default:
          break;
      }
    },
    [applyTransport],
  );

  const badge = activityTransportBadge(transportChrome, {
    d0: scene.d0,
    d1: scene.d1,
    isLive: scopeIsLive,
  });

  const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;

  // Current visible viewport height: the user's dragged height, else content
  // height up to the auto cap. The separator reports/adjusts this value.
  const effectiveViewportHeight =
    userHeight ??
    Math.min(
      size.height || DEFAULT_VIEWPORT_MAX_HEIGHT,
      DEFAULT_VIEWPORT_MAX_HEIGHT,
    );

  return (
    <section
      ref={containerRef}
      className={cn(
        "flex flex-col gap-2 rounded-lg border border-border/70 bg-background/80 p-2 shadow-xs focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
        className,
      )}
      aria-label="Agent file activity"
      // biome-ignore lint/a11y/noNoninteractiveTabindex: the section is the DVR keyboard surface (Space/arrows/L/R), same pattern as TerminalSubstrate.
      tabIndex={0}
      onKeyDown={handleKeyDown}
    >
      <div className="flex flex-wrap items-center gap-2">
        <TransportBadge badge={badge} />
        {pickerSlot ?? (
          <TurnScopePicker scope={scope} setScope={setScope} turns={turns} />
        )}
        {hasOlderHistory && onLoadOlder ? (
          <Button
            disabled={loadingOlder}
            size="xs"
            type="button"
            variant="ghost"
            onClick={() => void loadOlder()}
          >
            {loadingOlder ? "Loading…" : "Load older"}
          </Button>
        ) : null}
        <div className="ml-auto flex items-center gap-1">
          <Button
            size="xs"
            type="button"
            variant="ghost"
            onClick={() =>
              applyTransport((state) =>
                jumpActivityTransport(
                  state,
                  -ACTIVITY_SCRUB_STEP_MS,
                  boundsRef.current,
                ),
              )
            }
          >
            −2s
          </Button>
          <Button
            size="xs"
            type="button"
            variant="ghost"
            onClick={() => applyTransport(toggleActivityPlayback)}
          >
            {transportChrome.mode === "replay" && !transportChrome.playing
              ? "Play"
              : "Pause"}
          </Button>
          <Button
            size="xs"
            type="button"
            variant="ghost"
            onClick={() =>
              applyTransport((state) =>
                jumpActivityTransport(
                  state,
                  ACTIVITY_SCRUB_STEP_MS,
                  boundsRef.current,
                ),
              )
            }
          >
            +2s
          </Button>
          <Button
            size="xs"
            type="button"
            variant="ghost"
            onClick={() => applyTransport(cycleActivitySpeed)}
          >
            {transportChrome.speed}×
          </Button>
          {scopeIsLive && transportChrome.mode !== "live" ? (
            <Button
              size="xs"
              type="button"
              variant="secondary"
              onClick={() => applyTransport(jumpToLiveActivityTransport)}
            >
              Live
            </Button>
          ) : null}
          <Button
            size="xs"
            type="button"
            variant="ghost"
            onClick={() => applyTransport(restartActivityTransport)}
          >
            Restart
          </Button>
        </div>
      </div>

      <div className="relative">
        <div
          className="overflow-y-auto overscroll-contain rounded-md"
          style={{ maxHeight: effectiveViewportHeight }}
        >
          <canvas
            ref={canvasRef}
            className="w-full cursor-crosshair"
            style={{ height: size.height }}
            width={Math.max(1, Math.floor(size.width * dpr))}
            height={Math.max(1, Math.floor(size.height * dpr))}
            aria-label="Agent activity timeline"
            onPointerMove={handlePointerMove}
            onPointerLeave={handlePointerLeave}
            onPointerDown={handlePointerDown}
            onPointerUp={handlePointerUp}
          />
        </div>
        {hover ? <ActivityTooltip hover={hover} /> : null}
      </div>

      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 truncate text-2xs text-muted-foreground/70">
          Space play/pause · ←/→ scrub 2s · L live · R restart
        </p>
        {/* biome-ignore lint/a11y/useSemanticElements: an <hr> can't host the drag/keyboard resize interaction; window-splitter-style separator per WAI-ARIA. */}
        <div
          aria-label="Activity panel height"
          aria-orientation="horizontal"
          aria-valuemax={MAX_VIEWPORT_HEIGHT}
          aria-valuemin={MIN_VIEWPORT_HEIGHT}
          aria-valuenow={Math.round(effectiveViewportHeight)}
          className="h-2 w-16 shrink-0 cursor-row-resize touch-none rounded-full bg-border/70 hover:bg-border focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          role="separator"
          tabIndex={0}
          title="Drag or ↑/↓ to resize · double-click or Backspace to reset"
          onDoubleClick={() => setUserHeight(null)}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp" || event.key === "ArrowDown") {
              event.preventDefault();
              event.stopPropagation();
              const delta = event.key === "ArrowUp" ? -32 : 32;
              setUserHeight(
                clampViewportHeight(effectiveViewportHeight + delta),
              );
            } else if (event.key === "Backspace" || event.key === "Delete") {
              event.preventDefault();
              event.stopPropagation();
              setUserHeight(null);
            }
          }}
          onPointerDown={(event) => {
            event.preventDefault();
            event.currentTarget.setPointerCapture(event.pointerId);
            resizeDragRef.current = {
              startY: event.clientY,
              startHeight: effectiveViewportHeight,
            };
          }}
          onPointerMove={(event) => {
            const drag = resizeDragRef.current;
            if (!drag) return;
            const next = drag.startHeight + (event.clientY - drag.startY);
            setUserHeight(clampViewportHeight(next));
          }}
          onPointerUp={(event) => {
            resizeDragRef.current = null;
            event.currentTarget.releasePointerCapture(event.pointerId);
          }}
        />
      </div>
    </section>
  );
}

function TransportBadge({ badge }: { badge: "live" | "replay" | "idle" }) {
  if (badge === "live") {
    return <Badge variant="success">Live</Badge>;
  }
  if (badge === "idle") {
    // "Idle", not "Ended": the scope can wake again on the next turn.
    return <Badge variant="secondary">Idle</Badge>;
  }
  return <Badge variant="info">Replay</Badge>;
}

function TurnScopePicker({
  scope,
  setScope,
  turns,
}: {
  scope: ActivityScope;
  setScope: (scope: ActivityScope) => void;
  turns: ReturnType<typeof deriveActivityTurns>;
}) {
  if (turns.length === 0) return null;
  return (
    <select
      className="h-6 rounded-md border border-input/40 bg-background px-1 text-xs text-foreground"
      aria-label="Activity scope"
      value={scope}
      onChange={(event) => {
        const value = event.target.value;
        setScope(value === "session" ? "session" : turnScope(value.slice(5)));
      }}
    >
      <option value="session">Whole session</option>
      {turns.map((turn, index) => (
        <option key={turn.id} value={turnScope(turn.id)}>
          Turn {index + 1}
          {turn.source === "heartbeat" ? " (heartbeat)" : ""}
          {turn.endT === undefined ? " — live" : ""}
        </option>
      ))}
    </select>
  );
}

function describeEvent(event: ActivitySceneEvent): string {
  const verb = EVENT_VERBS[event.kind] ?? event.kind;
  if (event.path) return `${verb} ${event.path}`;
  return event.label ? `${verb} · ${event.label}` : verb;
}

function ActivityTooltip({ hover }: { hover: HoverState }) {
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const [offset, setOffset] = React.useState({ left: 0, top: 0 });

  React.useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const parent = element.offsetParent as HTMLElement | null;
    if (!parent) return;
    const parentRect = parent.getBoundingClientRect();
    const width = element.offsetWidth;
    let left = hover.clientX - parentRect.left + 14;
    const top = hover.clientY - parentRect.top + 14;
    if (hover.clientX + 14 + width > window.innerWidth - 8) {
      left = hover.clientX - parentRect.left - width - 14;
    }
    setOffset({ left, top });
  }, [hover]);

  const target = hover.target;
  const intent = target?.event.intent ?? target?.clusterFirst?.intent;

  return (
    <div
      ref={containerRef}
      className="pointer-events-none absolute z-20 max-w-72 rounded-md border border-border/70 bg-popover px-2.5 py-2 text-xs text-popover-foreground shadow-md"
      style={{ left: offset.left, top: offset.top }}
    >
      {intent ? (
        <p className="mb-1 line-clamp-4 text-muted-foreground italic">
          {intent}
        </p>
      ) : null}
      {hover.rider ? (
        <p className="font-medium">{hover.rider.agentName} — view activity</p>
      ) : null}
      {!hover.rider && target ? (
        target.clusterCount && target.clusterCount > 1 ? (
          <div>
            <p className="font-medium">ran {target.clusterCount}×</p>
            {target.clusterLabels ? (
              <p className="text-muted-foreground">
                {[...new Set(target.clusterLabels)].slice(0, 4).join(", ")}
              </p>
            ) : null}
          </div>
        ) : (
          <p className="font-medium">{describeEvent(target.event)}</p>
        )
      ) : null}
      {!hover.rider && !target && hover.rowPath ? (
        <p className="font-medium">{hover.rowPath}</p>
      ) : null}
      {!hover.rider && !target && hover.gapLabel ? (
        <p className="font-medium">{hover.gapLabel}</p>
      ) : null}
    </div>
  );
}
