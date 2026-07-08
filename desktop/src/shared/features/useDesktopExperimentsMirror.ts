import { useEffect } from "react";

import { setDesktopExperiments } from "@/shared/api/tauri";
import { useFeatureSnapshot } from "./useFeatureEnabled";

/**
 * Mirrors the preview-experiment overrides (localStorage) to the Rust side
 * so agent spawn-time code can consult them — e.g. the acpToolSummaries
 * experiment decides whether spawned agents get the tool-summary kill
 * switch. Runs on mount (app boot) and again whenever any toggle changes.
 *
 * Best-effort: a failed mirror only logs. The Rust read side treats missing
 * or stale state as "all experiments off" (the safe default).
 */
export function useDesktopExperimentsMirror(): void {
  const overrides = useFeatureSnapshot();

  useEffect(() => {
    void setDesktopExperiments(overrides).catch((error) => {
      console.warn("[FeatureFlags] failed to mirror experiments", error);
    });
  }, [overrides]);
}
