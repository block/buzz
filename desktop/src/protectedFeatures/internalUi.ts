import {
  createContext,
  createElement,
  type ReactNode,
  useContext,
  useMemo,
} from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { BestieGlobalOverlay } from "./bestie/BestieGlobalOverlay";
import { BestieCardBadge } from "./bestie/BestieCardBadge";
import { BestieMessageAction } from "./bestie/BestieMessageAction";
import { BestieProfileAction } from "./bestie/BestieProfileSection";
import { BestieSidebarEntry } from "./bestie/BestieSidebarEntry";
import { filterBestieDmChannels } from "./bestie/filterBestieDmChannels";
import { findAssignedLocalAgent } from "./bestie/findAssignedLocalAgent";
import { useBestieAssignmentQuery } from "./bestie/useBestie";

const ProtectedMessageActionsContext = createContext(true);

export function ProtectedGlobalOverlay() {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieGlobalOverlay) : null;
}

export function ProtectedMessageAction(props: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  const enabled = useFeatureEnabled("bestie");
  const actionsAllowed = useContext(ProtectedMessageActionsContext);
  return enabled && actionsAllowed
    ? createElement(BestieMessageAction, props)
    : null;
}

export function ProtectedMessageActionsBoundary({
  children,
}: {
  children: ReactNode;
}) {
  return createElement(
    ProtectedMessageActionsContext.Provider,
    { value: false },
    children,
  );
}

export function ProtectedAgentBestieAction(props: { agent: ManagedAgent }) {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieProfileAction, props) : null;
}

export function ProtectedBestieCardBadge(props: {
  agent: ManagedAgent;
  isBestie: boolean;
}) {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieCardBadge, props) : null;
}

export function ProtectedBestieSidebarEntry() {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieSidebarEntry) : null;
}

export function useProtectedBestiePubkey(agents: ManagedAgent[]) {
  const enabled = useFeatureEnabled("bestie");
  const { assignmentQuery } = useBestieAssignmentQuery(enabled);
  if (!enabled) return null;
  return findAssignedLocalAgent(agents, assignmentQuery.data)?.pubkey ?? null;
}

export function useProtectedVisibleDirectMessages(
  channels: Channel[],
  currentPubkey: string | undefined,
) {
  const enabled = useFeatureEnabled("bestie");
  const managedAgentsQuery = useManagedAgentsQuery({ enabled });
  const bestiePubkey = useProtectedBestiePubkey(managedAgentsQuery.data ?? []);

  return useMemo(
    () => filterBestieDmChannels(channels, currentPubkey, bestiePubkey),
    [bestiePubkey, channels, currentPubkey],
  );
}
