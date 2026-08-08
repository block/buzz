import * as React from "react";

import { listenForDesktopControlImports } from "@/shared/desktop-control";
import { requestOpenSnapshotImport } from "./openSnapshotImportFromUrlEvent";

/** Route retained local-control drafts through the existing team review UI. */
export function useDesktopControlImports(openAgents: () => void) {
  React.useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenForDesktopControlImports((request) => {
      requestOpenSnapshotImport({
        fileBytes: request.fileBytes,
        fileName: request.fileName,
        snapshotKind: "team",
        desktopControlRequestId: request.requestId,
      });
      openAgents();
    }).then((nextUnlisten) => {
      if (disposed) nextUnlisten();
      else unlisten = nextUnlisten;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [openAgents]);
}
