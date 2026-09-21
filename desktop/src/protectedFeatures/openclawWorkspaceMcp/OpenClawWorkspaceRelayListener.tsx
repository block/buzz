import { useEffect } from "react";
import { useFeatureEnabled } from "@/shared/features";

/**
 * Mounted only in Hula builds. Relay NOTICE / HULA frame handling is wired
 * through `handleProtectedRelayPayload` imported by the shared relay client
 * via `@protected-feature-components`.
 */
export function OpenClawWorkspaceRelayListener() {
  const enabled = useFeatureEnabled("openclaw-workspace-mcp");
  useEffect(() => {
    if (!enabled) return;
    // Status refresh hook point for future UI toasts.
  }, [enabled]);
  return null;
}
