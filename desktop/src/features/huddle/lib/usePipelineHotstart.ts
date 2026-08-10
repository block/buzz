import { invoke } from "@tauri-apps/api/core";
import * as React from "react";

import { useDocumentVisible } from "@/shared/lib/useDocumentVisible";

const PIPELINE_HOTSTART_INTERVAL_MS = 15_000;

/** Check if voice models finished downloading mid-huddle. */
export function usePipelineHotstart(ephemeralChannelId: string | null) {
  const documentVisible = useDocumentVisible();

  React.useEffect(() => {
    if (!ephemeralChannelId || !documentVisible) return;
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
  }, [documentVisible, ephemeralChannelId]);
}
