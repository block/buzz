import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  attachManagedAgentToChannel,
  createChannelManagedAgent,
  createChannelManagedAgents,
  ensureChannelAgentPresetInChannel,
} from "@/features/agents/channelAgents";
import { channelsQueryKey } from "@/features/channels/hooks";
import {
  createManagedAgent,
  deleteManagedAgent,
  discoverAcpRuntimes,
  discoverBackendProviders,
  discoverManagedAgentPrereqs,
  getManagedAgentLog,
  installAcpRuntime,
  listManagedAgents,
  listRelayAgents,
  startManagedAgent,
  stopManagedAgent,
  updateManagedAgent,
} from "@/shared/api/tauri";
import {
  createPersona,
  deletePersona,
  exportPersonaToJson,
  listPersonas,
  setPersonaActive,
  updatePersona,
} from "@/shared/api/tauriPersonas";
import { setManagedAgentStartOnAppLaunch } from "@/shared/api/tauriManagedAgents";
import {
  createTeam,
  deleteTeam,
  listTeams,
  updateTeam,
} from "@/shared/api/tauriTeams";
import type {
  AcpRuntime,
  AgentPersona,
  AgentTeam,
  CreateManagedAgentInput,
  CreatePersonaInput,
  CreateTeamInput,
  ManagedAgent,
  UpdateManagedAgentInput,
  UpdatePersonaInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import type {
  AttachManagedAgentToChannelInput,
  AttachManagedAgentToChannelResult,
  CreateChannelManagedAgentInput,
  CreateChannelManagedAgentsResult,
  CreateChannelManagedAgentResult,
  EnsureChannelAgentPresetInput,
  EnsureChannelAgentPresetResult,
} from "@/features/agents/channelAgents";
export { findReusableAgent } from "@/features/agents/agentReuse";
export type {
  AttachManagedAgentToChannelInput,
  AttachManagedAgentToChannelResult,
  CreateChannelManagedAgentInput,
  CreateChannelManagedAgentBatchFailure,
  CreateChannelManagedAgentsResult,
  CreateChannelManagedAgentResult,
  EnsureChannelAgentPresetInput,
  EnsureChannelAgentPresetResult,
} from "@/features/agents/channelAgents";

export const relayAgentsQueryKey = ["relay-agents"] as const;
export const managedAgentsQueryKey = ["managed-agents"] as const;
export const personasQueryKey = ["personas"] as const;
export const teamsQueryKey = ["teams"] as const;
export const acpRuntimesQueryKey = ["acp-runtimes"] as const;
export const managedAgentPrereqsQueryKey = ["managed-agent-prereqs"] as const;
export const backendProvidersQueryKey = ["backend-providers"] as const;

export type EnsureGooseInChannelResult = AttachManagedAgentToChannelResult & {
  created: boolean;
};

async function invalidateAgentQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string | null,
) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
    queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
    queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
    ...(channelId
      ? [
          queryClient.invalidateQueries({
            queryKey: ["channels", channelId, "members"],
          }),
        ]
      : []),
  ]);
}

export function useAcpRuntimesQuery() {
  return useQuery({
    queryKey: acpRuntimesQueryKey,
    queryFn: discoverAcpRuntimes,
    staleTime: 60_000,
  });
}

export function useAvailableAcpRuntimes() {
  const query = useAcpRuntimesQuery();
  const available = React.useMemo(
    () =>
      (query.data ?? []).filter(
        (p): p is AcpRuntime => p.availability === "available",
      ),
    [query.data],
  );
  return { ...query, data: available };
}

export function useInstallAcpRuntimeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (runtimeId: string) => installAcpRuntime(runtimeId),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: acpRuntimesQueryKey });
    },
  });
}

export function useBackendProvidersQuery() {
  return useQuery({
    queryKey: backendProvidersQueryKey,
    queryFn: discoverBackendProviders,
    staleTime: 30_000,
  });
}

