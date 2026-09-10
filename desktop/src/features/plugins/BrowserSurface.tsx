import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, ArrowRight, RotateCw } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  applyBrowserEvent,
  applyCommandFailure,
  blurAddress,
  browserErrorMessage,
  createCommandOutcomeTracker,
  createInitialChromeState,
  editAddress,
  eventExplainsCurrentCommand,
  focusAddress,
  resyncAfterCommand,
  submitAddress,
  type BrowserChromeEvent,
  type BrowserChromeState,
  type BrowserSessionIdentity,
} from "./browserChrome";
import { registerListeners } from "./registerListeners";
import { useBrowserSession, type Bounds } from "./useBrowserSession";

const MODAL_OVERLAY_SELECTOR =
  '[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"]';
const POPPER_OVERLAY_SELECTOR =
  '[data-radix-popper-content-wrapper], [role="menu"], [role="listbox"]';

function rectsIntersect(a: DOMRect, b: DOMRect): boolean {
  return (
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
  );
}

/** Any open Radix `Dialog`/`AlertDialog` blocks unconditionally — a toast is not modal and does not match this selector. */
function modalOverlayOpen(): boolean {
  return document.querySelector(MODAL_OVERLAY_SELECTOR) !== null;
}

/** A visible popper/menu/listbox blocks only when its rectangle intersects the browser surface. */
function popperOverlayBlocking(surfaceRect: DOMRect): boolean {
  const nodes = document.querySelectorAll<HTMLElement>(POPPER_OVERLAY_SELECTOR);
  for (const node of nodes) {
    const rect = node.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) continue; // not actually visible
    if (rectsIntersect(rect, surfaceRect)) return true;
  }
  return false;
}

function withinWindowViewport(rect: DOMRect): boolean {
  return (
    rect.width > 0 &&
    rect.height > 0 &&
    rect.right > 0 &&
    rect.bottom > 0 &&
    rect.left < window.innerWidth &&
    rect.top < window.innerHeight
  );
}

type BrowserEventPayload = {
  sessionId: string;
  generation: number;
  url?: string;
  canGoBack?: boolean;
  canGoForward?: boolean;
  code?: string;
};

function toChromeEvent(
  type: BrowserChromeEvent["type"],
  payload: BrowserEventPayload,
): BrowserChromeEvent {
  switch (type) {
    case "loading":
      return {
        type: "loading",
        sessionId: payload.sessionId,
        generation: payload.generation,
        url: payload.url ?? "",
      };
    case "location":
      return {
        type: "location",
        sessionId: payload.sessionId,
        generation: payload.generation,
        url: payload.url ?? "",
        canGoBack: payload.canGoBack ?? false,
        canGoForward: payload.canGoForward ?? false,
      };
    case "loaded":
      return {
        type: "loaded",
        sessionId: payload.sessionId,
        generation: payload.generation,
        url: payload.url ?? "",
      };
    case "error":
      return {
        type: "error",
        sessionId: payload.sessionId,
        generation: payload.generation,
        code: payload.code ?? "load-failed",
      };
  }
}

/**
 * The native child webview, its address chrome, and its bounds/visibility
 * lifecycle. The webview itself is drawn by the host over the window; this
 * component only measures and reports the rectangle in `contentRef` and
 * forwards user actions to the `plugin_browser_*` commands.
 */
