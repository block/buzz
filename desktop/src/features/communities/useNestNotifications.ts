import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { toast } from "sonner";

const MIGRATION_TOAST_KEY = "buzz-legacy-nest-migrated-notified";

/**
 * Register all nest-related backend event listeners.
 *
 * Extracted for testability: accepts `listenFn` and `toastFn` as parameters
 * so unit tests can inject mocks without a Tauri runtime. The production hook
 * calls this with the real `listen` and `toast`.
 *
 * Returns a cleanup function that calls every unlisten function.
 *
 * Covered events:
 * - `repos-dir-error`: a configured `repos_dir` failed to validate or its
 *   symlink could not be applied.
 * - `legacy-nest-migrated`: the agent's knowledge was carried over from a
 *   legacy `~/.sprout` nest. Shown once per machine (deduped via localStorage).
 * - `workspace-degraded`: a post-commit restore step failed after the workspace
 *   switch succeeded. Event-sync dispatch failure does not emit this event
 *   (shutdown-time error, no toast surface exists).
 */
export function registerNestNotifications(
  listenFn: typeof listen,
  toastFn: typeof toast,
): () => void {
  const unlistenReposError = listenFn<string>("repos-dir-error", (event) => {
    toastFn.error("Repos directory not applied", {
      description: event.payload,
    });
  });

  const unlistenMigrated = listenFn("legacy-nest-migrated", () => {
    if (localStorage.getItem(MIGRATION_TOAST_KEY) === "true") {
      return;
    }
    localStorage.setItem(MIGRATION_TOAST_KEY, "true");
    toastFn.success("Migrated notes from ~/.sprout", {
      description: "You can delete it to reclaim disk space.",
    });
  });

  const unlistenDegraded = listenFn<string>("workspace-degraded", (event) => {
    toastFn.error("Workspace partially degraded", {
      description: event.payload,
    });
  });

  return () => {
    void unlistenReposError.then((fn) => fn());
    void unlistenMigrated.then((fn) => fn());
    void unlistenDegraded.then((fn) => fn());
  };
}

/**
 * Surface nest-related backend events as toasts.
 *
 * Mounted at the app root ahead of the community-init effect so the listener
 * is registered before the first `apply_workspace` call.
 */
export function useNestNotifications(): void {
  useEffect(() => {
    const cleanup = registerNestNotifications(listen, toast);
    return cleanup;
  }, []);
}
