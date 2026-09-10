/**
 * Pure state logic for the plugin browser chrome (address field, back,
 * forward, reload, loading/error state). No React, no Tauri — the host
 * events named here are the `plugin-browser-*` contract described in
 * `docs/plugin-browser-prototype.md`.
 */

/** Closed set of browser error codes the host may emit. */
export type BrowserErrorCode =
  | "navigation-denied"
  | "load-failed"
  | "plugin-timeout"
  | "plugin-unavailable"
  | "plugin-protocol-error"
  | "plugin-contract-mismatch"
  | "unsupported-platform";

/** Fixed, host-independent sentence shown for each closed error code. */
export const BROWSER_ERROR_MESSAGES: Record<BrowserErrorCode, string> = {
  "navigation-denied": "This address can't be opened here.",
  "load-failed": "The page failed to load.",
  "plugin-timeout": "The plugin didn't respond in time.",
  "plugin-unavailable": "The plugin isn't available right now.",
  "plugin-protocol-error": "The plugin sent an invalid response.",
  "plugin-contract-mismatch":
    "This plugin isn't compatible with this version of Buzz.",
  "unsupported-platform": "Browsing isn't supported on this platform.",
};

const FALLBACK_ERROR_MESSAGE = "Something went wrong.";

/** Renders the fixed sentence for a browser error code. */
export function browserErrorMessage(code: string): string {
  return (
    BROWSER_ERROR_MESSAGES[code as BrowserErrorCode] ?? FALLBACK_ERROR_MESSAGE
  );
}

/** Identity used to reject events for a superseded session or generation. */
export type BrowserSessionIdentity = {
  sessionId: string;
  generation: number;
};

export type BrowserChromeState = {
  /** Text currently shown in the address field. */
  addressValue: string;
  /** Whether the address field currently has focus. */
  addressFocused: boolean;
  /** Whether the user has typed since the field was last synced. */
  addressEdited: boolean;
  /** Whether the field is showing a locally rejected (empty) submission. */
  addressInvalid: boolean;
  /** Last URL reported by the native view — the resync target. */
  lastKnownUrl: string;
  /** Native `WebView::can_go_back()` as of the last location poll. */
  canGoBack: boolean;
  /** Native `WebView::can_go_forward()` as of the last location poll. */
  canGoForward: boolean;
  /** Whether a tracked load is in flight. */
  loading: boolean;
  /** Closed error code from the most recent `plugin-browser-error`, if any. */
  errorCode: BrowserErrorCode | null;
  /** Fixed sentence for `errorCode`. */
  errorMessage: string | null;
};

export function createInitialChromeState(homeUrl: string): BrowserChromeState {
  return {
    addressValue: homeUrl,
    addressFocused: false,
    addressEdited: false,
    addressInvalid: false,
    lastKnownUrl: homeUrl,
    canGoBack: false,
    canGoForward: false,
    loading: true,
    errorCode: null,
    errorMessage: null,
  };
}

export type BrowserChromeEvent =
  | {
      type: "loading";
      sessionId: string;
      generation: number;
      url: string;
    }
  | {
      type: "location";
      sessionId: string;
      generation: number;
      url: string;
      canGoBack: boolean;
      canGoForward: boolean;
    }
  | {
      type: "loaded";
      sessionId: string;
      generation: number;
      url: string;
    }
  | {
      type: "error";
      sessionId: string;
      generation: number;
      code: string;
    };

/** True when an event's sessionId/generation match the live session identity. */
export function matchesSession(
  session: BrowserSessionIdentity,
  sessionId: string,
  generation: number,
): boolean {
  return session.sessionId === sessionId && session.generation === generation;
}

/** True while a focused, unsubmitted edit must not be overwritten by native events. */
function editLocked(state: BrowserChromeState): boolean {
  return state.addressFocused && state.addressEdited;
}

/**
 * Applies one host event to chrome state. Events for a stale session or a
 * superseded generation are ignored outright: a result belonging to an
 * earlier open/disable/uninstall/route-exit must never take effect once a
 * newer generation is live.
 */
export function applyBrowserEvent(
  state: BrowserChromeState,
  session: BrowserSessionIdentity,
  event: BrowserChromeEvent,
): BrowserChromeState {
  if (!matchesSession(session, event.sessionId, event.generation)) {
    return state;
  }

  switch (event.type) {
    case "loading":
      return { ...state, loading: true, errorCode: null, errorMessage: null };
    case "location": {
      const next: BrowserChromeState = {
        ...state,
        lastKnownUrl: event.url,
        canGoBack: event.canGoBack,
        canGoForward: event.canGoForward,
      };
      // Display-only: never clears loading state or the pending deadline.
      if (!editLocked(state)) {
        next.addressValue = event.url;
      }
      return next;
    }
    case "loaded": {
      const next: BrowserChromeState = {
        ...state,
        loading: false,
        errorCode: null,
        errorMessage: null,
        lastKnownUrl: event.url,
      };
      if (!editLocked(state)) {
        next.addressValue = event.url;
      }
      return next;
    }
    case "error":
      return {
        ...state,
        loading: false,
        errorCode: event.code as BrowserErrorCode,
        errorMessage: browserErrorMessage(event.code),
      };
    default:
      return state;
  }
}

