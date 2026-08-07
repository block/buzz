import * as React from "react";

import type { AgentInstallDeepLinkPayload } from "@/shared/deep-link";

/**
 * Prefill bridge for `buzz://install-agent?…` deep links, mirroring
 * `addCommunityPrefill`. The Rust handler queues the intent; the drain routes
 * it here, and the create-agent surface (`RequestedAgentCreateDialogs`) reads
 * the current request and opens its form PREFILLED.
 *
 * Security: this only prefills the owner's create-agent form — it never
 * auto-admits an agent or bypasses owner review. The owner still reviews and
 * saves the form in Desktop.
 */
export type AgentInstallPrefillRequest = AgentInstallDeepLinkPayload & {
  requestId: string;
};

let currentRequest: AgentInstallPrefillRequest | null = null;
const listeners = new Set<() => void>();
const availableListeners = new Set<() => void>();

export function requestAgentInstallPrefill(
  request: AgentInstallPrefillRequest,
): boolean {
  if (currentRequest) return false;
  currentRequest = request;
  for (const listener of listeners) listener();
  return true;
}

export function clearAgentInstallPrefill(requestId: string): void {
  if (!currentRequest || currentRequest.requestId !== requestId) return;
  currentRequest = null;
  for (const listener of listeners) listener();
  for (const listener of availableListeners) listener();
}

export function onAgentInstallPrefillAvailable(
  listener: () => void,
): () => void {
  availableListeners.add(listener);
  return () => availableListeners.delete(listener);
}

export function useAgentInstallPrefill(): AgentInstallPrefillRequest | null {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => currentRequest,
    () => null,
  );
}
