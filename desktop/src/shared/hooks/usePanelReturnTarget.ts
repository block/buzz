import * as React from "react";

import {
  createPanelReturnTargetStore,
  type PanelReturnTargetStore,
} from "@/shared/lib/panelReturnTarget";

/**
 * React binding for `createPanelReturnTargetStore`: a stable, render-silent
 * return-target breadcrumb for mutually-exclusive panels.
 *
 * The store lives in a ref, so capturing never triggers a render. A change of
 * `resetKey` (e.g. the active channel id) drops any recorded target, keeping
 * breadcrumbs from leaking across contexts.
 */
export function usePanelReturnTarget<T>(
  resetKey: unknown = null,
): PanelReturnTargetStore<T> {
  const storeRef = React.useRef<PanelReturnTargetStore<T> | null>(null);
  storeRef.current ??= createPanelReturnTargetStore<T>();

  const previousResetKeyRef = React.useRef(resetKey);
  if (previousResetKeyRef.current !== resetKey) {
    previousResetKeyRef.current = resetKey;
    storeRef.current.clear();
  }

  return storeRef.current;
}
