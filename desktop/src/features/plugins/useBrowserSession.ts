import * as React from "react";
import { invoke } from "@tauri-apps/api/core";

export type Bounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type OpenedSurface = {
  sessionId: string;
  generation: number;
  homeUrl: string;
};

type InvokeFn = <T>(
  command: string,
  payload?: Record<string, unknown>,
) => Promise<T>;

/**
 * Pure, injectable session lifecycle — a single serialized operation queue,
 * no wry/Tauri types. Every call (open or close) is appended to the same
 * chain and runs only after every previously queued call has fully settled,
 * including whatever native close that call performed. That serialization is
 * the whole correctness argument:
 *
 * - Opening a second surface replaces the first for free — every `open`
 *   closes whatever is currently active as the first thing it does, and by
 *   chain order "currently active" always reflects the immediately
 *   preceding queued operation's final state.
 * - A `closeActive` queued while an `open` is still awaiting its native
 *   `plugin_browser_open` response does not run until that response lands
 *   and the surface is adopted — so it always has a real session to close,
 *   never a race against an in-flight open.
 * - No separate token/generation bookkeeping is needed on this side: at most
 *   one operation is ever actually in flight, so there is nothing to
 *   supersede concurrently, only to sequence.
 * - A native close failure rejects the caller's own promise (so
 *   `resetCommunityState`'s `await closeActiveBrowserSession()` propagates
 *   it and the next community waits), but the chain itself is normalized to
 *   never reject, so one failed close does not wedge every later open/close.
 *
 * Exported (not just used internally) so tests can construct an isolated
 * lifecycle around a fake, deferred-promise `invoke` and assert ordering
 * deterministically instead of racing the real native bridge.
 */
export function createBrowserSessionLifecycle(invokeFn: InvokeFn) {
  let chain: Promise<unknown> = Promise.resolve();
  let activeSession: OpenedSurface | null = null;

  function enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const result = chain.then(operation, operation);
    // The chain itself must never reject, or every later queued operation
    // would silently stop running after the first failure.
    chain = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  async function closeCurrent(): Promise<void> {
    if (!activeSession) return;
    // Kept until the native close actually succeeds: clearing it first would
    // forget a still-live session on failure, so a later close/open would
    // never retry it and the native process would leak. On failure the next
    // queued close (or the close that precedes the next open) retries the
    // exact same sessionId.
    const sessionId = activeSession.sessionId;
    await invokeFn("plugin_browser_close", { sessionId });
    activeSession = null;
  }

  return {
    open(
      pluginId: string,
      contributionId: string,
      bounds: Bounds,
    ): Promise<OpenedSurface> {
      return enqueue(async () => {
        // If this throws, `activeSession` is left set to the still-live prior
        // session (see `closeCurrent`) and no new open is issued — the
        // failure propagates to this call's own caller instead of silently
        // orphaning the prior session.
        await closeCurrent();
        const surface = await invokeFn<OpenedSurface>("plugin_browser_open", {
          pluginId,
          contributionId,
          bounds,
        });
        activeSession = surface;
        return surface;
      });
    },
    closeActive(): Promise<void> {
      return enqueue(() => closeCurrent());
    },
  };
}

// Module-level, not component state: a native child webview session outlives
// any single route mount, and community teardown must be able to await and
// close it even when no `BrowserSurface` is mounted.
const productionLifecycle = createBrowserSessionLifecycle(invoke);

/**
 * Opens a browser surface, replacing whatever surface is currently open or
 * opening. See `createBrowserSessionLifecycle` for the ordering guarantee.
 */
export function openBrowserSession(
  pluginId: string,
  contributionId: string,
  bounds: Bounds,
): Promise<OpenedSurface> {
  return productionLifecycle.open(pluginId, contributionId, bounds);
}

/**
 * Awaited teardown of whatever browser session is open or opening. Called
 * from route unmount (idempotent) and from `resetCommunityState`, in both
 * cases before anything else that assumes no browser session survives.
 */
export function closeActiveBrowserSession(): Promise<void> {
  return productionLifecycle.closeActive();
}

/**
 * Opens a browser session for the lifetime of the calling component and
 * performs the same idempotent awaited close on unmount. `pluginId` and
 * `contributionId` are expected to stay fixed for the component's lifetime —
 * the route is remounted (via its key) when either changes.
 *
 * `listenersReady` gates the open itself: `plugin_browser_open`'s own native
 * execution can emit `plugin-browser-*` events before its own promise
 * resolves, so the caller must finish registering its event listeners (an
 * async `listen()` call) *before* this hook is allowed to issue the open —
 * otherwise a fast/cached load can complete and emit before anything is
 * listening. Pass `true` only once every registration has resolved.
 */
export function useBrowserSession(
  pluginId: string,
  contributionId: string,
  listenersReady: boolean,
): { surface: OpenedSurface | null; error: string | null } {
  const [surface, setSurface] = React.useState<OpenedSurface | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!listenersReady) return;
    let cancelled = false;
    const bounds: Bounds = { x: 0, y: 0, width: 0, height: 0 };
    setSurface(null);
    setError(null);
    openBrowserSession(pluginId, contributionId, bounds)
      .then((opened) => {
        if (!cancelled) setSurface(opened);
      })
      .catch((cause) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      cancelled = true;
      // Fire-and-forget by necessity (the component is gone, nothing left to
      // update), but never silently swallowed: a failure here still retains
      // the session in the lifecycle's own state (see `closeCurrent`), so the
      // next open/close retries it — this only guards against an unhandled
      // rejection reaching the console as a crash-looking error.
      closeActiveBrowserSession().catch((cause) => {
        // eslint-disable-next-line no-console
        console.error("closeActiveBrowserSession failed on unmount:", cause);
      });
    };
  }, [pluginId, contributionId, listenersReady]);

  return { surface, error };
}
