import { closeAllWebSockets } from "@/shared/api/relayWebSocketClose";

/** Upper bound on the graceful WebSocket teardown before the webview reloads. */
export const RELOAD_TEARDOWN_TIMEOUT_MS = 500;

/**
 * The single reload in progress. Overlapping triggers (a second Cmd+R, or the
 * idle backstop firing while Cmd+R is mid-teardown) collapse onto it so the
 * native WebSocket teardown never runs twice. Never cleared in production —
 * `location.reload()` discards the module — so a settled reload cannot start a
 * second one on the dying page.
 */
let inFlightReload: Promise<void> | null = null;

type RequestRendererReloadOverrides = {
  /** Navigate the webview. Injected in tests so the runner never reloads. */
  reload?: () => void;
  /** Bounded native teardown. Injected to exercise the rejection/timeout path. */
  closeSockets?: () => Promise<void>;
};

/**
 * Reload the webview after a bounded native WebSocket teardown, exactly once
 * per navigation.
 *
 * Shared by the Cmd+R shortcut (`useReloadShortcut`) and the idle backstop
 * (`useIdleAutoReload`): both need the same clean-teardown path so the native
 * engine and its child agents survive while the renderer's heap and timers are
 * discarded. Only the backstop applies eligibility guards; this primitive is
 * pure execution.
 *
 * The reload is unconditional. Teardown is attempted for at most
 * `RELOAD_TEARDOWN_TIMEOUT_MS`; a hung socket loses to the timeout and a
 * rejecting teardown is logged and swallowed, so the reload always runs in the
 * `finally`. A rejection must never win the race and strand the renderer on a
 * stale heap.
 */
export function requestRendererReload({
  reload = () => window.location.reload(),
  closeSockets = closeAllWebSockets,
}: RequestRendererReloadOverrides = {}): Promise<void> {
  if (inFlightReload) return inFlightReload;
  inFlightReload = (async () => {
    try {
      await Promise.race([
        closeSockets().catch((err) =>
          console.debug("requestRendererReload teardown rejected:", err),
        ),
        new Promise<void>((resolve) =>
          window.setTimeout(resolve, RELOAD_TEARDOWN_TIMEOUT_MS),
        ),
      ]);
    } finally {
      reload();
    }
  })();
  return inFlightReload;
}

/** Clear the module-level in-flight reload between tests (production reloads
 * the page, so the guard is never reset there). */
export function resetRendererReloadForTests(): void {
  inFlightReload = null;
}
