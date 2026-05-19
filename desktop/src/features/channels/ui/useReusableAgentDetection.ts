import * as React from "react";

import {
  findReusableAgent,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { AcpProvider, ManagedAgent } from "@/shared/api/types";

type Persona = { id: string };

/**
 * Detects whether a reusable managed agent exists for the current dialog
 * selection. Returns the reusable agent (if any) so the UI can show the
 * "reuse vs create new" guardrail.
 */
export function useReusableAgentDetection(
  channelId: string | null,
  enabled: boolean,
  selectedProvider: AcpProvider | null,
  selectedPersonas: readonly Persona[],
  includeGeneric: boolean,
  customPrompt: string,
): ManagedAgent | undefined {
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelMembersQuery = useChannelMembersQuery(channelId, enabled);

  return React.useMemo(() => {
    const agents = managedAgentsQuery.data;
    const members = channelMembersQuery.data;
    if (!agents || !members || !selectedProvider) return undefined;
    const memberPubkeys = new Set(
      members.map((m) => normalizePubkey(m.pubkey)),
    );

    // For persona selection: check the first selected persona
    if (selectedPersonas.length === 1 && !includeGeneric) {
      return findReusableAgent(agents, memberPubkeys, {
        personaId: selectedPersonas[0].id,
        command: selectedProvider.command,
      });
    }

    // For generic agent with no custom prompt
    if (
      includeGeneric &&
      selectedPersonas.length === 0 &&
      !customPrompt.trim()
    ) {
      return findReusableAgent(agents, memberPubkeys, {
        command: selectedProvider.command,
        systemPrompt: customPrompt,
      });
    }

    return undefined;
  }, [
    managedAgentsQuery.data,
    channelMembersQuery.data,
    selectedProvider,
    selectedPersonas,
    includeGeneric,
    customPrompt,
  ]);
}
