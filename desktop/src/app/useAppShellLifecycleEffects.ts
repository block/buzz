import * as React from "react";

import { setDesktopAppBadge } from "@/features/notifications/lib/desktop";
import { relayClient } from "@/shared/api/relayClient";
import { useRelayResumeTriggers } from "@/shared/api/useRelayResumeTriggers";

type AppShellLifecycleEffectsOptions = {
  desktopBadgeEnabled: boolean;
  homeBadgeCountExcludingHighPriority: number;
  unreadChannelIds: ReadonlySet<string>;
  unreadChannelNotificationCount: number;
};

export function useAppShellLifecycleEffects({
  desktopBadgeEnabled,
  homeBadgeCountExcludingHighPriority,
  unreadChannelIds,
  unreadChannelNotificationCount,
}: AppShellLifecycleEffectsOptions) {
  // Event-driven reconnect: network online / focus / visibility short-circuit
  // the backoff timer when the relay session is degraded (CMD+R gap G1).
  useRelayResumeTriggers();

  // Prevent webview file:/// navigation on file drop outside the composer.
  // Scoped to file drags only (text drag-and-drop into inputs still works).
  // Composer's onDrop fires first (React synthetic before window bubble).
  React.useEffect(() => {
    function preventNavigation(e: DragEvent) {
      if (e.dataTransfer?.types.includes("Files")) {
        e.preventDefault();
      }
    }
    window.addEventListener("dragover", preventNavigation);
    window.addEventListener("drop", preventNavigation);
    return () => {
      window.removeEventListener("dragover", preventNavigation);
      window.removeEventListener("drop", preventNavigation);
    };
  }, []);

  React.useEffect(() => {
    let isCancelled = false;

    const startPreconnect = () => {
      if (isCancelled) {
        return;
      }

      void relayClient.preconnect().catch((error) => {
        if (!isCancelled) {
          console.error("Failed to preconnect to relay", error);
        }
      });
    };

    if ("requestIdleCallback" in window) {
      const idleId = window.requestIdleCallback(startPreconnect, {
        timeout: 1_500,
      });
      return () => {
        isCancelled = true;
        window.cancelIdleCallback(idleId);
      };
    }

    const timeoutId = globalThis.setTimeout(startPreconnect, 250);
    return () => {
      isCancelled = true;
      globalThis.clearTimeout(timeoutId);
    };
  }, []);

  React.useEffect(() => {
    if (!desktopBadgeEnabled) {
      return;
    }

    const count =
      unreadChannelNotificationCount + homeBadgeCountExcludingHighPriority;
    void setDesktopAppBadge(
      count
        ? { kind: "count", count }
        : { kind: unreadChannelIds.size ? "dot" : "none" },
    );
  }, [
    desktopBadgeEnabled,
    homeBadgeCountExcludingHighPriority,
    unreadChannelIds,
    unreadChannelNotificationCount,
  ]);
}
