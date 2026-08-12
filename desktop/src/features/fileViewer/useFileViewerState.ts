import * as React from "react";

import {
  type FileViewerState,
  getFileViewerState,
  subscribeFileViewer,
} from "./fileViewerStore";

/** Reactive file-viewer snapshot for panel hosts and the panel itself. */
export function useFileViewerState(): FileViewerState {
  return React.useSyncExternalStore(
    subscribeFileViewer,
    getFileViewerState,
    getFileViewerState,
  );
}
