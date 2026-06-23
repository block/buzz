import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { toast } from "sonner";

const MIGRATION_TOAST_KEY = "buzz-legacy-nest-migrated-notified";

/**
 * Surface nest-related backend events as toasts.
 *
 * - `repos-dir-error`: a configured `repos_dir` failed to validate or its
 *   symlink could not be applied (invalid path, downgrade refused, external
 *   target gone). Emitted by `apply_workspace` on both the validate-reject
 *   and the runtime symlink-failure paths, so a bad `repos_dir` is always
 *   visibly surfaced rather than silently logged to console.
 * - `legacy-nest-migrated`: the agent's knowledge was carried over from a
 *   legacy `~/.sprout` nest. Shown once per machine (deduped via
 *   localStorage); the backend re-emits each launch while `~/.sprout` exists,
 *   which also covers the event being emitted before this listener mounts.
 *
 * Mounted at the app root ahead of the workspace-init effect so the listener
 * is registered before the first `apply_workspace` call.
 */
export function useNestNotifications(): void {
  useEffect(() => {
    const unlistenReposError = listen<string>("repos-dir-error", (event) => {
      toast.error("Repos directory not applied", {
        description: event.payload,
      });
    });

    const unlistenMigrated = listen("legacy-nest-migrated", () => {
      if (localStorage.getItem(MIGRATION_TOAST_KEY) === "true") {
        return;
      }
      localStorage.setItem(MIGRATION_TOAST_KEY, "true");
      toast.success("Migrated notes from ~/.sprout", {
        description: "You can delete it to reclaim disk space.",
      });
    });

    return () => {
      void unlistenReposError.then((fn) => fn());
      void unlistenMigrated.then((fn) => fn());
    };
  }, []);
}