/** Returns the value to send to `plugin_browser_navigate`, or `null` if the input is blank. */
export function normalizeSubmittedAddress(raw: string): string | null {
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** The user is typing — protects the field from a native event overwrite. */
export function editAddress(
  state: BrowserChromeState,
  value: string,
): BrowserChromeState {
  return {
    ...state,
    addressValue: value,
    addressEdited: true,
    addressInvalid: false,
  };
}

export function focusAddress(state: BrowserChromeState): BrowserChromeState {
  return { ...state, addressFocused: true };
}

/** Blur re-syncs the field to the last known URL only when there was no edit. */
export function blurAddress(state: BrowserChromeState): BrowserChromeState {
  if (state.addressEdited) {
    return { ...state, addressFocused: false };
  }
  return {
    ...state,
    addressFocused: false,
    addressEdited: false,
    addressValue: state.lastKnownUrl,
  };
}

/**
 * Empty or whitespace-only input stays local and invalid — no navigation is
 * requested. A non-blank submission clears the edit lock so subsequent native
 * events can update the field again.
 */
export function submitAddress(
  state: BrowserChromeState,
  raw: string,
): { state: BrowserChromeState; address: string | null } {
  const address = normalizeSubmittedAddress(raw);
  if (address === null) {
    return { state: { ...state, addressInvalid: true }, address: null };
  }
  return {
    state: {
      ...state,
      addressValue: address,
      addressEdited: false,
      addressInvalid: false,
    },
    address,
  };
}

/**
 * Back, forward, and reload all clear the edit lock and resync the display.
 * Does not touch `loading` — only the native `loading` event arms it (see
 * `applyBrowserEvent`). A same-document/fragment traversal never emits one,
 * and forcing it here left the spinner stuck on indefinitely since nothing
 * would ever clear it.
 */
export function resyncAfterCommand(
  state: BrowserChromeState,
): BrowserChromeState {
  return {
    ...state,
    addressEdited: false,
    addressInvalid: false,
    addressValue: state.lastKnownUrl,
    errorCode: null,
    errorMessage: null,
  };
}

/**
 * Applies a direct `invoke` rejection for a user-triggered command
 * (navigate/back/forward/reload) — as opposed to a `plugin-browser-error`
 * event, which `applyBrowserEvent` already handles. Always renders the fixed
 * `plugin-unavailable` fallback; the caller (`BrowserSurface`'s command
 * dispatch, via `createCommandOutcomeTracker`) is responsible for not
 * invoking this at all when a domain event has already explained the same
 * command's outcome, or when a newer command has since superseded it —
 * `loading` is not a reliable signal for that, since a location-only
 * traversal never sets it.
 */
export function applyCommandFailure(
  state: BrowserChromeState,
): BrowserChromeState {
  return {
    ...state,
    loading: false,
    errorCode: "plugin-unavailable",
    errorMessage: browserErrorMessage("plugin-unavailable"),
  };
}

/**
 * Only a domain `error` explains a rejected dispatch. `loading`/`location`
 * are display-only and fire independent of any dispatched command. `loaded`
 * is excluded too: it can belong to a prior in-flight document settling
 * while a newer address's own invoke is still pending, so it doesn't explain
 * that pending command's eventual rejection either. A successful invoke has
 * no `.catch` to suppress, so this only ever needs to gate the failure path.
 */
export function eventExplainsCommand(
  eventType: BrowserChromeEvent["type"],
): boolean {
  return eventType === "error";
}

/**
 * Whether a host event should mark the currently-dispatched command
 * explained (see `createCommandOutcomeTracker`) — requires BOTH
 * `eventExplainsCommand` (only a domain `error`) AND that the event's
 * sessionId/generation match the live session, not a stale/superseded one.
 * A wrong-session error must never suppress the fallback for whatever the
 * user is actually waiting on right now.
 */
export function eventExplainsCurrentCommand(
  session: BrowserSessionIdentity,
  event: BrowserChromeEvent,
): boolean {
  return (
    eventExplainsCommand(event.type) &&
    matchesSession(session, event.sessionId, event.generation)
  );
}

/**
 * Tracks whether the in-flight user-triggered command (navigate/back/
 * forward/reload) has already been explained by a domain event (see
 * `eventExplainsCommand`), and whether a settling invoke belongs to the
 * still-current dispatch or one superseded by a later dispatch. The host
 * contract guarantees a failure's domain error event always precedes its
 * invoke rejection.
 */
export function createCommandOutcomeTracker() {
  let token = 0;
  let explained = true;
  return {
    dispatch(): number {
      token += 1;
      explained = false;
      return token;
    },
    markExplained(): void {
      explained = true;
    },
    shouldReportFailure(dispatchedToken: number): boolean {
      return dispatchedToken === token && !explained;
    },
  };
}

/**
 * The fourteen `plugin_*` Tauri commands and the four Tauri event-plugin
 * commands that a native-smoke run must route to the real native `invoke`
 * instead of the mock command handler. This is the single source of truth
 * for that passthrough set — both `main.tsx`'s bootstrap gate and
 * `testing/e2eBridge.ts`'s invoke wrapper import it.
 */
export const PLUGIN_SMOKE_PASSTHROUGH_COMMANDS: ReadonlySet<string> = new Set([
  "plugin_pick_directory",
  "plugin_install",
  "plugin_list",
  "plugin_set_enabled",
  "plugin_uninstall",
  "plugin_browser_open",
  "plugin_browser_navigate",
  "plugin_browser_back",
  "plugin_browser_forward",
  "plugin_browser_reload",
  "plugin_browser_set_bounds",
  "plugin_browser_set_visible",
  "plugin_browser_close",
  "plugin_browser_smoke_enabled",
  "plugin:event|listen",
  "plugin:event|emit",
  "plugin:event|emit_to",
  "plugin:event|unlisten",
]);