export function usePersonasQuery() {
  return useQuery({
    queryKey: personasQueryKey,
    queryFn: listPersonas,
    staleTime: 30_000,
    refetchInterval: 30_000,
  });
}

export function useManagedAgentPrereqsQuery(
  acpCommand: string,
  mcpCommand: string,
) {
  const normalizedAcpCommand = acpCommand.trim();
  const normalizedMcpCommand = mcpCommand.trim();

  return useQuery({
    queryKey: [
      ...managedAgentPrereqsQueryKey,
      normalizedAcpCommand,
      normalizedMcpCommand,
    ],
    queryFn: () =>
      discoverManagedAgentPrereqs({
        acpCommand: normalizedAcpCommand || undefined,
        mcpCommand: normalizedMcpCommand || undefined,
      }),
    staleTime: 15_000,
  });
}

export function useRelayAgentsQuery(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: relayAgentsQueryKey,
    queryFn: listRelayAgents,
    staleTime: 30_000,
    refetchInterval: 30_000,
    enabled: options?.enabled,
  });
}

export function useManagedAgentsQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: managedAgentsQueryKey,
    queryFn: listManagedAgents,
    staleTime: 5_000,
    refetchInterval: (query) => {
      const agents = query.state.data as ManagedAgent[] | undefined;
      // Only local "running" agents need fast polling (process state can
      // change). "deployed" is static control-plane state — presence polling
      // handles the live signal for remote agents separately.
      return agents?.some((agent) => agent.status === "running")
        ? 5_000
        : 30_000;
    },
  });
}

export function useCreateManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateManagedAgentInput) => createManagedAgent(input),
    onSuccess: (created) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          const next = current ?? [];

          return [
            created.agent,
            ...next.filter((agent) => agent.pubkey !== created.agent.pubkey),
          ];
        },
      );
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      await queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
    },
  });
}

export function useUpdateManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: UpdateManagedAgentInput) => updateManagedAgent(input),
    onSuccess: (result) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          if (!current) return current;
          return current.map((agent) =>
            agent.pubkey === result.agent.pubkey ? result.agent : agent,
          );
        },
      );
    },
    onSettled: async (_data, _error, variables) => {
      // Backend republishes kind:0 on a name change (sync_managed_agent_profile),
      // so the relay has fresh profile data — but the desktop's React Query cache
      // for ["user-profile", pubkey] has a 60s staleTime and will not refetch on
      // its own. Invalidate explicitly so the profile pane re-renders against
      // the new display name / about / NIP-05 immediately. Also poke any
      // ["users-batch", ...] entries that include this pubkey so sidebar member
      // rows, channel header chips, and message author labels refresh too.
      const lowerPubkey = variables.pubkey.toLowerCase();

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
        queryClient.invalidateQueries({
          queryKey: ["user-profile", lowerPubkey],
        }),
        queryClient.invalidateQueries({
          predicate: (query) =>
            query.queryKey[0] === "users-batch" &&
            query.queryKey.includes(lowerPubkey),
        }),
      ]);
    },
  });
}

export function useCreatePersonaMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreatePersonaInput) => createPersona(input),
    onSuccess: (created) => {
      queryClient.setQueryData<AgentPersona[]>(personasQueryKey, (current) => {
        const next = current ?? [];
        return [
          created,
          ...next.filter((persona) => persona.id !== created.id),
        ];
      });
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: personasQueryKey });
    },
  });
}

export function useUpdatePersonaMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: UpdatePersonaInput) => updatePersona(input),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
    },
  });
}

export function useDeletePersonaMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deletePersona(id),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
    },
  });
}

export function useSetPersonaActiveMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, active }: { id: string; active: boolean }) =>
      setPersonaActive(id, active),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: teamsQueryKey }),
      ]);
    },
  });
}

export function useStartManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pubkey: string) => startManagedAgent(pubkey),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      await queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
    },
  });
}

