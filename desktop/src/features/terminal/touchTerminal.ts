// Touch support for Buzz Term on phones/tablets (the web build on iOS). Pure logic, no React/DOM: the substrate feeds pointer
// events in and acts on what comes out, which keeps the gesture rules testable.
//
//   one finger  tap          → click (mouse-tracking apps such as herdr) / just focus the keyboard
//               drag         → scroll (history, or wheel events for mouse-tracking apps)
//               long press   → selection mode; drag extends; release shows Copy / Paste
//   two fingers pinch        → font size
//   key bar     Esc Tab ←↑↓→ plus sticky Ctrl / Alt that apply to the next typed key

export type TouchAction =
  | { type: "tap"; x: number; y: number }
  | { type: "scroll"; dy: number }
  | { type: "longpress"; x: number; y: number }
  | { type: "select"; x: number; y: number }
  | { type: "selectEnd"; x: number; y: number }
  | { type: "pinch"; scale: number }
  | { type: "pinchEnd" }
  | { type: "cancel" };

type Mode = "idle" | "pending" | "scroll" | "select" | "pinch" | "dead";
type Pt = { x: number; y: number };

export type TouchGestureOptions = {
  /** Finger travel (px) before a touch stops being a tap/long-press and becomes a scroll. */
  slopPx?: number;
  longPressMs?: number;
  tapMaxMs?: number;
};

export class TouchGestures {
  private mode: Mode = "idle";
  private pts = new Map<number, Pt>();
  private startPt: Pt = { x: 0, y: 0 };
  private lastY = 0;
  private downAt = 0;
  private pinchStart = 0;
  private slop: number;
  private longMs: number;
  private tapMs: number;

  constructor(o: TouchGestureOptions = {}) {
    this.slop = o.slopPx ?? 8;
    this.longMs = o.longPressMs ?? 450;
    this.tapMs = o.tapMaxMs ?? 400;
  }

  get state(): Mode {
    return this.mode;
  }
  get longPressMs(): number {
    return this.longMs;
  }

