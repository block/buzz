import * as React from "react";

import type { AddCommunityDeepLinkPayload } from "@/shared/deep-link";

export type AddCommunityPrefillRequest = AddCommunityDeepLinkPayload & {
  requestId: string;
};

let currentRequest: AddCommunityPrefillRequest | null = null;
const listeners = new Set<() => void>();

export function requestAddCommunityPrefill(
  request: AddCommunityPrefillRequest,
): boolean {
  currentRequest = request;
  for (const listener of listeners) listener();
  return true;
}

export function clearAddCommunityPrefill(requestId: string): void {
  if (!currentRequest || currentRequest.requestId !== requestId) return;
  currentRequest = null;
  for (const listener of listeners) listener();
}

export function useAddCommunityPrefill(): AddCommunityPrefillRequest | null {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => currentRequest,
    () => null,
  );
}