export function useStopManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pubkey: string) => stopManagedAgent(pubkey),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      await queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
    },
  });
}

export function useSetManagedAgentStartOnAppLaunchMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      pubkey,
      startOnAppLaunch,
    }: {
      pubkey: string;
      startOnAppLaunch: boolean;
    }) => setManagedAgentStartOnAppLaunch(pubkey, startOnAppLaunch),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      await queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
    },
  });
}

export function useDeleteManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      pubkey,
      forceRemoteDelete,
    }: {
      pubkey: string;
      forceRemoteDelete?: boolean;
    }) => deleteManagedAgent(pubkey, forceRemoteDelete),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      await queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
    },
  });
}

export function useAttachManagedAgentToChannelMutation(
  channelId: string | null,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: AttachManagedAgentToChannelInput) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return attachManagedAgentToChannel(channelId, input);
    },
    onSettled: async () => {
      await invalidateAgentQueries(queryClient, channelId);
    },
  });
}

export function useEnsureChannelAgentPresetMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: EnsureChannelAgentPresetInput,
    ): Promise<EnsureChannelAgentPresetResult> => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return ensureChannelAgentPresetInChannel(channelId, input);
    },
    onSettled: async () => {
      await invalidateAgentQueries(queryClient, channelId);
    },
  });
}

export function useCreateChannelManagedAgentMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: CreateChannelManagedAgentInput,
    ): Promise<CreateChannelManagedAgentResult> => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return createChannelManagedAgent(channelId, input);
    },
    onSettled: async () => {
      await invalidateAgentQueries(queryClient, channelId);
    },
  });
}

export function useCreateChannelManagedAgentsMutation(
  channelId: string | null,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      inputs: readonly CreateChannelManagedAgentInput[],
    ): Promise<CreateChannelManagedAgentsResult> => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      return createChannelManagedAgents(channelId, inputs);
    },
    onSettled: async () => {
      await invalidateAgentQueries(queryClient, channelId);
    },
  });
}

export function useEnsureGooseInChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (): Promise<EnsureGooseInChannelResult> => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      const attached = await ensureChannelAgentPresetInChannel(channelId, {
        runtime: {
          id: "goose",
          label: "Goose",
          command: "goose",
          defaultArgs: ["acp"],
          mcpCommand: null,
        },
        role: "bot",
      });

      return {
        agent: attached.agent,
        membershipAdded: attached.membershipAdded,
        restarted: attached.restarted,
        started: attached.started,
        created: attached.created,
      };
    },
    onSettled: async () => {
      await invalidateAgentQueries(queryClient, channelId);
    },
  });
}

export function useExportPersonaJsonMutation() {
  return useMutation({
    mutationFn: (id: string) => exportPersonaToJson(id),
  });
}

export function useManagedAgentLogQuery(
  pubkey: string | null,
  lineCount = 120,
) {
  return useQuery({
    queryKey: ["managed-agent-log", pubkey, lineCount],
    queryFn: () => getManagedAgentLog(pubkey as string, lineCount),
    enabled: pubkey !== null,
    retry: false,
    staleTime: 3_000,
    refetchInterval: pubkey ? 30_000 : false,
  });
}

export function useTeamsQuery() {
  return useQuery({
    queryKey: teamsQueryKey,
    queryFn: listTeams,
    staleTime: 30_000,
    refetchInterval: 30_000,
  });
}

export function useCreateTeamMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateTeamInput) => createTeam(input),
    onSuccess: (created) => {
      queryClient.setQueryData<AgentTeam[]>(teamsQueryKey, (current) => {
        const next = current ?? [];
        return [created, ...next.filter((team) => team.id !== created.id)];
      });
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: teamsQueryKey });
    },
  });
}

export function useUpdateTeamMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: UpdateTeamInput) => updateTeam(input),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: teamsQueryKey });
    },
  });
}

export function useDeleteTeamMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteTeam(id),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: teamsQueryKey });
    },
  });
}
