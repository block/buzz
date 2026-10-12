import * as React from "react";
import { createPortal } from "react-dom";
import { ChevronRight, Maximize2, Minimize2, Plus, X } from "lucide-react";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import {
  INITIAL_HANDOFF_STATE,
  accumulateScrollLines,
  encodePaste,
  encodeTerminalKeystroke,
  matchTabChord,
  reduceHandoff,
  stepSession,
} from "./terminalState";
import { buildTerminalBanner } from "./terminalBanner";
import { paintTerminalBanner } from "./terminalBannerPainter";
import { buildBannerColorTable, phaseAt } from "./terminalBannerWave";
import {
  type CellMetrics,
  type TerminalFrame,
  type TerminalSelectionRow,
  TerminalGrid,
} from "./terminalRenderer";
import {
  type TermFontSize,
  terminalCellMetricsForFontSize,
  useTermPreferences,
} from "./termPreferences.ts";
import { TerminalTouchBar } from "./TerminalTouchBar";
import {
  type StickyMods,
  type TouchAction,
  TouchGestures,
  applyStickyModifiers,
  detectTouchUi,
  fontSizeForPinch,
  getTouchFontSize,
  keyboardInset,
  setTouchFontSize,
  subscribeTouchFontSize,
} from "./touchTerminal";

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
  /** Channel display name for global (all-tabs) provenance labels. */
  channelName?: string | null;
};

type TerminalSubstrateProps = {
  channelName: string | null;
  frame?: TerminalFrame;
  sessionFrames?: readonly { sessionId: string; frame: TerminalFrame }[];
  sessions: readonly TerminalSessionTab[];
  /** When `all`, show channel provenance on each tab. */
  tabScope?: "channel" | "all";
  bracketedPaste: boolean;
  focusReportingEnabled: boolean;
  mouseReportingEnabled?: boolean;
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
  onMouse?: (event: {
    button: "left" | "middle" | "right" | "wheelUp" | "wheelDown";
    action: "press" | "release" | "move";
    column: number;
    row: number;
    shift?: boolean;
    meta?: boolean;
    ctrl?: boolean;
  }) => void;
  onTerminalFocusChange: (focused: boolean) => void;
  onSelectSession: (id: string) => void;
  onCloseSession: (id: string) => void;
  /** New tab; `kind` comes from the + menu when herdr is the Session host ("shell" = plain Terminal, "herdr" = herdr). */
  onNewSession: (kind?: "shell" | "herdr") => void;
  /** Session host is herdr: + opens a small Terminal / herdr menu instead of creating a tab directly. */
  herdrNewTab?: boolean;
  /** False hides the + (web: the desktop's 3-terminal cap is reached). */
  canAddTab?: boolean;
  /** Phone/tablet behaviour (defaults to "the primary pointer is a finger"; desktop mice are unchanged). */
  touchUi?: boolean;
};

/** Zero-width placeholder kept in the hidden input on touch so iOS has something to delete (and keeps repeating backspace). */
const TOUCH_SENTINEL = "\u200b";
const TOUCH_SEED = TOUCH_SENTINEL.repeat(64);

/** Touch scroll gain: 1:1 for slow drags (precise), up to 3x for fast flicks; mouse-tracking TUIs get 2x more (wheel = 1 line). */
function touchScrollGain(speedPxPerMs: number, mouseTracking: boolean): number {
  const accel = 1 + Math.min(2, Math.max(0, speedPxPerMs - 0.3) * 1.5);
  return mouseTracking ? accel * 2 : accel;
}

function isToggleChord(event: KeyboardEvent): boolean {
  return (
    event.code === "KeyJ" &&
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey
  );
}

const NOOP = () => {};
const SPLASH_DURATION_MS = 2_500;

