import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useAvailableAcpRuntimes,
  useCreateManagedAgentMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useRelayAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
  resolveStartRuntimeForDefinition,
} from "@/features/agents/lib/instanceInputForDefinition";
import { startManagedAgentWithRules } from "@/features/agents/lib/managedAgentControlActions";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { useOpenDmMutation } from "@/features/channels/hooks";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import { openPersonaConversation } from "./openPersonaConversation";

const EMPTY_DEFINITIONS: readonly AgentPersona[] = [];
const EMPTY_MANAGED_AGENTS: readonly ManagedAgent[] = [];

export function usePersonaConversation() {
  const { globalConfig } = useGlobalAgentConfig();
  const definitionsQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const createMutation = useCreateManagedAgentMutation();
  const startMutation = useStartManagedAgentMutation();
  const openDmMutation = useOpenDmMutation();
  const { goChannel } = useAppNavigation();
  const [pendingPersonaIds, setPendingPersonaIds] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const pendingRef = React.useRef(new Set<string>());
  const [error, setError] = React.useState<string | null>(null);

  const createAgent = createMutation.mutateAsync;
  const startAgent = startMutation.mutateAsync;
  const openDm = openDmMutation.mutateAsync;
  const definitions = definitionsQuery.data ?? EMPTY_DEFINITIONS;
  const managedAgents = managedAgentsQuery.data ?? EMPTY_MANAGED_AGENTS;
  const refetchManagedAgents = managedAgentsQuery.refetch;
  const refetchRelayAgents = relayAgentsQuery.refetch;
  const availableRuntimes = availableRuntimesQuery.data;
  const runtimesFetched = availableRuntimesQuery.isFetched;
  const refetchRuntimes = availableRuntimesQuery.refetch;

  const setPending = React.useCallback(
    (personaId: string, pending: boolean) => {
      const next = new Set(pendingRef.current);
      if (pending) next.add(personaId);
      else next.delete(personaId);
      pendingRef.current = next;
      setPendingPersonaIds(next);
    },
    [],
  );

  const open = React.useCallback(
    async (personaId: string) => {
      if (pendingRef.current.has(personaId)) return;
      setPending(personaId, true);
      setError(null);
      try {
        await openPersonaConversation(personaId, {
          definitions,
          managedAgents,
          buildInput: async (definition) => {
            const runtimes = await availableRuntimesForStart({
              data: availableRuntimes,
              isFetched: runtimesFetched,
              refetch: refetchRuntimes,
            });
            const { runtime } = resolveStartRuntimeForDefinition(
              definition,
              runtimes,
              globalConfig.preferred_runtime,
            );
            return buildInstanceInputForDefinition(definition, runtime);
          },
          createAgent,
          startAgent: async (pubkey) => {
            const agent = managedAgents.find(
              (candidate) => candidate.pubkey === pubkey,
            );
            if (!agent) throw new Error("Managed adviser instance not found.");
            await startManagedAgentWithRules({
              agent,
              startManagedAgent: startAgent,
            });
          },
          openDm: (pubkeys) => openDm({ pubkeys }),
          navigate: goChannel,
          refetch: async () => {
            await Promise.all([refetchManagedAgents(), refetchRelayAgents()]);
          },
        });
      } catch (cause) {
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not open the adviser conversation.",
        );
      } finally {
        setPending(personaId, false);
      }
    },
    [
      availableRuntimes,
      createAgent,
      definitions,
      globalConfig.preferred_runtime,
      goChannel,
      managedAgents,
      openDm,
      refetchManagedAgents,
      refetchRelayAgents,
      refetchRuntimes,
      runtimesFetched,
      setPending,
      startAgent,
    ],
  );

  return { error, open, pendingPersonaIds };
}
