import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { startBootWarm } from "@/features/agents/acpRuntimesQuery";
import { setDesktopAppBadge } from "@/features/notifications/lib/desktop";
import { useForegroundQueryRefresh } from "@/features/workflows/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { useRelayResumeTriggers } from "@/shared/api/useRelayResumeTriggers";

type AppShellLifecycleEffectsOptions = {
  desktopBadgeEnabled: boolean;
  homeBadgeCountExcludingHighPriority: number;
  topLevelUnreadChannelIds: ReadonlySet<string>;
  unreadChannelNotificationCount: number;
};

export function useAppShellLifecycleEffects({
  desktopBadgeEnabled,
  homeBadgeCountExcludingHighPriority,
  topLevelUnreadChannelIds,
  unreadChannelNotificationCount,
}: AppShellLifecycleEffectsOptions) {
  // Event-driven reconnect: network online / focus / visibility short-circuit
  // the backoff timer when the relay session is degraded (CMD+R gap G1).
  useRelayResumeTriggers();
  useForegroundQueryRefresh();

  // Warm the ACP runtime catalog once at app launch. The shared runtime-catalog
  // cache is in-memory only, so it starts cold every boot; the cheap discovery
  // path reports every harness as "(not installed)" until a forced pass warms
  // it. The create/edit picker and Agents > Agent defaults surfaces read that
  // cheap path, so without this warm they render all-missing (and block agent
  // save) until the user visits Settings > Agents — the accidental workaround.
  // `startBootWarm` drives the module-level boot-warm gate (once per launch, so
  // this remounting effect never re-fires the probe) which makes those cheap
  // surfaces show loading/retryable-error instead of blessing the cold catalog,
  // and swallows the probe's own errors so a failure leaves the last good
  // catalog in place without an unhandled rejection.
  const queryClient = useQueryClient();
  React.useEffect(() => {
    void startBootWarm(queryClient);
  }, [queryClient]);

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
    void relayClient.preconnect().catch((error) => {
      if (!isCancelled) {
        console.error("Failed to preconnect to relay", error);
      }
    });
    return () => {
      isCancelled = true;
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
        : { kind: topLevelUnreadChannelIds.size ? "dot" : "none" },
    );
  }, [
    desktopBadgeEnabled,
    homeBadgeCountExcludingHighPriority,
    topLevelUnreadChannelIds,
    unreadChannelNotificationCount,
  ]);
}