export function TerminalSubstrate({
  frame,
  sessionFrames,
  sessions,
  tabScope = "channel",
  bracketedPaste,
  focusReportingEnabled,
  mouseReportingEnabled = false,
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
  onMouse,
  onTerminalFocusChange,
  onSelectSession,
  onCloseSession,
  onNewSession,
  herdrNewTab = false,
  canAddTab = true,
  touchUi: touchUiProp,
}: TerminalSubstrateProps) {
  const { terminalPalette } = useTheme();
  const termPrefs = useTermPreferences();
  const touchUi = React.useMemo(() => touchUiProp ?? detectTouchUi(), [touchUiProp]);
  const pinchedFont = React.useSyncExternalStore(
    subscribeTouchFontSize,
    getTouchFontSize,
    () => null,
  );
  const effectiveFontSize = (touchUi && pinchedFont ? pinchedFont : termPrefs.fontSize) as TermFontSize;
  const cellMetrics: CellMetrics = React.useMemo(
    () => terminalCellMetricsForFontSize(effectiveFontSize),
    [effectiveFontSize],
  );
  const CELL_WIDTH = cellMetrics.width;
  const CELL_HEIGHT = cellMetrics.height;

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
  const substrateRef = React.useRef<HTMLElement | null>(null);
  const onScrollRef = React.useRef(onScroll);
  onScrollRef.current = onScroll;
  const cellMetricsRef = React.useRef(cellMetrics);
  cellMetricsRef.current = cellMetrics;
  const mouseReportingRef = React.useRef(mouseReportingEnabled);
  mouseReportingRef.current = mouseReportingEnabled;
  const onMouseRef = React.useRef(onMouse);
  onMouseRef.current = onMouse;
  const viewportRef = React.useRef({ columns: 1, rows: 1 });
  const pointerScrollRef = React.useRef({ id: -1, y: 0, remainderPx: 0 });
  const activeSession = sessions.find((session) => session.active);
  const activeSessionId = activeSession?.id ?? null;
  const activeSessionIdForWheelRef = React.useRef<string | null>(null);
  activeSessionIdForWheelRef.current = activeSessionId;
  const ownerRef = React.useRef<"buzz" | "terminal">("buzz");
  /** Set by keydown encode path so onInput does not double-send the same char. */
  const printableKeyHandledRef = React.useRef<string | null>(null);
  const visibleRef = React.useRef(visible);
  visibleRef.current = visible;
  const enabledRef = React.useRef(enabled);
  enabledRef.current = enabled;
  const frames = React.useMemo(
    () =>
      sessionFrames ??
      (activeSessionId && frame ? [{ sessionId: activeSessionId, frame }] : []),
    [activeSessionId, frame, sessionFrames],
  );
  const [owner, setOwner] = React.useState<"buzz" | "terminal">("buzz");
  ownerRef.current = owner;
  const [viewport, setViewport] = React.useState({ columns: 1, rows: 1 });
  viewportRef.current = viewport;
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
  // ── touch ──
  const [mods, setMods] = React.useState<StickyMods>({ ctrl: false, alt: false, meta: false });
  const [selectMode, setSelectMode] = React.useState(false);
  const [newTabMenu, setNewTabMenu] = React.useState<{ left: number; top: number } | null>(null);
  const newTabMenuRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (!newTabMenu) return;
    const away = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (!newTabMenuRef.current?.contains(t) && !t?.closest?.('[data-testid="terminal-new-tab"]')) setNewTabMenu(null);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setNewTabMenu(null);
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", esc, true);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("keydown", esc, true);
    };
  }, [newTabMenu]);
  const selectModeRef = React.useRef(selectMode);
  selectModeRef.current = selectMode;
  const modsRef = React.useRef(mods);
  modsRef.current = mods;
  const gesturesRef = React.useRef(new TouchGestures({ longPressMs: 500 }));
  const longPressTimerRef = React.useRef(0);
  const touchScrollRef = React.useRef({ remainderPx: 0 });
  const touchPointRef = React.useRef({ x: 0, y: 0 });
  const pinchBaseRef = React.useRef<number | null>(null);
  const effectiveFontRef = React.useRef<number>(effectiveFontSize);
  effectiveFontRef.current = effectiveFontSize;
  type CellPos = { row: number; col: number };
  const [touchSel, setTouchSel] = React.useState<{ anchor: CellPos; head: CellPos; done: boolean } | null>(null);
  const touchSelRef = React.useRef(touchSel);
  touchSelRef.current = touchSel;
  const [pasteBox, setPasteBox] = React.useState(false);
  const [kb, setKb] = React.useState({ open: false, top: 0, height: 0, width: 0 });
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

  const lastCursorResetRef = React.useRef(0);
  const sendInput = React.useEffectEvent((text: string) => {
    if (!text) return;
    let out = text;
    if (touchUi && (modsRef.current.ctrl || modsRef.current.alt || modsRef.current.meta)) {
      const applied = applyStickyModifiers(text, modsRef.current);
      out = applied.text;
      if (applied.used) setMods({ ctrl: false, alt: false, meta: false });
    }
    // Send first; the cosmetic state below re-renders this large component, so on touch it is throttled
    // (key repeat would otherwise re-render on every event).
    onInput(out);
    setWelcomeVisible(false);
    setCursorPainted(true);
    const now = performance.now();
    if (!touchUi || now - lastCursorResetRef.current > 250) {
      lastCursorResetRef.current = now;
      setCursorReset((current) => current + 1);
    }
  });
  const cellAt = (clientX: number, clientY: number): CellPos => {
    const canvas = canvasRef.current;
    const metrics = cellMetricsRef.current;
    const vp = viewportRef.current;
    if (!canvas) return { row: 0, col: 0 };
    const bounds = canvas.getBoundingClientRect();
    return {
      col: Math.max(0, Math.min(vp.columns - 1, Math.floor((clientX - bounds.left) / metrics.width))),
      row: Math.max(0, Math.min(vp.rows - 1, Math.floor((clientY - bounds.top) / metrics.height))),
    };
  };
  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // legacy path (older iOS / insecure contexts)
    }
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    try {
      document.execCommand("copy");
    } finally {
      area.remove();
    }
  };
  const pasteFromClipboard = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text) sendInput(encodePaste(text, bracketedPaste));
      setTouchSel(null);
      textareaRef.current?.focus({ preventScroll: true });
    } catch {
      // Safari only reads the clipboard behind its own "Paste" bubble; offer a box to paste into instead.
      setPasteBox(true);
    }
  };
  const sendMouseAt = (cell: CellPos, action: "press" | "release") => {
    onMouseRef.current?.({ button: "left", action, column: cell.col + 1, row: cell.row + 1 });
  };
  const applyTouchActions = (actions: TouchAction[]) => {
    for (const action of actions) {
      switch (action.type) {
        case "tap": {
          setTouchSel(null);
          setPasteBox(false);
          setOwner("terminal");
          textareaRef.current?.focus({ preventScroll: true });
          if (mouseReportingRef.current && onMouseRef.current) {
            const cell = cellAt(action.x, action.y);
            sendMouseAt(cell, "press");
            sendMouseAt(cell, "release");
          }
          break;
        }
        case "scroll": {
          // Touch: track velocity for inertia and scale fast swipes (and mouse-tracking TUIs, where one wheel step is one
          // line) so a single swipe covers many lines. Fractions accumulate; nothing is throttled or dropped.
          let dy = action.dy;
          if (touchUi) {
            const now = performance.now();
            const v = touchVelRef.current;
            const dt = Math.max(1, now - v.t);
            v.v = 0.8 * (dy / dt) + 0.2 * v.v; // px/ms, smoothed
            v.t = now;
            if (!inertiaActiveRef.current) dy *= touchScrollGain(Math.abs(v.v), mouseReportingRef.current);
          }
          const result = accumulateScrollLines(touchScrollRef.current, dy, cellMetricsRef.current.height);
          touchScrollRef.current = result.state;
          if (result.lines === 0) break;
          if (mouseReportingRef.current && onMouseRef.current) {
            const cell = cellAt(touchPointRef.current.x, touchPointRef.current.y);
            for (let i = 0; i < Math.abs(result.lines); i += 1) {
              onMouseRef.current({
                button: result.lines < 0 ? "wheelUp" : "wheelDown",
                action: "press",
                column: cell.col + 1,
                row: cell.row + 1,
              });
            }
          } else {
            onScrollRef.current(result.lines);
          }
          break;
        }
        case "longpress": {
          const cell = cellAt(action.x, action.y);
          // Mouse-tracking apps (Herder, vim, tmux…): a long press is a right-click at that cell; no local selection.
          if (mouseReportingRef.current && onMouseRef.current && !selectModeRef.current) {
            gesturesRef.current.kill();
            onMouseRef.current({ button: "right", action: "press", column: cell.col + 1, row: cell.row + 1 });
            onMouseRef.current({ button: "right", action: "release", column: cell.col + 1, row: cell.row + 1 });
            navigator.vibrate?.(12);
            break;
          }
          const word = gridRef.current?.wordAt(cell.row, cell.col) ?? null;
          setPasteBox(false);
          setTouchSel({
            anchor: { row: cell.row, col: word?.start ?? cell.col },
            head: { row: cell.row, col: word?.end ?? cell.col },
            done: false,
          });
          navigator.vibrate?.(8);
          break;
        }
        case "select": {
          const cell = cellAt(action.x, action.y);
          setTouchSel((c) => (c ? { ...c, head: cell } : c));
          break;
        }
        case "selectEnd":
          setTouchSel((c) => (c ? { ...c, done: true } : c));
          break;
        case "pinch": {
          pinchBaseRef.current ??= effectiveFontRef.current;
          const size = fontSizeForPinch(pinchBaseRef.current, action.scale);
          if (size !== effectiveFontRef.current) setTouchFontSize(size);
          break;
        }
        case "pinchEnd":
          pinchBaseRef.current = null;
          break;
        case "cancel":
          setTouchSel(null);
          break;
      }
    }
  };
  const touchDown = (event: React.PointerEvent) => {
    stopInertia();
    touchVelRef.current = { v: 0, t: performance.now() };
    setOwner("terminal");
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // ignore
    }
    touchPointRef.current = { x: event.clientX, y: event.clientY };
    touchScrollRef.current = { remainderPx: 0 };
    const gestures = gesturesRef.current;
    applyTouchActions(gestures.down(event.pointerId, event.clientX, event.clientY, performance.now()));
    if (selectModeRef.current && gestures.state === "pending") {
      // Select mode: the drag selects rendered cells (works in the alternate screen too); no scroll, no long-press timer.
      gestures.beginSelect();
      const cell = cellAt(event.clientX, event.clientY);
      setTouchSel({ anchor: cell, head: cell, done: false });
      window.clearTimeout(longPressTimerRef.current);
      return;
    }
    window.clearTimeout(longPressTimerRef.current);
    if (gestures.state === "pending") {
      longPressTimerRef.current = window.setTimeout(
        () => applyTouchActions(gestures.timeout(performance.now())),
        gestures.longPressMs,
      );
    }
  };
  const touchVelRef = React.useRef({ v: 0, t: 0 });
  const inertiaRafRef = React.useRef(0);
  const inertiaActiveRef = React.useRef(false);
  const stopInertia = () => {
    window.cancelAnimationFrame(inertiaRafRef.current);
    inertiaRafRef.current = 0;
    inertiaActiveRef.current = false;
  };
  React.useEffect(() => () => window.cancelAnimationFrame(inertiaRafRef.current), []);
  const touchMove = (event: React.PointerEvent) => {
    touchPointRef.current = { x: event.clientX, y: event.clientY };
    const actions = gesturesRef.current.move(event.pointerId, event.clientX, event.clientY, performance.now());
    if (actions.length > 0) window.clearTimeout(longPressTimerRef.current);
    applyTouchActions(actions);
  };
  const touchUp = (event: React.PointerEvent) => {
    window.clearTimeout(longPressTimerRef.current);
    const wasScrolling = gesturesRef.current.state === "scroll";
    applyTouchActions(gesturesRef.current.up(event.pointerId, performance.now()));
    // Inertia: keep scrolling with the release velocity, decaying like native momentum.
    const v0 = touchVelRef.current;
    if (wasScrolling && performance.now() - v0.t < 80 && Math.abs(v0.v) > 0.25) {
      let v = v0.v * touchScrollGain(Math.abs(v0.v), mouseReportingRef.current);
      let last = performance.now();
      inertiaActiveRef.current = true;
      const step = (t: number) => {
        const dt = Math.min(48, t - last);
        last = t;
        v *= Math.pow(0.995, dt); // ~0.92 per 16 ms frame
        if (Math.abs(v) < 0.03) return stopInertia();
        applyTouchActions([{ type: "scroll", dy: v * dt }]);
        inertiaRafRef.current = window.requestAnimationFrame(step);
      };
      inertiaRafRef.current = window.requestAnimationFrame(step);
    }
  };
  const touchCancel = () => {
    window.clearTimeout(longPressTimerRef.current);
    applyTouchActions(gesturesRef.current.cancel());
  };
  const copySelection = async () => {
    const sel = touchSelRef.current;
    const grid = gridRef.current;
    if (!sel || !grid) return;
    const text = grid.textBetweenCells(sel.anchor.row, sel.anchor.col, sel.head.row, sel.head.col);
    if (text) await copyToClipboard(text);
    setTouchSel(null);
    setSelectMode(false); // Copy returns to scroll mode
    textareaRef.current?.focus({ preventScroll: true });
  };
  // Touch: the on-screen keyboard opens only from the keyboard button (toggle). Elsewhere the hidden input stays inputmode="none",
  // so taps/focus reclaims never pop the keyboard (a hardware keyboard still types).
  const [kbWanted, setKbWantedState] = React.useState(false);
  // Synchronous mirror: the toggle decides from this, never from a stale render.
  const kbWantedRef = React.useRef(false);
  const setKbWanted = (on: boolean) => {
    kbWantedRef.current = on;
    setKbWantedState(on);
  };
  /** Until this time (ms) after an intentional open, blur/close detection is ignored (our own blur→focus, the keyboard animating in). */
  const kbOpenGuardRef = React.useRef(0);
  const toggleKeyboard = () => {
    const area = textareaRef.current;
    if (!area) return;
    if (touchUi) {
      if (kbWantedRef.current) {
        kbOpenGuardRef.current = 0;
        setKbWanted(false);
        area.inputMode = "none";
        area.blur();
        return;
      }
      window.dispatchEvent(new Event("buzz:collapse-sidebar"));
      setOwner("terminal");
      kbOpenGuardRef.current = performance.now() + 600;
      setKbWanted(true);
      // All synchronous inside the tap (iOS only shows the keyboard for a focus in the user gesture). Re-focus so the new
      // inputmode takes effect; the blur this causes is inside the guard, so it can't flip the toggle back off.
      area.inputMode = "text";
      if (document.activeElement === area) area.blur();
      area.focus({ preventScroll: true });
      return;
    }
    if (document.activeElement === area) area.blur();
    else {
      setOwner("terminal");
      area.focus({ preventScroll: true });
    }
  };
  const isTouchPointer = (event: React.PointerEvent) => touchUi && event.pointerType !== "mouse";
  /**
   * Reclaim the hidden textarea so printables keep reaching the PTY.
   * `hard` schedules rAF + rAF + setTimeout(0) after an immediate focus —
   * needed after contextmenu / right-click where the browser steals focus
   * asynchronously after our synchronous preventDefault.
   */
  const reclaimTerminalInputFocus = React.useEffectEvent(
    (opts?: { hard?: boolean; claimOwner?: boolean }) => {
      if (opts?.claimOwner) setOwner("terminal");
      else if (ownerRef.current !== "terminal") return;
      const focus = () => textareaRef.current?.focus({ preventScroll: true });
      focus();
      if (!opts?.hard) return;
      window.requestAnimationFrame(() => {
        focus();
        window.requestAnimationFrame(() => {
          focus();
          window.setTimeout(focus, 0);
        });
      });
    },
  );
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

  React.useEffect(() => {
    reportViewportSize();
  }, [cellMetrics.width, cellMetrics.height]);

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

  // Opening from closed paints two frames with data-terminal-visible=false
  // (pointer-events: none). Re-focus once the substrate is interactive so
  // keys reach the PTY instead of the channel composer.
  React.useLayoutEffect(() => {
    if (!enabled || !visible || !onToggle) return;
    setOwner("terminal");
    textareaRef.current?.focus({ preventScroll: true });
  }, [enabled, onToggle, visible]);

  // Reclaim textarea focus after tab switches and mouse-mode CSI toggles while
  // Term still owns input. herdr tracking preventDefault can leave the
  // textarea blurred; do not steal focus after a deliberate outside click.
  React.useLayoutEffect(() => {
    if (!enabled || !visible || owner !== "terminal") return;
    const active = document.activeElement;
    const substrate = substrateRef.current;
    const textarea = textareaRef.current;
    if (!textarea || !substrate) return;
    const insideSubstrate =
      active != null &&
      typeof active === "object" &&
      "nodeType" in active &&
      substrate.contains(active as unknown as globalThis.Node);
    if (
      active === textarea ||
      active === document.body ||
      active === document.documentElement ||
      active == null ||
      insideSubstrate
    ) {
      textarea.focus({ preventScroll: true });
    }
  }, [activeSessionId, enabled, mouseReportingEnabled, owner, visible]);

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
    gridRef.current?.paint(context, cellMetrics, terminalPalette);
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
  }, [activeSessionId, cursorPainted, frames, terminalPalette, cellMetrics]);

  const runTabAction = (action: () => void) => {
    action();
    if (owner === "terminal") {
      textareaRef.current?.focus({ preventScroll: true });
    }
  };

  // iOS keeps the layout viewport full-height when the keyboard opens; follow the visual viewport so the terminal and key bar
  // stay above it.
  React.useEffect(() => {
    if (!touchUi) return;
    const vv = window.visualViewport;
    if (!vv) return;
    // One layout pass per frame while the keyboard animates (no thrash); the canvas refits from the new box and keeps the cursor row.
    let raf = 0;
    const apply = () => {
      raf = 0;
      const next = keyboardInset(vv, window.innerHeight);
      setKb((c) => (c.open === next.open && c.top === next.top && c.height === next.height ? c : next));
      if (next.open && (window.scrollY || document.documentElement.scrollTop)) window.scrollTo(0, 0);
    };
    const update = () => {
      if (!raf) raf = window.requestAnimationFrame(apply);
    };
    apply();
    vv.addEventListener("resize", update);
    vv.addEventListener("scroll", update);
    return () => {
      window.cancelAnimationFrame(raf);
      vv.removeEventListener("resize", update);
      vv.removeEventListener("scroll", update);
    };
  }, [touchUi]);
  React.useEffect(() => {
    if (touchUi) setTouchSel(null);
  }, [touchUi, activeSessionId]);

  // Touch: iOS auto-repeats a held backspace only as beforeinput delete events (no keydown repeat) and only while the input has
  // something to delete; after a while it accelerates to word deletes. Each event goes to the PTY at once: one DEL per char
  // repeat, one ^W per word repeat (same as native). No per-event DOM work: the cancelled event leaves the 64-char sentinel
  // buffer untouched, so it's only reseeded on focus or after real typing.
  React.useEffect(() => {
    const area = textareaRef.current;
    if (!touchUi || !area) return;
    const seed = () => {
      if (area.value !== TOUCH_SEED) {
        area.value = TOUCH_SEED;
        area.setSelectionRange(TOUCH_SEED.length, TOUCH_SEED.length);
      }
    };
    const onBeforeInput = (event: InputEvent) => {
      const seq = event.inputType === "deleteContentBackward" ? "\x7f" : event.inputType === "deleteWordBackward" ? "\x17" : null;
      if (!seq) return;
      event.preventDefault();
      sendInput(seq);
    };
    area.addEventListener("beforeinput", onBeforeInput);
    area.addEventListener("focus", seed);
    seed();
    return () => {
      area.removeEventListener("beforeinput", onBeforeInput);
      area.removeEventListener("focus", seed);
    };
  }, [touchUi]);

  // Touch (iPad/iPhone Safari, PWA): size from the real visible area instead of trusting flex/100vh, which left maximized at
  // ~3/4 height. Maximized pins to the content surface's box down to the visual viewport's bottom; docked keeps its height within
  // the real height. Recomputed on viewport/orientation/surface resize. Desktop (non-touch) is untouched.
  const [touchBox, setTouchBox] = React.useState<{ top: number; left: number; width: number; bottom: number; vh: number } | null>(null);
  React.useEffect(() => {
    if (!touchUi) return;
    const surface = substrateRef.current?.closest("[data-buzz-content-surface]") as HTMLElement | null;
    const vv = window.visualViewport;
    const update = () => {
      const vh = vv ? vv.height + vv.offsetTop : window.innerHeight;
      const r = surface?.getBoundingClientRect();
      const next = r
        ? { top: Math.round(r.top), left: Math.round(r.left), width: Math.round(r.width), bottom: Math.max(0, Math.round(window.innerHeight - vh)), vh: Math.round(vh) }
        : null;
      setTouchBox((c) => (c && next && c.top === next.top && c.left === next.left && c.width === next.width && c.bottom === next.bottom && c.vh === next.vh ? c : next));
    };
    update();
    const ro = surface ? new ResizeObserver(update) : null;
    if (surface) ro?.observe(surface);
    vv?.addEventListener("resize", update);
    window.addEventListener("resize", update);
    window.addEventListener("orientationchange", update);
    return () => {
      ro?.disconnect();
      vv?.removeEventListener("resize", update);
      window.removeEventListener("resize", update);
      window.removeEventListener("orientationchange", update);
    };
  }, [touchUi, mode]);

  // Native non-passive wheel: React's delegated onWheel is passive in the
  // browsers we ship, so preventDefault there cannot stop the outer UI from
  // stealing the gesture. Match AppTopChrome / useScrollBoundaryLock.
  // Mouse tracking on (herdr / xterm CSI) → wheel to PTY; off → scrollback.
  // Never gate on textarea focus — a wrong focus must not drop the gesture.
  React.useEffect(() => {
    const node = substrateRef.current;
    if (!node) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const sessionId = activeSessionIdForWheelRef.current;
      if (!sessionId) {
        reclaimTerminalInputFocus();
        return;
      }
      const cellHeight = cellMetricsRef.current.height;
      const cellWidth = cellMetricsRef.current.width;
      const deltaPx =
        event.deltaMode === 1
          ? event.deltaY * cellHeight
          : event.deltaMode === 2
            ? event.deltaY * node.clientHeight
            : event.deltaY;
      const result = accumulateScrollLines(
        { remainderPx: scrollBySessionRef.current.get(sessionId) ?? 0 },
        deltaPx,
        cellHeight,
      );
      scrollBySessionRef.current.set(sessionId, result.state.remainderPx);
      if (result.lines !== 0) {
        if (mouseReportingRef.current && onMouseRef.current) {
          const canvas = canvasRef.current;
          const vp = viewportRef.current;
          let column = 1;
          let row = 1;
          if (canvas) {
            const bounds = canvas.getBoundingClientRect();
            column = Math.max(
              1,
              Math.min(
                vp.columns,
                Math.floor((event.clientX - bounds.left) / cellWidth) + 1,
              ),
            );
            row = Math.max(
              1,
              Math.min(
                vp.rows,
                Math.floor((event.clientY - bounds.top) / cellHeight) + 1,
              ),
            );
          }
          const button = result.lines < 0 ? "wheelUp" : "wheelDown";
          const count = Math.abs(result.lines);
          for (let i = 0; i < count; i += 1) {
            onMouseRef.current({
              button,
              action: "press",
              column,
              row,
              shift: event.shiftKey,
              meta: event.metaKey,
              ctrl: event.ctrlKey,
            });
          }
        } else {
          onScrollRef.current(result.lines);
        }
      }
      // herdr mouse-mode CSI / prior preventDefault presses often leave
      // activeElement on body. Letters only reach the PTY via textarea `input`
      // (Backspace uses keydown encode) — reclaim after every wheel gesture.
      reclaimTerminalInputFocus();
    };
    const options: AddEventListenerOptions = { capture: true, passive: false };
    node.addEventListener("wheel", onWheel, options);
    return () => node.removeEventListener("wheel", onWheel, options);
  }, []);

  // Belt-and-suspenders: while Term owns input, window capture-phase keydown
  // forwards printables + specials to the PTY whenever focus is the hidden
  // textarea, body/html, or anything inside the substrate. Do not rely on
  // textarea `input` alone — after right-click / herdr CSI, activeElement is
  // often body and letters never insert. Skip real INPUT/TEXTAREA outside Term
  // (channel composer). Must be on window — body-focused keys never traverse
  // the substrate.
  React.useEffect(() => {
    const onKeyDownCapture = (event: KeyboardEvent) => {
      if (
        !enabledRef.current ||
        !visibleRef.current ||
        ownerRef.current !== "terminal" ||
        event.isComposing ||
        event.defaultPrevented
      ) {
        return;
      }
      if (isToggleChord(event)) return;
      if (matchTabChord(event, isMacPlatform())) return;

      const textarea = textareaRef.current;
      const substrate = substrateRef.current;
      if (!textarea || !substrate) return;
      const active = document.activeElement;
      const onTerminalTextarea = active === textarea;
      const insideSubstrate =
        active != null &&
        typeof active === "object" &&
        "nodeType" in active &&
        substrate.contains(active as unknown as globalThis.Node);
      const focusLost =
        active == null ||
        active === document.body ||
        active === document.documentElement;
      // Composer / other app fields outside Term keep their keys.
      if (
        !onTerminalTextarea &&
        !focusLost &&
        !insideSubstrate &&
        active instanceof HTMLElement &&
        (active.tagName === "INPUT" ||
          active.tagName === "TEXTAREA" ||
          active.isContentEditable)
      ) {
        return;
      }
      if (!onTerminalTextarea && !focusLost && !insideSubstrate) {
        return;
      }

      const encoded = encodeTerminalKeystroke(event);
      if (!encoded) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      printableKeyHandledRef.current = encoded;
      textarea.focus({ preventScroll: true });
      sendInput(encoded);
    };
    window.addEventListener("keydown", onKeyDownCapture, true);
    return () => window.removeEventListener("keydown", onKeyDownCapture, true);
  }, []);

  return (
    <section
      ref={substrateRef}
      aria-label="Buzz Term"
      className="buzz-terminal-substrate"
      data-terminal-mode={mode}
      data-terminal-owner={owner}
      data-terminal-visible={visible ? "true" : "false"}
      data-touch-ui={touchUi ? "true" : undefined}
      data-keyboard-open={touchUi && kb.open ? "true" : undefined}
      style={{
        ...terminalStyle,
        ...(mode === "docked"
          ? { height: touchUi && touchBox ? Math.min(dockHeight, Math.round(touchBox.vh * 0.7)) : dockHeight }
          : undefined),
        ...(touchUi && touchBox && mode === "maximized" && visible && !kb.open
          ? {
              position: "fixed" as const,
              inset: "auto",
              top: touchBox.top,
              left: touchBox.left,
              width: touchBox.width,
              bottom: touchBox.bottom,
              height: "auto",
              paddingBottom: "env(safe-area-inset-bottom)",
              zIndex: 30,
            }
          : undefined),
        ...(touchUi && kb.open
          ? { position: "fixed" as const, inset: "auto", left: 0, right: 0, top: kb.top, height: kb.height, zIndex: 60 }
          : undefined),
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
                aria-label={`Terminal ${index + 1}${
                  tabScope === "all" && session.channelName
                    ? `, ${session.channelName}`
                    : ""
                }${session.closing ? ", closing" : session.title !== "SHELL" ? `, ${session.title}` : ""}`}
                aria-selected={session.active}
                className="buzz-terminal-tab-select"
                disabled={session.closing}
                onClick={() => runTabAction(() => onSelectSession(session.id))}
                role="tab"
                type="button"
              >
                <span className="buzz-terminal-designator buzz-terminal-tab-title">
                  {tabScope === "all" && session.channelName ? (
                    <span className="buzz-terminal-tab-channel">
                      {session.channelName}
                    </span>
                  ) : null}
                  {tabScope === "all" && session.channelName ? (
                    <span className="buzz-terminal-tab-sep"> — </span>
                  ) : null}
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
          {canAddTab ? (
          <button
            aria-label="New Buzz Term tab"
            aria-haspopup={herdrNewTab ? "menu" : undefined}
            aria-expanded={herdrNewTab ? newTabMenu !== null : undefined}
            className="buzz-terminal-new-tab"
            data-testid="terminal-new-tab"
            onClick={(event) => {
              if (!herdrNewTab) return runTabAction(() => onNewSession());
              const r = event.currentTarget.getBoundingClientRect();
              setNewTabMenu((m) => (m ? null : { left: r.left, top: r.bottom + 4 }));
            }}
            type="button"
          >
            <Plus />
          </button>
          ) : null}
          {newTabMenu
            ? createPortal(
                <div
                  className="buzz-terminal-newtab-menu"
                  data-testid="terminal-new-tab-menu"
                  ref={newTabMenuRef}
                  role="menu"
                  style={{ left: Math.max(4, Math.min(newTabMenu.left, window.innerWidth - 164)), top: newTabMenu.top }}
                >
                  {(
                    [
                      ["shell", "Terminal"],
                      ["herdr", "herdr"],
                    ] as const
                  ).map(([kind, label]) => (
                    <button
                      data-testid={`terminal-new-tab-${kind}`}
                      key={kind}
                      onClick={() => {
                        setNewTabMenu(null);
                        runTabAction(() => onNewSession(kind));
                      }}
                      role="menuitem"
                      type="button"
                    >
                      {label}
                    </button>
                  ))}
                </div>,
                document.body,
              )
            : null}
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
      <div
        className="buzz-terminal-viewport px-5 pt-2"
        data-testid="buzz-terminal-viewport"
        onPointerDown={(event) => {
          if (isTouchPointer(event)) {
            event.preventDefault();
            touchDown(event);
            return;
          }
          if (event.button > 2) return;
          // Always claim keyboard focus on viewport interaction — even before
          // the canvas exists. herdr (and other mouse-mode TUIs) call
          // preventDefault below, which would otherwise leave keys going to
          // the channel composer.
          setOwner("terminal");
          textareaRef.current?.focus({ preventScroll: true });
          const canvas = canvasRef.current;
          if (!canvas) return;
          const bounds = canvas.getBoundingClientRect();
          const metrics = cellMetricsRef.current;
          const column = Math.max(
            1,
            Math.min(
              viewport.columns,
              Math.floor((event.clientX - bounds.left) / metrics.width) + 1,
            ),
          );
          const row = Math.max(
            1,
            Math.min(
              viewport.rows,
              Math.floor((event.clientY - bounds.top) / metrics.height) + 1,
            ),
          );
          if (mouseReportingRef.current && onMouseRef.current) {
            const button =
              event.button === 1
                ? "middle"
                : event.button === 2
                  ? "right"
                  : "left";
            // Forward all buttons (incl. right) so herdr / xterm mouse-mode
            // CSI context menus work. Browser native contextmenu stays
            // suppressed in onContextMenu; hard focus reclaim on release
            // keeps printables working after the herdr menu closes.
            event.preventDefault();
            try {
              canvas.setPointerCapture(event.pointerId);
            } catch {
              // ignore
            }
            onMouseRef.current({
              button,
              action: "press",
              column,
              row,
              shift: event.shiftKey,
              meta: event.metaKey,
              ctrl: event.ctrlKey,
            });
            return;
          }
          // Touch / pen pan → scrollback when mouse mode is off.
          if (event.pointerType === "touch" || event.pointerType === "pen") {
            event.preventDefault();
            pointerScrollRef.current = {
              id: event.pointerId,
              y: event.clientY,
              remainderPx: 0,
            };
            try {
              canvas.setPointerCapture(event.pointerId);
            } catch {
              // ignore
            }
          }
        }}
        onPointerMove={(event) => {
          if (isTouchPointer(event)) {
            touchMove(event);
            return;
          }
          const canvas = canvasRef.current;
          if (!canvas) return;
          const bounds = canvas.getBoundingClientRect();
          const metrics = cellMetricsRef.current;
          const column = Math.max(
            1,
            Math.min(
              viewport.columns,
              Math.floor((event.clientX - bounds.left) / metrics.width) + 1,
            ),
          );
          const row = Math.max(
            1,
            Math.min(
              viewport.rows,
              Math.floor((event.clientY - bounds.top) / metrics.height) + 1,
            ),
          );
          if (
            mouseReportingRef.current &&
            onMouseRef.current &&
            event.buttons !== 0
          ) {
            event.preventDefault();
            const button =
              event.buttons === 4
                ? "middle"
                : event.buttons === 2
                  ? "right"
                  : "left";
            onMouseRef.current({
              button,
              action: "move",
              column,
              row,
              shift: event.shiftKey,
              meta: event.metaKey,
              ctrl: event.ctrlKey,
            });
            return;
          }
          const tracked = pointerScrollRef.current;
          if (tracked.id !== event.pointerId) return;
          event.preventDefault();
          const deltaY = tracked.y - event.clientY;
          tracked.y = event.clientY;
          const result = accumulateScrollLines(
            { remainderPx: tracked.remainderPx },
            deltaY,
            metrics.height,
          );
          tracked.remainderPx = result.state.remainderPx;
          if (result.lines !== 0) onScrollRef.current(result.lines);
        }}
        onPointerUp={(event) => {
          if (isTouchPointer(event)) {
            touchUp(event);
            return;
          }
          const canvas = canvasRef.current;
          const metrics = cellMetricsRef.current;
          if (mouseReportingRef.current && onMouseRef.current && canvas) {
            const button =
              event.button === 1
                ? "middle"
                : event.button === 2
                  ? "right"
                  : "left";
            const bounds = canvas.getBoundingClientRect();
            const column = Math.max(
              1,
              Math.min(
                viewport.columns,
                Math.floor((event.clientX - bounds.left) / metrics.width) + 1,
              ),
            );
            const row = Math.max(
              1,
              Math.min(
                viewport.rows,
                Math.floor((event.clientY - bounds.top) / metrics.height) + 1,
              ),
            );
            onMouseRef.current({
              button,
              action: "release",
              column,
              row,
              shift: event.shiftKey,
              meta: event.metaKey,
              ctrl: event.ctrlKey,
            });
          }
          if (pointerScrollRef.current.id === event.pointerId) {
            pointerScrollRef.current.id = -1;
          }
          // Mouse-mode CSI / right-click preventDefault often leave the
          // hidden textarea blurred; reclaim so printable keys keep working.
          reclaimTerminalInputFocus(
            event.button === 2 ? { hard: true, claimOwner: true } : undefined,
          );
        }}
        onPointerCancel={(event) => {
          if (isTouchPointer(event)) {
            touchCancel();
            return;
          }
          if (pointerScrollRef.current.id === event.pointerId) {
            pointerScrollRef.current.id = -1;
          }
          reclaimTerminalInputFocus();
        }}
        onAuxClick={(event) => {
          // Non-primary buttons (esp. right) fire auxclick; reclaim before
          // contextmenu can leave focus on body / chrome.
          if (event.button !== 2) return;
          event.preventDefault();
          reclaimTerminalInputFocus({ hard: true, claimOwner: true });
        }}
        onContextMenu={(event) => {
          if (touchUi) {
            event.preventDefault();
            return;
          }
          // No custom Term menu — always suppress the browser menu. Native
          // contextmenu after right-click steals focus from the hidden
          // textarea asynchronously; hard reclaim races that steal.
          event.preventDefault();
          reclaimTerminalInputFocus({ hard: true, claimOwner: true });
        }}
        style={{ touchAction: "none" }}
      >
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
        {touchUi && touchSel ? (
          <TouchSelectionOverlay
            sel={touchSel}
            metrics={cellMetrics}
            columns={viewport.columns}
            canvas={canvasRef.current}
            onCopy={() => void copySelection()}
            onPaste={() => void pasteFromClipboard()}
            onDone={() => setTouchSel(null)}
          />
        ) : null}
        {touchUi && pasteBox ? (
          <textarea
            aria-label="Paste here"
            autoFocus
            className="buzz-terminal-pastebox"
            data-testid="terminal-pastebox"
            placeholder="Long-press, Paste, then tap away"
            onPaste={(event) => {
              event.preventDefault();
              sendInput(encodePaste(event.clipboardData.getData("text/plain"), bracketedPaste));
              setPasteBox(false);
              textareaRef.current?.focus({ preventScroll: true });
            }}
            onBlur={() => setPasteBox(false)}
          />
        ) : null}
        {welcomeVisible && banner ? (
          <canvas className="buzz-terminal-welcome" ref={bannerCanvasRef} />
        ) : null}
        <textarea
          aria-label="Terminal input"
          autoCapitalize="off"
          autoComplete="off"
          className="buzz-terminal-input"
          inputMode={touchUi && !kbWanted ? "none" : undefined}
          onBlur={() => {
            // iOS "Done"/swipe-down dismissed the keyboard: back to tap-safe mode (not our own re-focus right after an open)
            if (touchUi && kbWantedRef.current && performance.now() >= kbOpenGuardRef.current) {
              setKbWanted(false);
              if (textareaRef.current) textareaRef.current.inputMode = "none";
            }
          }}
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
            const raw = event.currentTarget.value;
            // touch: the sentinel isn't user text; if it vanished without a cancelable beforeinput, that was a delete
            const committed = touchUi ? raw.replaceAll(TOUCH_SENTINEL, "") : raw;
            if (touchUi && !raw.includes(TOUCH_SENTINEL) && !committed) sendInput("\x7f");
            event.currentTarget.value = touchUi ? TOUCH_SEED : "";
            if (!committed) return;
            // Capture/textarea keydown already forwarded this printable.
            if (printableKeyHandledRef.current === committed) {
              printableKeyHandledRef.current = null;
              return;
            }
            sendInput(committed);
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) {
              event.stopPropagation();
              return;
            }
            if (isToggleChord(event.nativeEvent)) return;
            // Prefer encodeTerminalKeystroke so letters still reach the PTY if
            // window capture missed (focus quirks). preventDefault stops the
            // subsequent `input` insertion; the handled-ref guards any race.
            const encoded = encodeTerminalKeystroke(event);
            if (encoded) {
              event.preventDefault();
              printableKeyHandledRef.current = encoded;
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
      {touchUi ? (
        <TerminalTouchBar
          ctrl={mods.ctrl}
          alt={mods.alt}
          meta={!!mods.meta}
          onToggleCtrl={() => setMods((m) => ({ ...m, ctrl: !m.ctrl }))}
          onToggleAlt={() => setMods((m) => ({ ...m, alt: !m.alt }))}
          onToggleMeta={() => setMods((m) => ({ ...m, meta: !m.meta }))}
          selectMode={selectMode}
          onToggleSelect={() => {
            setSelectMode((on) => !on);
            setTouchSel(null);
          }}
          onKey={(seq) => sendInput(seq)}
          onPaste={() => void pasteFromClipboard()}
          keyboardOpen={kb.open}
          onToggleKeyboard={toggleKeyboard}
        />
      ) : null}
      <div aria-live="polite" className="sr-only">
        {owner === "terminal" ? "Buzz Term mode" : "Buzz mode"}
      </div>
    </section>
  );
}

function TouchSelectionOverlay({
  sel,
  metrics,
  columns,
  canvas,
  onCopy,
  onPaste,
  onDone,
}: {
  sel: { anchor: { row: number; col: number }; head: { row: number; col: number }; done: boolean };
  metrics: CellMetrics;
  columns: number;
  canvas: HTMLCanvasElement | null;
  onCopy: () => void;
  onPaste: () => void;
  onDone: () => void;
}) {
  const [a, b] =
    sel.anchor.row < sel.head.row || (sel.anchor.row === sel.head.row && sel.anchor.col <= sel.head.col)
      ? [sel.anchor, sel.head]
      : [sel.head, sel.anchor];
  const left = canvas?.offsetLeft ?? 0;
  const top = canvas?.offsetTop ?? 0;
  const rects: React.ReactNode[] = [];
  for (let row = a.row; row <= b.row; row += 1) {
    const c0 = row === a.row ? a.col : 0;
    const c1 = row === b.row ? b.col : columns - 1;
    rects.push(
      <div
        key={row}
        className="buzz-terminal-touch-sel"
        style={{
          left: left + c0 * metrics.width,
          top: top + row * metrics.height,
          width: (c1 - c0 + 1) * metrics.width,
          height: metrics.height,
        }}
      />,
    );
  }
  return (
    <>
      <div aria-hidden="true" data-testid="terminal-touch-selection" style={{ pointerEvents: "none" }}>
        {rects}
      </div>
      {sel.done ? (
        <div
          className="buzz-terminal-touch-menu"
          data-testid="terminal-touch-menu"
          style={{ left: Math.max(8, left + a.col * metrics.width), top: Math.max(4, top + a.row * metrics.height - 44) }}
          onPointerDown={(e) => e.stopPropagation()}
          onPointerUp={(e) => e.stopPropagation()}
        >
          <button type="button" onClick={onCopy}>Copy</button>
          <button type="button" onClick={onPaste}>Paste</button>
          <button type="button" onClick={onDone}>Done</button>
        </div>
      ) : null}
    </>
  );
}
