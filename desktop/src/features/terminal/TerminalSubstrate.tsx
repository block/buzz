import * as React from "react";
import { ChevronRight, Maximize2, Minimize2, Plus, X } from "lucide-react";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import {
  INITIAL_HANDOFF_STATE,
  accumulateScrollLines,
  encodePaste,
  encodeTerminalKey,
  matchTabChord,
  reduceHandoff,
  stepSession,
} from "./terminalState";
import { buildTerminalBanner } from "./terminalBanner";
import { paintTerminalBanner } from "./terminalBannerPainter";
import { buildBannerColorTable, phaseAt } from "./terminalBannerWave";
import {
  TERMINAL_CELL_METRICS,
  type TerminalFrame,
  type TerminalSelectionRow,
  TerminalGrid,
} from "./terminalRenderer";

export type TerminalViewportSize = {
  columns: number;
  rows: number;
  pixelWidth: number;
  pixelHeight: number;
};

export type TerminalSessionTab = {
  id: string;
  title: string;
  closing: boolean;
  active: boolean;
};

type TerminalSubstrateProps = {
  channelName: string | null;
  frame?: TerminalFrame;
  sessionFrames?: readonly { sessionId: string; frame: TerminalFrame }[];
  sessions: readonly TerminalSessionTab[];
  bracketedPaste: boolean;
  focusReportingEnabled: boolean;
  enabled?: boolean;
  mode?: "docked" | "maximized";
  visible?: boolean;
  onHide?: () => void;
  onModeChange?: (mode: "docked" | "maximized") => void;
  onToggle?: () => void;
  onFrameConsumed?: (frame: TerminalFrame) => void;
  onViewportSize?: (size: TerminalViewportSize) => void;
  viewportReportingEnabled?: boolean;
  showSplash?: boolean;
  onSplashStarted?: () => void;
  onInput: (text: string) => void;
  /** Whole cells scrolled, keeping the DOM's sign: negative goes back. */
  onScroll: (lines: number) => void;
  onTerminalFocusChange: (focused: boolean) => void;
  onSelectSession: (id: string) => void;
  onCloseSession: (id: string) => void;
  onNewSession: () => void;
};

function isToggleChord(event: KeyboardEvent): boolean {
  return (
    event.code === "KeyJ" &&
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey
  );
}

const { width: CELL_WIDTH, height: CELL_HEIGHT } = TERMINAL_CELL_METRICS;
const NOOP = () => {};
const SPLASH_DURATION_MS = 2_500;

