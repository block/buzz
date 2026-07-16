import * as React from "react";

import type { AddCommunityDeepLinkPayload } from "@/shared/deep-link";

export type AddCommunityPrefillRequest = AddCommunityDeepLinkPayload & {
  requestId: string;
};

let currentRequest: AddCommunityPrefillRequest | null = null;
const listeners = new Set<() => void>();
const availableListeners = new Set<() => void>();

export function requestAddCommunityPrefill(
  request: AddCommunityPrefillRequest,
): boolean {
  if (currentRequest) return false;
  currentRequest = request;
  for (const listener of listeners) listener();
  return true;
}

export function clearAddCommunityPrefill(requestId: string): void {
  if (!currentRequest || currentRequest.requestId !== requestId) return;
  currentRequest = null;
  for (const listener of listeners) listener();
  for (const listener of availableListeners) listener();
}

export function onAddCommunityPrefillAvailable(
  listener: () => void,
): () => void {
  availableListeners.add(listener);
  return () => availableListeners.delete(listener);
}

function useAddCommunityPrefill(): AddCommunityPrefillRequest | null {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => currentRequest,
    () => null,
  );
}

export function useAddCommunityDialogState() {
  const prefill = useAddCommunityPrefill();
  const [open, setOpen] = React.useState(false);

  React.useEffect(() => {
    if (prefill) setOpen(true);
  }, [prefill]);

  const onOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      setOpen(nextOpen);
      if (!nextOpen && prefill) clearAddCommunityPrefill(prefill.requestId);
    },
    [prefill],
  );

  return { prefill, open, onOpenChange, openDialog: () => setOpen(true) };
}
