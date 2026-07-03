import * as React from "react";

import {
  createPanelReturnTargetStore,
  type PanelReturnTargetStore,
} from "@/shared/lib/panelReturnTarget";

/**
 * React binding for `createPanelReturnTargetStore`: a stable return-target
 * breadcrumb for mutually-exclusive panels, plus a reactive `hasTarget` so
 * back affordances can hide when there is nowhere to return to.
 *
 * The store identity is stable for the component's lifetime, so callbacks
 * can list it as a dependency without churning. A change of `resetKey`
 * (e.g. the active channel id) drops any recorded target, keeping
 * breadcrumbs from leaking across contexts.
 */
export function usePanelReturnTarget<T>(resetKey: unknown = null): {
  hasTarget: boolean;
  store: PanelReturnTargetStore<T>;
} {
  const storeRef = React.useRef<PanelReturnTargetStore<T> | null>(null);
  storeRef.current ??= createPanelReturnTargetStore<T>();
  const store = storeRef.current;

  const previousResetKeyRef = React.useRef(resetKey);
  if (previousResetKeyRef.current !== resetKey) {
    previousResetKeyRef.current = resetKey;
    // Render-safe silent drop: useSyncExternalStore re-reads the snapshot
    // during this same render, so no notification is needed (or allowed).
    store.reset();
  }

  const hasTarget = React.useSyncExternalStore(
    store.subscribe,
    () => store.peek() != null,
  );

  return { hasTarget, store };
}
