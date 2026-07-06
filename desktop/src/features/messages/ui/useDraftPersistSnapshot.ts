import * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";

/**
 * Owns the `pendingImetaForPersistRef` — the ref that the draft-persist
 * cleanup reads when writing `pendingImeta` to the draft store.
 *
 * Two update paths:
 *
 * 1. **Render-time** (passive): `pendingImeta` is passed in and the ref is
 *    updated on every render, keeping it in sync with committed React state.
 *    This is the normal path: user adds/removes images during a mounted
 *    session; state commits; cleanup fires; ref is current.
 *
 * 2. **Synchronous restore** (active, via `snapshotPendingImeta`): when the
 *    draft-restore effect body loads a saved draft and calls
 *    `media.setPendingImeta(saved.pendingImeta)`, that state update is
 *    *asynchronous* — it won't commit until React re-renders.  React
 *    StrictMode (dev builds) simulates an unmount immediately after the
 *    effect body, before the re-render.  Without the synchronous write the
 *    cleanup would read `[]` and overwrite the just-restored images.
 *
 *    `snapshotPendingImeta` sets the ref synchronously inside the same
 *    microtask as the effect body, so the cleanup always sees the correct
 *    value regardless of when React commits the state update.
 *
 * The hook is extracted from `MessageComposer` so it can be imported and
 * exercised directly in a StrictMode lifecycle test without needing to mount
 * the full composer.
 */
export function useDraftPersistSnapshot(livePendingImeta: ImetaMedia[]): {
  pendingImetaForPersistRef: React.MutableRefObject<ImetaMedia[]>;
  snapshotPendingImeta: (imeta: ImetaMedia[]) => void;
} {
  const pendingImetaForPersistRef = React.useRef<ImetaMedia[]>([]);
  // Render-time update: keep the ref in sync with committed state so the
  // cleanup always reads the latest value during normal mounted operation.
  pendingImetaForPersistRef.current = livePendingImeta;

  const snapshotPendingImeta = React.useCallback(
    (imeta: ImetaMedia[]) => {
      pendingImetaForPersistRef.current = imeta;
    },
    // pendingImetaForPersistRef is stable (useRef); no deps needed.
    [],
  );

  return { pendingImetaForPersistRef, snapshotPendingImeta };
}
