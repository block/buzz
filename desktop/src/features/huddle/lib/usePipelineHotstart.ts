import { invoke } from "@tauri-apps/api/core";
import * as React from "react";

const PIPELINE_HOTSTART_INTERVAL_MS = 15_000;

/** Check if voice models finished downloading mid-huddle. */
export function usePipelineHotstart(ephemeralChannelId: string | null) {
  React.useEffect(() => {
    if (!ephemeralChannelId) return;
    const checkPipelineHotstart = () => {
      invoke("check_pipeline_hotstart").catch(() => {
        /* best-effort */
      });
    };
    checkPipelineHotstart();
    const id = window.setInterval(
      checkPipelineHotstart,
      PIPELINE_HOTSTART_INTERVAL_MS,
    );
    return () => window.clearInterval(id);
  }, [ephemeralChannelId]);
}