  private dist(): number {
    const [a, b] = [...this.pts.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  }

  down(id: number, x: number, y: number, t: number): TouchAction[] {
    this.pts.set(id, { x, y });
    if (this.pts.size === 1) {
      this.mode = "pending";
      this.startPt = { x, y };
      this.lastY = y;
      this.downAt = t;
      return [];
    }
    if (this.pts.size === 2) {
      const wasSelecting = this.mode === "select";
      this.mode = "pinch";
      this.pinchStart = this.dist() || 1;
      return wasSelecting ? [{ type: "cancel" }] : [];
    }
    return [];
  }

  move(id: number, x: number, y: number, _t: number): TouchAction[] {
    if (!this.pts.has(id)) return [];
    this.pts.set(id, { x, y });
    if (this.mode === "pinch") {
      return this.pinchStart > 0 ? [{ type: "pinch", scale: this.dist() / this.pinchStart }] : [];
    }
    if (this.mode === "select") return [{ type: "select", x, y }];
    if (this.mode === "pending") {
      if (Math.hypot(x - this.startPt.x, y - this.startPt.y) < this.slop) return [];
      this.mode = "scroll";
      const dy = this.startPt.y - y;
      this.lastY = y;
      return dy ? [{ type: "scroll", dy }] : [];
    }
    if (this.mode === "scroll") {
      const dy = this.lastY - y;
      this.lastY = y;
      return dy ? [{ type: "scroll", dy }] : [];
    }
    return [];
  }

  /** Call when the long-press timer fires. */
  timeout(t: number): TouchAction[] {
    if (this.mode !== "pending" || t - this.downAt < this.longMs - 1) return [];
    this.mode = "select";
    return [{ type: "longpress", x: this.startPt.x, y: this.startPt.y }];
  }

  up(id: number, t: number): TouchAction[] {
    const p = this.pts.get(id);
    this.pts.delete(id);
    const mode = this.mode;
    if (this.pts.size === 0) this.mode = "idle";
    else if (mode === "pinch") this.mode = "dead"; // the remaining finger must not start a scroll/tap
    if (!p) return [];
    if (mode === "pinch") return [{ type: "pinchEnd" }];
    if (mode === "select") return [{ type: "selectEnd", x: p.x, y: p.y }];
    if (mode === "pending" && t - this.downAt <= this.tapMs) return [{ type: "tap", x: p.x, y: p.y }];
    return [];
  }

  /** Select mode (key-bar Select toggle): the current one-finger touch selects from where it went down instead of scrolling. */
  beginSelect(): void {
    if (this.pts.size === 1) this.mode = "select";
  }
  /** Ignore the rest of the current touch (e.g. after a long-press right-click). */
  kill(): void {
    if (this.pts.size > 0) this.mode = "dead";
  }

  cancel(): TouchAction[] {
    const was = this.mode;
    this.pts.clear();
    this.mode = "idle";
    return was === "select" || was === "pinch" ? [{ type: "cancel" }] : [];
  }
}

// ── on-screen key bar ───────────────────────────────────────────────────────────────────────
export const TOUCH_KEYS = {
  esc: "\u001b",
  tab: "\t",
  up: "\u001b[A",
  down: "\u001b[B",
  right: "\u001b[C",
  left: "\u001b[D",
  ctrlC: "\u0003",
} as const;

/** `meta` (Cmd) is sent like Alt/Option (ESC prefix), the usual "Meta sends Escape" terminal convention. */
export type StickyMods = { ctrl: boolean; alt: boolean; meta?: boolean };

/**
 * Apply armed Ctrl/Alt to what the keyboard just produced. Ctrl maps a single letter (or one of @ [ \ ] ^ _ / space) to its
 * control code; Alt prefixes ESC. `used` says whether the armed modifiers were spent (an unmappable key leaves Ctrl armed).
 */
export function applyStickyModifiers(text: string, mods: StickyMods): { text: string; used: boolean } {
  if (!text || (!mods.ctrl && !mods.alt && !mods.meta)) return { text, used: false };
  let out = text;
  let used = false;
  if (mods.ctrl) {
    const first = [...out][0] ?? "";
    if (first.length === 1 && [...out].length === 1) {
      if (first === " ") {
        out = "\u0000";
        used = true;
      } else if (first === "?") {
        out = "\u007f";
        used = true;
      } else {
        const code = first.toUpperCase().charCodeAt(0);
        if (code >= 64 && code <= 95) {
          out = String.fromCharCode(code - 64);
          used = true;
        }
      }
    }
  }
  if (mods.alt || mods.meta) {
    out = `\u001b${out}`;
    used = true;
  }
  return { text: out, used };
}

// ── pinch zoom ──────────────────────────────────────────────────────────────────────────────
export const TOUCH_FONT_MIN = 8;
export const TOUCH_FONT_MAX = 28;
export function fontSizeForPinch(startSize: number, scale: number): number {
  if (!Number.isFinite(scale) || scale <= 0) return startSize;
  return Math.min(TOUCH_FONT_MAX, Math.max(TOUCH_FONT_MIN, Math.round(startSize * scale)));
}

const FONT_KEY = "buzz.term.touchFontSize";
let touchFont: number | null = readFont();
const fontSubs = new Set<() => void>();
function readFont(): number | null {
  try {
    const n = Number.parseInt(globalThis.localStorage?.getItem(FONT_KEY) ?? "", 10);
    return Number.isFinite(n) && n >= TOUCH_FONT_MIN && n <= TOUCH_FONT_MAX ? n : null;
  } catch {
    return null;
  }
}
/** The font size chosen by pinching on this device (null = use Settings → Buzz Term → Font size). */
export const getTouchFontSize = (): number | null => touchFont;
export function setTouchFontSize(n: number | null) {
  touchFont = n;
  try {
    if (n === null) globalThis.localStorage?.removeItem(FONT_KEY);
    else globalThis.localStorage?.setItem(FONT_KEY, String(n));
  } catch {
    // best effort
  }
  for (const f of fontSubs) f();
}
export const subscribeTouchFontSize = (f: () => void) => {
  fontSubs.add(f);
  return () => void fontSubs.delete(f);
};

/** Phones and tablets: the primary pointer is a finger. (A Mac with a mouse stays on the desktop behaviour.) */
export function detectTouchUi(w: { matchMedia?: (q: string) => { matches: boolean } } | undefined = globalThis.window): boolean {
  try {
    return !!w?.matchMedia?.("(pointer: coarse)").matches;
  } catch {
    return false;
  }
}

/** Keyboard-aware rectangle from visualViewport: when the on-screen keyboard is up the layout viewport doesn't shrink on iOS. */
export function keyboardInset(
  vv: { height: number; offsetTop: number; width: number } | null | undefined,
  innerHeight: number,
  minKeyboardPx = 120,
): { open: boolean; top: number; height: number; width: number } {
  if (!vv) return { open: false, top: 0, height: innerHeight, width: 0 };
  const covered = innerHeight - vv.height - vv.offsetTop;
  const open = covered > minKeyboardPx || innerHeight - vv.height > minKeyboardPx;
  return { open, top: vv.offsetTop, height: vv.height, width: vv.width };
}
