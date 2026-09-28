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
  return createElement(
    Fragment,
    null,
    createElement(BestieGlobalOverlay),
    createElement(OpenClawWorkspaceRelayListener),
  );
}

export function ProtectedMessageAction(props: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  const actionsAllowed = useContext(ProtectedMessageActionsContext);
  return actionsAllowed ? createElement(BestieMessageAction, props) : null;
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
  return createElement(BestieProfileAction, props);
}

export function ProtectedBestieCardBadge(props: {
  agent: ManagedAgent;
  isBestie: boolean;
}) {
  return createElement(BestieCardBadge, props);
}

export function ProtectedBestieSidebarEntry() {
  return createElement(BestieSidebarEntry);
}

export function ProtectedBestieProfileTrigger() {
  return createElement(BestieProfileTrigger);
}

export function useProtectedBestiePubkey(agents: ManagedAgent[]) {
  const { assignmentQuery } = useBestieAssignmentQuery(true);
  return findAssignedLocalAgent(agents, assignmentQuery.data)?.pubkey ?? null;
}

export function useProtectedVisibleDirectMessages(
  channels: Channel[],
  currentPubkey: string | undefined,
) {
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const bestiePubkey = useProtectedBestiePubkey(managedAgentsQuery.data ?? []);

  return useMemo(
    () => filterBestieDmChannels(channels, currentPubkey, bestiePubkey),
    [bestiePubkey, channels, currentPubkey],
  );
}

export function useProtectedBestieChannelExtras(
  activeChannel: Channel | null | undefined,
) {
  return useBestieDmChannelExtras(activeChannel ?? null);
}

/**
 * ChannelScreen with Bestie DM project-home RHS: fixed category column +
 * idleAuxiliary slide only when drilling into Reminders/To-dos.
 */
export function ProtectedChannelScreen(props: ChannelScreenProps) {
  const extras = useBestieDmChannelExtras(props.activeChannel ?? null);
  const { contextColumn, contextColumnOpen, ...channelExtras } = extras;
  const screen = createElement(ChannelScreen, { ...props, ...channelExtras });
  if (contextColumn == null) {
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
