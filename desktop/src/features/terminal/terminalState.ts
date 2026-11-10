export type TerminalOwner = "buzz" | "terminal";

export type HandoffState = {
  owner: TerminalOwner;
  chordArmed: boolean;
  composing: boolean;
};

export type HandoffAction =
  | { type: "composition-start" }
  | { type: "composition-end" }
  | { type: "chord-down"; repeat: boolean }
  | { type: "chord-up" }
  | { type: "focus-lost" };

export type HandoffResult = {
  state: HandoffState;
  toggled: boolean;
};

export const INITIAL_HANDOFF_STATE: HandoffState = {
  owner: "buzz",
  chordArmed: false,
  composing: false,
};

/** The shortcut is a transaction completed only by a matching key-up. */
export function reduceHandoff(
  state: HandoffState,
  action: HandoffAction,
): HandoffResult {
  if (action.type === "composition-start") {
    return {
      state: { ...state, chordArmed: false, composing: true },
      toggled: false,
    };
  }
  if (action.type === "composition-end") {
    return { state: { ...state, composing: false }, toggled: false };
  }
  if (action.type === "focus-lost") {
    return { state: { ...state, chordArmed: false }, toggled: false };
  }
  if (action.type === "chord-down") {
    if (state.composing || action.repeat) return { state, toggled: false };
    return { state: { ...state, chordArmed: true }, toggled: false };
  }
  if (!state.chordArmed || state.composing) {
    return {
      state: { ...state, chordArmed: false },
      toggled: false,
    };
  }
  return {
    state: {
      ...state,
      chordArmed: false,
      owner: state.owner === "buzz" ? "terminal" : "buzz",
    },
    toggled: true,
  };
}

export function encodeTerminalKey(event: {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}): string | null {
  if (event.metaKey) return null;
  const special: Readonly<Record<string, string>> = {
    Enter: "\r",
    Backspace: "\u007f",
    Tab: "\t",
    Escape: "\u001b",
    ArrowUp: "\u001b[A",
    ArrowDown: "\u001b[B",
    ArrowRight: "\u001b[C",
    ArrowLeft: "\u001b[D",
    Home: "\u001b[H",
    End: "\u001b[F",
    Delete: "\u001b[3~",
    PageUp: "\u001b[5~",
    PageDown: "\u001b[6~",
  };
  const specialValue = special[event.key];
  if (specialValue) return `${event.altKey ? "\u001b" : ""}${specialValue}`;
  if (event.ctrlKey && event.key.length === 1) {
    const code = event.key.toUpperCase().charCodeAt(0);
    if (code >= 64 && code <= 95) return String.fromCharCode(code - 64);
  }
  if (event.altKey && event.key.length === 1) return `\u001b${event.key}`;
  return null;
}

export type TabChord = "close" | "new" | "next" | "previous";

/**
 * Tab-management chords, matched on `code` so they survive alternate layouts.
 * `isMac` is a parameter rather than a `navigator` read so matching stays pure,
 * the same shape as `matchBackForwardChord`.
 *
 * The modifier is the platform's primary one and *only* that one — deliberately
 * narrower than the sibling `isToggleChord`, which accepts either. On macOS
 * `^W` is werase and `^T` is transpose; a capture-phase listener that claimed
 * them would consume the event before the textarea could encode it, so Control
 * must keep falling through to the PTY. Non-mac platforms pay the mirror cost
 * on Ctrl (matching gnome-terminal, which moves its own tab chords onto
 * Ctrl+Shift for exactly this reason) — see the report for why that is scoped
 * out rather than solved here.
 *
 * ⌘W is matched even though macOS currently resolves it as the File > Close
 * Window key equivalent before the webview sees any key event: the accelerator
 * has to be released natively for this arm to ever run, and matching it now
 * means the frontend needs no further change when it is.
 */
export function matchTabChord(
  event: {
    altKey: boolean;
    code: string;
    ctrlKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
  },
  isMac: boolean,
): TabChord | null {
  if (event.altKey) return null;
  if (isMac ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey)
    return null;
  if (event.shiftKey) {
    if (event.code === "ArrowLeft") return "previous";
    if (event.code === "ArrowRight") return "next";
    return null;
  }
  if (event.code === "KeyT") return "new";
  if (event.code === "KeyW") return "close";
  return null;
}

/**
 * The id ⇧⌘←/→ moves to, wrapping at both ends.
 *
 * Closing tabs are skipped because the tab bar disables their select button —
 * the keyboard path must not reach a tab the mouse path cannot.
 */
export function stepSession(
  sessions: readonly { active: boolean; closing: boolean; id: string }[],
  direction: -1 | 1,
): string | null {
  const count = sessions.length;
  const activeIndex = sessions.findIndex((session) => session.active);
  if (activeIndex < 0) return null;
  for (let offset = 1; offset < count; offset += 1) {
    const index =
      (((activeIndex + direction * offset) % count) + count) % count;
    const candidate = sessions[index];
    if (!candidate.closing) return candidate.id;
  }
  return null;
}

export function encodePaste(text: string, bracketed: boolean): string {
  return bracketed ? `\u001b[200~${text}\u001b[201~` : text;
}

export type ScrollAccumulator = { remainderPx: number };

/** Preserve sub-cell trackpad movement instead of truncating every event. */
export function accumulateScrollLines(
  state: ScrollAccumulator,
  deltaPx: number,
  cellHeight: number,
): { state: ScrollAccumulator; lines: number } {
  if (!(cellHeight > 0) || !Number.isFinite(deltaPx)) {
    return { state, lines: 0 };
  }
  const total = state.remainderPx + deltaPx;
  const lines = Math.trunc(total / cellHeight);
  return {
    state: { remainderPx: total - lines * cellHeight },
    lines,
  };
}
