import {
  createContext,
  createElement,
  Fragment,
  type ReactNode,
  useContext,
  useMemo,
} from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { BestieGlobalOverlay } from "./bestie/BestieGlobalOverlay";
import { BestieProfileTrigger } from "./bestie/BestieProfileTrigger";
import { BestieCardBadge } from "./bestie/BestieCardBadge";
import { BestieMessageAction } from "./bestie/BestieMessageAction";
import { BestieProfileAction } from "./bestie/BestieProfileSection";
import { BestieSidebarEntry } from "./bestie/BestieSidebarEntry";
import { filterBestieDmChannels } from "./bestie/filterBestieDmChannels";
import { findAssignedLocalAgent } from "./bestie/findAssignedLocalAgent";
import { useBestieAssignmentQuery } from "./bestie/useBestie";
import { BestieDmChannelFrame } from "./bestie/BestieDmChannelFrame";
import { useBestieDmChannelExtras } from "./bestie/useBestieDmChannelExtras";
import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import type { ChannelScreenProps } from "@/features/channels/ui/ChannelScreen.types";
import { OpenClawWorkspaceRelayListener } from "./openclawWorkspaceMcp/OpenClawWorkspaceRelayListener";
import { OpenClawWorkspaceSettingsCard } from "./openclawWorkspaceMcp/OpenClawWorkspaceSettingsCard";
import { handleProtectedRelayPayload } from "./openclawWorkspaceMcp/handleRelayPayload";

const ProtectedMessageActionsContext = createContext(true);

export function ProtectedGlobalOverlay() {
  const bestieEnabled = useFeatureEnabled("bestie");
  return createElement(
    Fragment,
    null,
    bestieEnabled ? createElement(BestieGlobalOverlay) : null,
    createElement(OpenClawWorkspaceRelayListener),
  );
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

export function ProtectedBestieProfileTrigger() {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieProfileTrigger) : null;
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

export function useProtectedBestieChannelExtras(
  activeChannel: Channel | null | undefined,
) {
  const enabled = useFeatureEnabled("bestie");
  const extras = useBestieDmChannelExtras(enabled ? activeChannel : null);
  return enabled ? extras : { contextColumn: null, contextColumnOpen: false };
}

/**
 * ChannelScreen with Bestie DM project-home RHS: fixed category column +
 * idleAuxiliary slide only when drilling into Reminders/To-dos.
 */
export function ProtectedChannelScreen(props: ChannelScreenProps) {
  const enabled = useFeatureEnabled("bestie");
  const extras = useBestieDmChannelExtras(enabled ? props.activeChannel : null);
  const { contextColumn, contextColumnOpen, ...channelExtras } = extras;
  const screen = createElement(ChannelScreen, { ...props, ...channelExtras });
  if (!enabled || contextColumn == null) {
    return screen;
  }
  return createElement(BestieDmChannelFrame, {
    children: screen,
    column: contextColumn,
    open: contextColumnOpen,
  });
}

export function ProtectedOpenClawWorkspaceSettingsCard() {
  return createElement(OpenClawWorkspaceSettingsCard);
}

export { handleProtectedRelayPayload };
