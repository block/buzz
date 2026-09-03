import type { ReactNode } from "react";

import type { TimelineMessage } from "@/features/messages/types";
import type { Channel, ManagedAgent } from "@/shared/api/types";

export function ProtectedGlobalOverlay() {
  return null;
}

export function ProtectedMessageAction(_props: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  return null;
}

export function ProtectedMessageActionsBoundary({
  children,
}: {
  children: ReactNode;
}) {
  return children;
}

export function ProtectedAgentBestieAction(_props: { agent: ManagedAgent }) {
  return null;
}

export function ProtectedBestieCardBadge(_props: {
  agent: ManagedAgent;
  isBestie: boolean;
}) {
  return null;
}

export function ProtectedBestieSidebarEntry() {
  return null;
}

export function useProtectedBestiePubkey(
  _agents: ManagedAgent[],
): string | null {
  return null;
}

export function useProtectedVisibleDirectMessages(
  channels: Channel[],
  _currentPubkey: string | undefined,
) {
  return channels;
}