export function BrowserSurface({
  pluginId,
  contributionId,
}: {
  pluginId: string;
  contributionId: string;
}) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const chromeRef = React.useRef<BrowserChromeState | null>(null);
  const [chrome, setChrome] = React.useState<BrowserChromeState | null>(null);

  // `plugin_browser_open` creates and navigates the native webview before its
  // own promise resolves, so a fast (cached, local) home load can emit
  // `plugin-browser-loading`/`-loaded` before this component ever learns the
  // session identity to filter on. Listeners below are registered once, on
  // mount, independent of `surface`; anything that arrives before the
  // session is known is buffered here and replayed the moment it is.
  const sessionRef = React.useRef<BrowserSessionIdentity | null>(null);
  const eventBufferRef = React.useRef<BrowserChromeEvent[]>([]);
  // Tracks whether a dispatched command's failure should still be reported —
  // superseded by a later dispatch, or already explained by a host event —
  // independent of `loading` (see `createCommandOutcomeTracker`'s doc).
  const commandOutcomeRef = React.useRef(createCommandOutcomeTracker());

  // Registering listeners is itself async (`listen()` returns a promise), so
  // "registered once, on mount" is not enough on its own — `useBrowserSession`
  // must not issue `plugin_browser_open` until every registration below has
  // actually resolved, or a fast native response can still land before
  // anything is listening. `listenersReady` is the gate for that.
  const [listenersReady, setListenersReady] = React.useState(false);
  const [listenerError, setListenerError] = React.useState<string | null>(null);
  const { surface, error: openError } = useBrowserSession(
    pluginId,
    contributionId,
    listenersReady,
  );

  const applyChromeUpdate = React.useCallback(
    (updater: (current: BrowserChromeState) => BrowserChromeState) => {
      setChrome((current) => {
        if (!current) return current;
        const next = updater(current);
        chromeRef.current = next;
        return next;
      });
    },
    [],
  );

  const setChromeState = React.useCallback((next: BrowserChromeState) => {
    chromeRef.current = next;
    setChrome(next);
  }, []);

  React.useEffect(() => {
    if (surface) {
      const session: BrowserSessionIdentity = {
        sessionId: surface.sessionId,
        generation: surface.generation,
      };
      sessionRef.current = session;
      let next = createInitialChromeState(surface.homeUrl);
      for (const event of eventBufferRef.current) {
        next = applyBrowserEvent(next, session, event);
      }
      eventBufferRef.current = [];
      setChromeState(next);
    } else {
      sessionRef.current = null;
      eventBufferRef.current = [];
      chromeRef.current = null;
      setChrome(null);
    }
  }, [surface, setChromeState]);

  // Mount-once: registered before any open completes, not keyed on `surface`.
  React.useEffect(() => {
    let cancelled = false;
    const unlistenFns: Array<() => void> = [];

    const handleIncoming = (
      type: BrowserChromeEvent["type"],
      payload: BrowserEventPayload,
    ) => {
      const event = toChromeEvent(type, payload);
      const session = sessionRef.current;
      if (!session) {
        eventBufferRef.current.push(event);
        return;
      }
      if (eventExplainsCurrentCommand(session, event)) {
        commandOutcomeRef.current.markExplained();
      }
      applyChromeUpdate((current) =>
        applyBrowserEvent(current, session, event),
      );
    };

    void (async () => {
      // One rejected registration must not lose track of the others that
      // *did* succeed (including a late one that only resolves after a
      // sibling rejects) — those still need to be unlistened, not leaked,
      // whether the effect was cancelled in the meantime or not.
      const { succeeded, failedReasons } = await registerListeners([
        () =>
          listen<BrowserEventPayload>("plugin-browser-loading", (event) =>
            handleIncoming("loading", event.payload),
          ),
        () =>
          listen<BrowserEventPayload>("plugin-browser-location", (event) =>
            handleIncoming("location", event.payload),
          ),
        () =>
          listen<BrowserEventPayload>("plugin-browser-loaded", (event) =>
            handleIncoming("loaded", event.payload),
          ),
        () =>
          listen<BrowserEventPayload>("plugin-browser-error", (event) =>
            handleIncoming("error", event.payload),
          ),
      ]);

      if (cancelled) {
        for (const unlisten of succeeded) unlisten();
        return;
      }

      unlistenFns.push(...succeeded);

      if (failedReasons.length > 0) {
        console.error(
          "failed to register plugin-browser-* event listeners",
          failedReasons,
        );
        setListenerError(
          "The browser view couldn't fully start up. Try reopening it.",
        );
        // Never opens the gate on a partial registration — `useBrowserSession`
        // must not issue `plugin_browser_open` when some events can't be
        // observed, since chrome state would then desync from what the
        // native view actually does.
        return;
      }

      setListenersReady(true);
    })();

    return () => {
      cancelled = true;
      for (const unlisten of unlistenFns) unlisten();
    };
  }, [applyChromeUpdate]);

  // Bounds and visibility: measured on mount, resize, scroll, and any DOM
  // mutation (so an opening/closing dialog or popper is caught immediately).
  React.useEffect(() => {
    if (!surface) return;
    let frame = 0;
    // Set once this effect's own cleanup has run — a rejection that lands
    // after teardown (surface changed, route unmounted) is the known stale
    // case and must stay silent. A rejection while still live means the
    // active session's own bounds/visibility calls are genuinely failing —
    // that must not repeat silently forever, so report once and stop
    // retrying until a new surface (new effect run) resets it.
    let cancelled = false;
    let reportedUnavailable = false;

    const handleGenuineFailure = () => {
      if (cancelled || reportedUnavailable) return;
      reportedUnavailable = true;
      applyChromeUpdate((current) => ({
        ...current,
        loading: false,
        errorCode: "plugin-unavailable",
        errorMessage: browserErrorMessage("plugin-unavailable"),
      }));
    };

    const computeAndApply = () => {
      if (reportedUnavailable) return;
      const node = contentRef.current;
      if (!node) return;
      const rect = node.getBoundingClientRect();
      const shouldShow =
        withinWindowViewport(rect) &&
        document.visibilityState !== "hidden" &&
        !modalOverlayOpen() &&
        !popperOverlayBlocking(rect);

      const bounds: Bounds = {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
      };
      invoke("plugin_browser_set_bounds", {
        sessionId: surface.sessionId,
        bounds,
      }).catch(handleGenuineFailure);
      invoke("plugin_browser_set_visible", {
        sessionId: surface.sessionId,
        visible: shouldShow,
      }).catch(handleGenuineFailure);
    };

    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(computeAndApply);
    };

    schedule();
    const observer = new MutationObserver(schedule);
    observer.observe(document.body, {
      attributes: true,
      childList: true,
      subtree: true,
    });
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    document.addEventListener("visibilitychange", schedule);

    return () => {
      cancelled = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      document.removeEventListener("visibilitychange", schedule);
    };
  }, [surface, applyChromeUpdate]);

  // A command the user directly triggered (navigate/back/forward/reload)
  // failed at the invoke layer itself — e.g. a stale/closed session — rather
  // than through a `plugin-browser-error` event. `applyCommandFailure` maps
  // it to the same closed, fixed-text error surface, but only for the
  // dispatch `token` identifies: not when a domain event already explained
  // it, and not when a later command has since superseded it (see
  // `createCommandOutcomeTracker`'s doc).
  const reportCommandFailure = React.useCallback(
    (token: number) => {
      if (!commandOutcomeRef.current.shouldReportFailure(token)) return;
      applyChromeUpdate(applyCommandFailure);
    },
    [applyChromeUpdate],
  );

  const dispatchChromeCommand = (invokeCommand: () => Promise<unknown>) => {
    if (!chromeRef.current) return;
    setChromeState(resyncAfterCommand(chromeRef.current));
    const token = commandOutcomeRef.current.dispatch();
    invokeCommand().catch(() => reportCommandFailure(token));
  };

  const handleBack = () => {
    if (!surface || !chrome?.canGoBack) return;
    dispatchChromeCommand(() =>
      invoke("plugin_browser_back", { sessionId: surface.sessionId }),
    );
  };
  const handleForward = () => {
    if (!surface || !chrome?.canGoForward) return;
    dispatchChromeCommand(() =>
      invoke("plugin_browser_forward", { sessionId: surface.sessionId }),
    );
  };
  const handleReload = () => {
    if (!surface) return;
    dispatchChromeCommand(() =>
      invoke("plugin_browser_reload", { sessionId: surface.sessionId }),
    );
  };
  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!surface || !chromeRef.current) return;
    const { state: next, address } = submitAddress(
      chromeRef.current,
      chromeRef.current.addressValue,
    );
    setChromeState(next);
    if (address !== null) {
      const token = commandOutcomeRef.current.dispatch();
      invoke("plugin_browser_navigate", {
        sessionId: surface.sessionId,
        input: address,
      }).catch(() => reportCommandFailure(token));
    }
  };

  return (
    <section
      className="flex h-full min-h-0 flex-col"
      data-testid="plugin-browser-surface"
    >
      <form
        className="flex items-center gap-2 border-b border-border/70 p-2"
        onSubmit={handleSubmit}
      >
        <Button
          aria-label="Back"
          disabled={!chrome?.canGoBack}
          onClick={handleBack}
          size="icon"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <Button
          aria-label="Forward"
          disabled={!chrome?.canGoForward}
          onClick={handleForward}
          size="icon"
          type="button"
          variant="ghost"
        >
          <ArrowRight className="h-4 w-4" />
        </Button>
        <Button
          aria-label="Reload"
          onClick={handleReload}
          size="icon"
          type="button"
          variant="ghost"
        >
          <RotateCw className="h-4 w-4" />
        </Button>
        <Input
          aria-invalid={chrome?.addressInvalid || undefined}
          aria-label="Address"
          onBlur={() => {
            if (chromeRef.current)
              setChromeState(blurAddress(chromeRef.current));
          }}
          onChange={(event) => {
            if (chromeRef.current) {
              setChromeState(
                editAddress(chromeRef.current, event.target.value),
              );
            }
          }}
          onFocus={() => {
            if (chromeRef.current)
              setChromeState(focusAddress(chromeRef.current));
          }}
          value={chrome?.addressValue ?? ""}
        />
      </form>
      {chrome?.errorMessage ? (
        <p
          className="border-b border-destructive/40 bg-destructive/5 p-2 text-sm text-destructive"
          role="alert"
        >
          {chrome.errorMessage}
        </p>
      ) : null}
      {openError ? (
        <p
          className="border-b border-destructive/40 bg-destructive/5 p-2 text-sm text-destructive"
          role="alert"
        >
          {openError}
        </p>
      ) : null}
      {listenerError ? (
        <p
          className="border-b border-destructive/40 bg-destructive/5 p-2 text-sm text-destructive"
          role="alert"
        >
          {listenerError}
        </p>
      ) : null}
      <div
        className="min-h-0 flex-1"
        data-testid="plugin-browser-content"
        ref={contentRef}
      />
    </section>
  );
}
