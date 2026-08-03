import * as React from "react";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import { FadeController } from "./fadeController";
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
  appSurfaceRef?: React.RefObject<HTMLDivElement | null>;
  channelName: string | null;
  frame?: TerminalFrame;
  sessionFrames?: readonly { sessionId: string; frame: TerminalFrame }[];
  sessions: readonly TerminalSessionTab[];
  bracketedPaste: boolean;
  focusReportingEnabled: boolean;
  enabled?: boolean;
  onFrameConsumed?: (frame: TerminalFrame) => void;
  onViewportSize?: (size: TerminalViewportSize) => void;
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

function hasVisibleOutput(frame: TerminalFrame): boolean {
  return frame.rows.some((row) =>
    row.spans.some((span) =>
      span.clusters.some((cluster) => cluster.text.trim().length > 0),
    ),
  );
}

export function TerminalSubstrate({
  appSurfaceRef,
  channelName,
  frame,
  sessionFrames,
  sessions,
  bracketedPaste,
  focusReportingEnabled,
  enabled = true,
  onFrameConsumed,
  onViewportSize,
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
  const fadeRef = React.useRef<FadeController | null>(null);
  const handoffRef = React.useRef(INITIAL_HANDOFF_STATE);
  const gridsRef = React.useRef(new Map<string, TerminalGrid>());
  const appliedFramesRef = React.useRef(new WeakSet<TerminalFrame>());
  const gridRef = React.useRef<TerminalGrid | null>(null);
  const paintedPaletteRef = React.useRef(terminalPalette);
  const paintedSessionRef = React.useRef<string | null>(null);
  const previousFocusRef = React.useRef<HTMLElement | null>(null);
  const reportedFocusRef = React.useRef<boolean | null>(null);
  const scrollBySessionRef = React.useRef(new Map<string, number>());
  const revealedRef = React.useRef(false);
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
  const [welcomeVisible, setWelcomeVisible] = React.useState(true);
  const [cursorPainted, setCursorPainted] = React.useState(true);
  const [cursorReset, setCursorReset] = React.useState(0);
  const [reducedMotion, setReducedMotion] = React.useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  const shortcutLabel = /Mac|iPhone|iPad/.test(navigator.platform)
    ? "⌘J"
    : "CTRL+J";
  const getAppSurface = React.useCallback(
    () =>
      appSurfaceRef?.current ??
      document.querySelector<HTMLDivElement>(".buzz-huddle-app-surface"),
    [appSurfaceRef],
  );
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

  const commitOwner = React.useEffectEvent((next: "buzz" | "terminal") => {
    const appSurface = getAppSurface();
    if (!appSurface) return;
    if (next === "terminal") {
      revealedRef.current = true;
      previousFocusRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      appSurface.inert = true;
      appSurface.setAttribute("aria-hidden", "true");
      textareaRef.current?.focus({ preventScroll: true });
    } else {
      appSurface.inert = false;
      appSurface.removeAttribute("aria-hidden");
      const previous = previousFocusRef.current;
      if (previous?.isConnected) previous.focus({ preventScroll: true });
      else appSurface.focus({ preventScroll: true });
    }
    setOwner(next);
  });

  const forceBuzzFallback = React.useEffectEvent(() => {
    handoffRef.current = { ...INITIAL_HANDOFF_STATE };
    commitOwner("buzz");
    fadeRef.current?.settle("conceal");
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
    const canvas = canvasRef.current;
    if (!canvas) return;
    const bounds = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const pixelWidth = Math.max(1, Math.round(bounds.width * dpr));
    const pixelHeight = Math.max(1, Math.round(bounds.height * dpr));
    const columns = Math.max(1, Math.floor(bounds.width / CELL_WIDTH));
    const rows = Math.max(1, Math.floor(bounds.height / CELL_HEIGHT));
    setViewport((current) =>
      current.columns === columns && current.rows === rows
        ? current
        : { columns, rows },
    );
    onViewportSize?.({ columns, rows, pixelWidth, pixelHeight });
  });

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
    const observer = new ResizeObserverConstructor(reportViewportSize);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

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
    const appSurface = getAppSurface();
    if (!appSurface) return;
    fadeRef.current = new FadeController(appSurface);
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!enabled) return;
      if (runTabChord(event)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      if (!isToggleChord(event)) return;
      if (event.isComposing) {
        handoffRef.current = reduceHandoff(handoffRef.current, {
          type: "focus-lost",
        }).state;
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      const result = reduceHandoff(handoffRef.current, {
        type: "chord-down",
        repeat: event.repeat,
      });
      handoffRef.current = result.state;
    };
    const handleKeyUp = (event: KeyboardEvent) => {
      if (!enabled || !isToggleChord(event)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.isComposing) return;
      const result = reduceHandoff(handoffRef.current, { type: "chord-up" });
      handoffRef.current = result.state;
      if (!result.toggled) return;
      commitOwner(result.state.owner);
      fadeRef.current?.toggle(
        window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      );
    };
    const cancelChord = () => {
      handoffRef.current = reduceHandoff(handoffRef.current, {
        type: "focus-lost",
      }).state;
    };
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("keyup", handleKeyUp, true);
    window.addEventListener("blur", cancelChord);
    document.addEventListener("visibilitychange", cancelChord);
    return () => {
      fadeRef.current?.settle("conceal");
      fadeRef.current = null;
      appSurface.inert = false;
      appSurface.removeAttribute("aria-hidden");
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("keyup", handleKeyUp, true);
      window.removeEventListener("blur", cancelChord);
      document.removeEventListener("visibilitychange", cancelChord);
    };
  }, [enabled, getAppSurface]);

  // The banner's animation loop. It runs only while the splash is ON SCREEN,
  // which needs BOTH conditions below — they are different questions:
  //   - `welcomeVisible`: the splash has not been dismissed by terminal output.
  //   - `owner === "terminal"`: the terminal layer is revealed at all.
  //
  // `owner` is the load-bearing one and it is not optional. This substrate is
  // mounted unconditionally by AppShell on every route and merely CSS-concealed
  // in Buzz mode (`.buzz-terminal-substrate` is `position:absolute; inset:0`),
  // and `welcomeVisible` starts `true` and only clears on terminal INPUT. So a
  // loop gated on `welcomeVisible` alone runs forever behind the whole app for
  // anyone who never opens the terminal — measured at 120 rAF/s in the channel
  // view, repainting a canvas nobody can see and slowing every other paint.
  //
  // Deliberately NOT gated on `enabled`: that is `available && Boolean(active)`
  // where `available` is `isTauri()`, and a session is auto-created on channel
  // open (TerminalBootstrap), so `enabled` is true while still concealed in the
  // app and permanently false in the browser — it would gate the tests green
  // and leave real users paying the cost. `owner` is user-gestured in both.
  //
  // `prefers-reduced-motion` takes the STATIC path (no motion argument), which
  // is the shipped painter call, not a paused animation. Those are different:
  // a stopped loop still parks on whatever phase it halted at.
  React.useEffect(() => {
    const canvas = bannerCanvasRef.current;
    if (!canvas || !banner || !terminalPalette || !welcomeVisible) return;
    if (owner !== "terminal") return;

    const dpr = window.devicePixelRatio || 1;
    if (reducedMotion) {
      if (!paintTerminalBanner(canvas, banner, terminalPalette, dpr))
        setWelcomeVisible(false);
      return;
    }

    // Built once per palette, not per frame: the table is phase-independent.
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
  }, [banner, owner, reducedMotion, terminalPalette, welcomeVisible]);

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
      if (
        delivered.sessionId === activeSessionId &&
        revealedRef.current &&
        hasVisibleOutput(delivered.frame)
      ) {
        // Policy: the banner is a splash for the reveal, so spawn-time shell
        // output must not dismiss it. Only visible output from the active PTY
        // that arrives after the terminal has been revealed (or the first
        // keystroke, see sendInput) removes the overlay.
        setWelcomeVisible(false);
      }
    }
    gridRef.current = activeSessionId
      ? (gridsRef.current.get(activeSessionId) ?? null)
      : null;

    const canvas = canvasRef.current;
    if (!canvas) return;
    if (!terminalPalette) {
      forceBuzzFallback();
      return;
    }
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
      data-terminal-owner={owner}
      style={terminalStyle}
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
                aria-selected={session.active}
                className="buzz-terminal-tab-select"
                disabled={session.closing}
                onClick={() => runTabAction(() => onSelectSession(session.id))}
                role="tab"
                type="button"
              >
                <span className="buzz-terminal-designator">
                  SYS.{String(index + 1).padStart(2, "0")}
                </span>
                <span>{session.closing ? "CLOSING" : session.title}</span>
              </button>
              <button
                aria-label={`Close ${session.title}`}
                className="buzz-terminal-close"
                disabled={session.closing}
                onClick={() => runTabAction(() => onCloseSession(session.id))}
                type="button"
              >
                ×
              </button>
            </div>
          ))}
          <button
            aria-label="New Buzz Term tab"
            className="buzz-terminal-new-tab"
            onClick={() => runTabAction(onNewSession)}
            type="button"
          >
            +
          </button>
        </div>
        <div className="buzz-terminal-readout">
          <span>{channelName ? `#${channelName}` : "BUZZ"}</span>
          <span>LOCAL PTY · PRIVATE</span>
          <span>{shortcutLabel} BUZZ</span>
        </div>
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the hidden textarea owns keyboard semantics; this only preserves its focus across canvas clicks. */}
      <div
        className="buzz-terminal-viewport"
        onMouseDown={(event) => {
          if (owner !== "terminal") return;
          // Preventing the canvas mousedown also suppresses selection. Revisit
          // this when the terminal gains mouse selection support.
          event.preventDefault();
          textareaRef.current?.focus({ preventScroll: true });
        }}
      >
        <canvas ref={canvasRef} />
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
          tabIndex={owner === "terminal" ? 0 : -1}
        />
      </div>
      <div aria-live="polite" className="sr-only">
        {owner === "terminal" ? "Buzz Term mode" : "Buzz mode"}
      </div>
    </section>
  );
}