export function TerminalSubstrate({
  frame,
  sessionFrames,
  sessions,
  bracketedPaste,
  focusReportingEnabled,
  enabled = true,
  mode = "docked",
  visible = true,
  onHide = NOOP,
  onModeChange = NOOP,
  onToggle,
  onFrameConsumed,
  onViewportSize,
  viewportReportingEnabled = true,
  showSplash = true,
  onSplashStarted,
  onInput,
  onScroll,
  onTerminalFocusChange,
  onSelectSession,
  onCloseSession,
  onNewSession,
}: TerminalSubstrateProps) {
  const { terminalPalette } = useTheme();
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const bannerCanvasRef = React.useRef<HTMLCanvasElement>(null);
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);
  const handoffRef = React.useRef(INITIAL_HANDOFF_STATE);
  const gridsRef = React.useRef(new Map<string, TerminalGrid>());
  const appliedFramesRef = React.useRef(new WeakSet<TerminalFrame>());
  const gridRef = React.useRef<TerminalGrid | null>(null);
  const paintedPaletteRef = React.useRef(terminalPalette);
  const paintedSessionRef = React.useRef<string | null>(null);
  const reportedFocusRef = React.useRef<boolean | null>(null);
  const reportedViewportSizeRef = React.useRef<TerminalViewportSize | null>(
    null,
  );
  const dragCleanupRef = React.useRef<(() => void) | null>(null);
  const resizeReportFrameRef = React.useRef(0);
  const resizingRef = React.useRef(false);
  const scrollBySessionRef = React.useRef(new Map<string, number>());
  const activeSession = sessions.find((session) => session.active);
  const activeSessionId = activeSession?.id ?? null;
  const frames = React.useMemo(
    () =>
      sessionFrames ??
      (activeSessionId && frame ? [{ sessionId: activeSessionId, frame }] : []),
    [activeSessionId, frame, sessionFrames],
  );
  const [owner, setOwner] = React.useState<"buzz" | "terminal">("buzz");
  const [viewport, setViewport] = React.useState({ columns: 1, rows: 1 });
  const [selectionRows, setSelectionRows] = React.useState<
    readonly TerminalSelectionRow[]
  >([]);
  const [welcomeVisible, setWelcomeVisible] = React.useState(false);
  const [cursorPainted, setCursorPainted] = React.useState(true);
  const [cursorReset, setCursorReset] = React.useState(0);
  const [reducedMotion, setReducedMotion] = React.useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  const [dockHeight, setDockHeight] = React.useState(() => {
    const stored = Number.parseInt(
      window.localStorage.getItem("buzz-terminal-dock-height") ?? "",
      10,
    );
    return Number.isFinite(stored) ? stored : 320;
  });
  const banner = React.useMemo(
    () =>
      buildTerminalBanner(
        viewport.columns,
        viewport.rows,
        CELL_HEIGHT / CELL_WIDTH,
      ),
    [viewport.columns, viewport.rows],
  );
  const terminalStyle = terminalPalette
    ? ({
        "--buzz-terminal-background": terminalPalette.background,
        "--buzz-terminal-foreground": terminalPalette.foreground,
      } as React.CSSProperties)
    : undefined;

  const forceBuzzFallback = React.useEffectEvent(() => {
    handoffRef.current = { ...INITIAL_HANDOFF_STATE };
    setOwner("buzz");
  });

  const sendInput = React.useEffectEvent((text: string) => {
    if (!text) return;
    setWelcomeVisible(false);
    setCursorPainted(true);
    setCursorReset((current) => current + 1);
    onInput(text);
  });
  const consumeFrame = React.useEffectEvent((nextFrame: TerminalFrame) => {
    onFrameConsumed?.(nextFrame);
  });
  const beginSplash = React.useEffectEvent(() => {
    if (!showSplash) return false;
    onSplashStarted?.();
    setWelcomeVisible(true);
    return true;
  });
  /**
   * Tab chords are handled at the window in capture phase, like the ⌘J
   * handoff, so they win over the focused textarea. Gated on terminal
   * ownership: in Buzz mode these keys belong to the rest of the app.
   */
  const runTabChord = React.useEffectEvent((event: KeyboardEvent): boolean => {
    if (owner !== "terminal" || event.isComposing) return false;
    const chord = matchTabChord(event, isMacPlatform());
    if (!chord) return false;
    if (chord === "new") {
      onNewSession();
      return true;
    }
    if (chord === "close") {
      // A tab already closing has a disabled × in the tab bar; re-firing close
      // on it would send a second shutdown for a session on its way out.
      if (!activeSession || activeSession.closing) return true;
      onCloseSession(activeSession.id);
      return true;
    }
    const next = stepSession(sessions, chord === "next" ? 1 : -1);
    if (next) onSelectSession(next);
    return true;
  });
  const reportViewportSize = React.useEffectEvent(() => {
    if (!viewportReportingEnabled) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const bounds = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const pixelWidth = Math.max(1, Math.round(bounds.width * dpr));
    const pixelHeight = Math.max(1, Math.round(bounds.height * dpr));
    const columns = Math.max(1, Math.floor(bounds.width / CELL_WIDTH));
    const rows = Math.max(1, Math.floor(bounds.height / CELL_HEIGHT));
    const size = { columns, rows, pixelWidth, pixelHeight };
    if (resizingRef.current) return;
    setViewport((current) =>
      current.columns === columns && current.rows === rows
        ? current
        : { columns, rows },
    );
    const reported = reportedViewportSizeRef.current;
    if (
      reported?.columns === columns &&
      reported.rows === rows &&
      reported.pixelWidth === pixelWidth &&
      reported.pixelHeight === pixelHeight
    )
      return;
    reportedViewportSizeRef.current = size;
    onViewportSize?.(size);
  });

  React.useEffect(
    () => () => {
      dragCleanupRef.current?.();
      window.cancelAnimationFrame(resizeReportFrameRef.current);
    },
    [],
  );

  React.useEffect(() => {
    if (!enabled) forceBuzzFallback();
  }, [enabled]);

  React.useEffect(() => {
    const preference = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(preference.matches);
    preference.addEventListener("change", update);
    return () => preference.removeEventListener("change", update);
  }, []);

  // Input activity is intentionally an effect trigger: restarting this timer is
  // what resets the blink phase even when the cursor is already painted.
  // biome-ignore lint/correctness/useExhaustiveDependencies: cursorReset restarts the timer by design.
  React.useEffect(() => {
    setCursorPainted(true);
    if (owner !== "terminal" || reducedMotion) return;
    const timer = window.setInterval(
      () => setCursorPainted((painted) => !painted),
      500,
    );
    return () => window.clearInterval(timer);
  }, [cursorReset, owner, reducedMotion]);

  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    reportViewportSize();
    const ResizeObserverConstructor = window.ResizeObserver;
    if (!ResizeObserverConstructor) return;
    const observer = new ResizeObserverConstructor(() => reportViewportSize());
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  React.useLayoutEffect(() => {
    if (viewportReportingEnabled) reportViewportSize();
  }, [viewportReportingEnabled]);

  React.useEffect(() => {
    if (!focusReportingEnabled) {
      reportedFocusRef.current = null;
      return;
    }
    const reportCompositeFocus = () => {
      const focused = owner === "terminal" && document.hasFocus();
      if (reportedFocusRef.current === focused) return;
      reportedFocusRef.current = focused;
      onTerminalFocusChange(focused);
    };
    reportCompositeFocus();
    window.addEventListener("focus", reportCompositeFocus);
    window.addEventListener("blur", reportCompositeFocus);
    return () => {
      window.removeEventListener("focus", reportCompositeFocus);
      window.removeEventListener("blur", reportCompositeFocus);
    };
  }, [focusReportingEnabled, onTerminalFocusChange, owner]);

  React.useLayoutEffect(() => {
    if (!enabled) {
      forceBuzzFallback();
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (runTabChord(event)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      if (!isToggleChord(event) || event.isComposing) return;
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    const handleKeyUp = (event: KeyboardEvent) => {
      if (!isToggleChord(event) || event.isComposing) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (onToggle) onToggle();
      else {
        setOwner((current) => {
          const next = current === "terminal" ? "buzz" : "terminal";
          if (next === "terminal") {
            textareaRef.current?.focus({ preventScroll: true });
          }
          return next;
        });
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("keyup", handleKeyUp, true);
    if (onToggle) {
      setOwner("terminal");
      textareaRef.current?.focus({ preventScroll: true });
    }
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("keyup", handleKeyUp, true);
    };
  }, [enabled, onToggle]);

  React.useEffect(() => {
    if (!visible) {
      setWelcomeVisible(false);
      return;
    }
    if (!showSplash || !viewportReportingEnabled || !banner || !beginSplash())
      return;
  }, [banner, showSplash, viewportReportingEnabled, visible]);

  React.useEffect(() => {
    if (!welcomeVisible) return;
    const timeout = window.setTimeout(
      () => setWelcomeVisible(false),
      SPLASH_DURATION_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [welcomeVisible]);

  // The splash is a bounded decoration, never a PTY-readiness gate. Each open
  // gets one animation epoch; input may dismiss it early and the deadline ends
  // it unconditionally even when an idle shell emits no new frames.
  React.useEffect(() => {
    const canvas = bannerCanvasRef.current;
    if (!canvas || !banner || !terminalPalette || !welcomeVisible || !visible)
      return;
    const dpr = window.devicePixelRatio || 1;
    if (reducedMotion) {
      if (!paintTerminalBanner(canvas, banner, terminalPalette, dpr))
        setWelcomeVisible(false);
      return;
    }

    const table = buildBannerColorTable(terminalPalette);
    const start = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      if (
        !paintTerminalBanner(canvas, banner, terminalPalette, dpr, {
          phase: phaseAt((now - start) / 1000),
          table,
        })
      ) {
        setWelcomeVisible(false);
        return;
      }
      frame = window.requestAnimationFrame(tick);
    };
    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [banner, reducedMotion, terminalPalette, visible, welcomeVisible]);

  const paintTerminal = React.useEffectEvent(() => {
    const canvas = canvasRef.current;
    if (!canvas || !terminalPalette) return;
    const context = canvas.getContext("2d", { alpha: false });
    if (!context) {
      forceBuzzFallback();
      return;
    }
    const dpr = window.devicePixelRatio || 1;
    const bounds = canvas.getBoundingClientRect();
    const pixelWidth = Math.round(bounds.width * dpr);
    const pixelHeight = Math.round(bounds.height * dpr);
    const resized =
      canvas.width !== pixelWidth || canvas.height !== pixelHeight;
    const paletteChanged = paintedPaletteRef.current !== terminalPalette;
    paintedPaletteRef.current = terminalPalette;
    // A grid drains its dirty set in paint(), so a session that was painted and
    // then deactivated comes back holding rows with nothing marked dirty. Both
    // refs are written after the early returns above, so a pass that bails
    // keeps the switch pending instead of swallowing it.
    const sessionChanged = paintedSessionRef.current !== activeSessionId;
    paintedSessionRef.current = activeSessionId;
    const repaintAll = resized || paletteChanged || sessionChanged;
    if (resized) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }
    if (repaintAll) {
      gridRef.current?.markAllDirty();
    }
    context.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (repaintAll) {
      // Not grid-guarded: switching to a session that has delivered no frame
      // yet has no grid to mark or paint, so this fill is the only thing that
      // erases the outgoing session's pixels.
      context.fillStyle = terminalPalette.background;
      context.fillRect(0, 0, bounds.width, bounds.height);
    }
    gridRef.current?.setCursorPainted(cursorPainted);
    gridRef.current?.paint(context, TERMINAL_CELL_METRICS, terminalPalette);
  });

  // Palette and blink changes must trigger a repaint; paintTerminal is an
  // Effect Event, so the dependency analyzer cannot see those reads.
  // biome-ignore lint/correctness/useExhaustiveDependencies: visual-only inputs intentionally trigger this paint effect.
  React.useEffect(() => {
    for (const delivered of frames) {
      if (appliedFramesRef.current.has(delivered.frame)) continue;
      appliedFramesRef.current.add(delivered.frame);
      let grid = gridsRef.current.get(delivered.sessionId);
      if (!grid) {
        grid = new TerminalGrid(delivered.frame.viewport);
        gridsRef.current.set(delivered.sessionId, grid);
      } else if (
        grid.viewport.generation !== delivered.frame.viewport.generation ||
        grid.viewport.columns !== delivered.frame.viewport.columns ||
        grid.viewport.screenLines !== delivered.frame.viewport.screenLines
      ) {
        grid.resize(delivered.frame.viewport);
      }
      grid.apply(delivered.frame);
      consumeFrame(delivered.frame);
    }
    gridRef.current = activeSessionId
      ? (gridsRef.current.get(activeSessionId) ?? null)
      : null;
    setSelectionRows(gridRef.current?.selectionRows() ?? []);

    paintTerminal();
  }, [activeSessionId, cursorPainted, frames, terminalPalette]);

  const runTabAction = (action: () => void) => {
    action();
    if (owner === "terminal") {
      textareaRef.current?.focus({ preventScroll: true });
    }
  };

  return (
    <section
      aria-label="Buzz Term"
      className="buzz-terminal-substrate"
      data-terminal-mode={mode}
      data-terminal-owner={owner}
      data-terminal-visible={visible ? "true" : "false"}
      style={{
        ...terminalStyle,
        ...(mode === "docked" ? { height: dockHeight } : undefined),
      }}
      onWheel={(event) => {
        event.preventDefault();
        const sessionId = activeSession?.id;
        if (!sessionId) return;
        const deltaPx =
          event.deltaMode === 1
            ? event.deltaY * CELL_HEIGHT
            : event.deltaMode === 2
              ? event.deltaY * event.currentTarget.clientHeight
              : event.deltaY;
        const result = accumulateScrollLines(
          { remainderPx: scrollBySessionRef.current.get(sessionId) ?? 0 },
          deltaPx,
          CELL_HEIGHT,
        );
        scrollBySessionRef.current.set(sessionId, result.state.remainderPx);
        if (result.lines !== 0) onScroll(result.lines);
      }}
    >
      {mode === "docked" ? (
        <hr
          aria-label="Resize Buzz Term"
          aria-orientation="horizontal"
          aria-valuemax={Math.round(window.innerHeight * 0.7)}
          aria-valuemin={180}
          aria-valuenow={Math.round(dockHeight)}
          className="buzz-terminal-resize-handle"
          onKeyDown={(event) => {
            if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
            event.preventDefault();
            const delta = event.key === "ArrowUp" ? 16 : -16;
            const next = Math.max(
              180,
              Math.min(window.innerHeight * 0.7, dockHeight + delta),
            );
            setDockHeight(next);
            window.localStorage.setItem(
              "buzz-terminal-dock-height",
              String(Math.round(next)),
            );
          }}
          onPointerDown={(event) => {
            event.preventDefault();
            dragCleanupRef.current?.();
            window.cancelAnimationFrame(resizeReportFrameRef.current);
            const handle = event.currentTarget;
            const substrate = handle.closest<HTMLElement>(
              ".buzz-terminal-substrate",
            );
            if (!substrate) return;
            const pointerId = event.pointerId;
            handle.setPointerCapture(pointerId);
            resizingRef.current = true;
            substrate.dataset.terminalResizing = "true";
            const startY = event.clientY;
            const startHeight = dockHeight;
            let nextHeight = startHeight;
            let frame = 0;
            const applyHeight = () => {
              frame = 0;
              substrate.style.height = `${nextHeight}px`;
              // Repaint the canvas at its new CSS size in the same visual
              // frame. PTY geometry is still reported only on release, but
              // leaving the old backing bitmap in a `height: 100%` canvas
              // makes the browser stretch terminal rows during the drag.
              paintTerminal();
            };
            const cleanup = () => {
              window.cancelAnimationFrame(frame);
              frame = 0;
              handle.removeEventListener("pointermove", move);
              handle.removeEventListener("pointerup", finish);
              handle.removeEventListener("pointercancel", finish);
              if (dragCleanupRef.current === cleanup)
                dragCleanupRef.current = null;
            };
            const move = (moveEvent: PointerEvent) => {
              if (moveEvent.pointerId !== pointerId) return;
              nextHeight = Math.max(
                180,
                Math.min(
                  window.innerHeight * 0.7,
                  startHeight + startY - moveEvent.clientY,
                ),
              );
              if (!frame) frame = window.requestAnimationFrame(applyHeight);
            };
            const finish = (finishEvent: PointerEvent) => {
              if (finishEvent.pointerId !== pointerId) return;
              if (frame) {
                window.cancelAnimationFrame(frame);
                applyHeight();
              }
              cleanup();
              resizingRef.current = false;
              delete substrate.dataset.terminalResizing;
              setDockHeight(nextHeight);
              window.localStorage.setItem(
                "buzz-terminal-dock-height",
                String(Math.round(nextHeight)),
              );
              resizeReportFrameRef.current = window.requestAnimationFrame(
                () => {
                  resizeReportFrameRef.current = 0;
                  if (!resizingRef.current) reportViewportSize();
                },
              );
            };
            dragCleanupRef.current = cleanup;
            handle.addEventListener("pointermove", move);
            handle.addEventListener("pointerup", finish);
            handle.addEventListener("pointercancel", finish);
          }}
          tabIndex={0}
        />
      ) : null}
      <div className="buzz-terminal-contract-bar">
        <div className="buzz-terminal-tabs" role="tablist">
          {sessions.map((session, index) => (
            <div
              className={cn(
                "buzz-terminal-tab",
                session.active && "buzz-terminal-tab-active",
              )}
              key={session.id}
              role="presentation"
            >
              <button
                aria-label={`Close ${session.title}`}
                className="buzz-terminal-close"
                disabled={session.closing}
                onClick={() => runTabAction(() => onCloseSession(session.id))}
                type="button"
              >
                <X />
              </button>
              <button
                aria-label={`Terminal ${index + 1}${session.closing ? ", closing" : session.title !== "SHELL" ? `, ${session.title}` : ""}`}
                aria-selected={session.active}
                className="buzz-terminal-tab-select"
                disabled={session.closing}
                onClick={() => runTabAction(() => onSelectSession(session.id))}
                role="tab"
                type="button"
              >
                <span className="buzz-terminal-designator buzz-terminal-tab-title">
                  {session.title === "SHELL" ? (
                    <>
                      <ChevronRight />
                      <span>{index + 1}</span>
                    </>
                  ) : (
                    session.title
                  )}
                </span>
                {session.closing ? (
                  <span className="buzz-terminal-tab-title">Closing…</span>
                ) : null}
              </button>
            </div>
          ))}
          <button
            aria-label="New Buzz Term tab"
            className="buzz-terminal-new-tab"
            onClick={() => runTabAction(onNewSession)}
            type="button"
          >
            <Plus />
          </button>
        </div>
        <div className="buzz-terminal-readout">
          <button
            aria-label={
              mode === "maximized" ? "Restore Buzz Term" : "Maximize Buzz Term"
            }
            className="buzz-terminal-window-action"
            onClick={() =>
              onModeChange(mode === "maximized" ? "docked" : "maximized")
            }
            type="button"
          >
            {mode === "maximized" ? <Minimize2 /> : <Maximize2 />}
          </button>
          <button
            aria-label="Hide Buzz Term"
            className="buzz-terminal-window-action"
            onClick={onHide}
            type="button"
          >
            <X />
          </button>
        </div>
      </div>
      <div className="buzz-terminal-viewport px-5 pt-2">
        <canvas ref={canvasRef} />
        <div
          aria-hidden="true"
          className="buzz-terminal-selection-layer"
          onCopy={(event) => {
            const selection = window.getSelection();
            const grid = gridRef.current;
            if (!selection || selection.isCollapsed || !grid) return;
            const rowFor = (node: Node | null) =>
              (node?.nodeType === 1
                ? (node as Element)
                : node?.parentElement
              )?.closest<HTMLElement>("[data-terminal-selection-row]");
            const anchorNode = selection.anchorNode;
            const focusNode = selection.focusNode;
            if (!anchorNode || !focusNode) return;
            let startRow = rowFor(anchorNode);
            let endRow = rowFor(focusNode);
            if (!startRow || !endRow) return;
            let startIndex = Number(startRow.dataset.terminalSelectionRow);
            let endIndex = Number(endRow.dataset.terminalSelectionRow);
            const offsetInRow = (
              row: HTMLElement,
              node: Node,
              offset: number,
            ) => {
              const range = document.createRange();
              range.selectNodeContents(row);
              range.setEnd(node, offset);
              return range.toString().length;
            };
            let startOffset = offsetInRow(
              startRow,
              anchorNode,
              selection.anchorOffset,
            );
            let endOffset = offsetInRow(
              endRow,
              focusNode,
              selection.focusOffset,
            );
            if (
              startIndex > endIndex ||
              (startIndex === endIndex && startOffset > endOffset)
            ) {
              [startRow, endRow] = [endRow, startRow];
              [startIndex, endIndex] = [endIndex, startIndex];
              [startOffset, endOffset] = [endOffset, startOffset];
            }
            startOffset = grid.normalizeSelectionOffset(
              startIndex,
              startOffset,
              "start",
            );
            endOffset = grid.normalizeSelectionOffset(
              endIndex,
              endOffset,
              "end",
            );
            event.preventDefault();
            event.clipboardData.setData(
              "text/plain",
              grid.selectionText(startIndex, startOffset, endIndex, endOffset),
            );
          }}
          onMouseUp={() => {
            if (window.getSelection()?.isCollapsed !== false) {
              textareaRef.current?.focus({ preventScroll: true });
            }
          }}
        >
          {selectionRows.map((row) => (
            <div data-terminal-selection-row={row.line} key={row.line}>
              {row.text || "\u00a0"}
            </div>
          ))}
        </div>
        {welcomeVisible && banner ? (
          <canvas className="buzz-terminal-welcome" ref={bannerCanvasRef} />
        ) : null}
        <textarea
          aria-label="Terminal input"
          autoCapitalize="off"
          autoComplete="off"
          className="buzz-terminal-input"
          onCompositionEnd={() => {
            handoffRef.current = reduceHandoff(handoffRef.current, {
              type: "composition-end",
            }).state;
          }}
          onCompositionStart={() => {
            handoffRef.current = reduceHandoff(handoffRef.current, {
              type: "composition-start",
            }).state;
          }}
          onInput={(event) => {
            if (handoffRef.current.composing) return;
            const committed = event.currentTarget.value;
            event.currentTarget.value = "";
            sendInput(committed);
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) {
              event.stopPropagation();
              return;
            }
            if (isToggleChord(event.nativeEvent)) return;
            const encoded = encodeTerminalKey(event);
            if (encoded) {
              event.preventDefault();
              sendInput(encoded);
            }
          }}
          onPaste={(event) => {
            event.preventDefault();
            sendInput(
              encodePaste(
                event.clipboardData.getData("text/plain"),
                bracketedPaste,
              ),
            );
          }}
          ref={textareaRef}
          spellCheck={false}
          tabIndex={0}
        />
      </div>
      <div aria-live="polite" className="sr-only">
        {owner === "terminal" ? "Buzz Term mode" : "Buzz mode"}
      </div>
    </section>
  );
}
