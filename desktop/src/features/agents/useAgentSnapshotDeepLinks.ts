import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { requestOpenSnapshotImport } from "@/features/agents/openSnapshotImportFromUrlEvent";
import { listenForAgentSnapshotDeepLinks } from "@/shared/deep-link";

/**
 * Sends `buzz://agent-import` files through the same preview-and-confirm flow
 * used by message attachment cards and manual file selection.
 */
export function useAgentSnapshotDeepLinks() {
  const { goAgents } = useAppNavigation();

  React.useEffect(() => {
    let cancelled = false;
    const unlistenPromise = listenForAgentSnapshotDeepLinks((payload) => {
      if (cancelled) return false;
      requestOpenSnapshotImport({
        fileBytes: payload.fileBytes,
        fileName: payload.fileName,
        snapshotKind: "agent",
      });
      void goAgents();
      return true;
    });
    return () => {
      cancelled = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [goAgents]);
}
