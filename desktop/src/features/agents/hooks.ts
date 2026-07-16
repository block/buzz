import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { hasRunningAgentInCommunity } from "@/features/agents/agentRelayScope";
import {
  attachManagedAgentToChannel,
  createChannelManagedAgents,
  ensureChannelAgentPresetInChannel,
  provisionChannelManagedAgent,
} from "@/features/agents/channelAgents";
import { useActiveRelayUrl } from "@/features/communities/useCommunities";
import { resolveSnapshotAvatarPng } from "@/features/agents/ui/snapshotAvatarPng";
import {
  channelsQueryKey,
  upsertCachedChannelMember,
} from "@/features/channels/hooks";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import {
  createManagedAgent,
  deleteManagedAgent,
  discoverAcpRuntimes,
  discoverBackendProviders,
  discoverGitBashPrerequisite,
  discoverManagedAgentPrereqs,
  getAgentConfigSurface,
  getBakedBuildEnv,
  getBakedBuildEnvKeys,
  getChannelMembers,
  getManagedAgentLog,
  getRuntimeFileConfig,
  installAcpRuntime,
  listManagedAgents,
  listRelayAgents,
  updateManagedAgent,
} from "@/shared/api/tauri";
import {
  setManagedAgentAutoRestart,
  setManagedAgentStartOnAppLaunch,
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import {
  createPersona,
  deletePersona,
  exportAgentSnapshot,
  encodeAgentSnapshotForSend,
  previewAgentSnapshotImport,
  confirmAgentSnapshotImport,
  type AgentSnapshotImportPreview,
  type AgentSnapshotImportConfirm,
  type AgentSnapshotImportResult,
  type EncodedSnapshotPayload,
  type SnapshotMemoryLevel,
  type SnapshotFormat,
  listPersonas,
  setPersonaActive,
  updatePersona,
} from "@/shared/api/tauriPersonas";
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
  Channel,
  CreateManagedAgentInput,
  CreatePersonaInput,
  CreateTeamInput,
  ManagedAgent,
  UpdateManagedAgentInput,
  UpdatePersonaInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type {
  AttachManagedAgentToChannelInput,
  CreateChannelManagedAgentInput,
  CreateChannelManagedAgentsResult,
  CreateChannelManagedAgentResult,
  EnsureChannelAgentPresetInput,
  EnsureChannelAgentPresetResult,
  ProvisionChannelManagedAgentResult,
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
  ProvisionChannelManagedAgentResult,
} from "@/features/agents/channelAgents";

export const relayAgentsQueryKey = ["relay-agents"] as const;
export const managedAgentsQueryKey = ["managed-agents"] as const;
export const personasQueryKey = ["personas"] as const;
export const teamsQueryKey = ["teams"] as const;
export const acpRuntimesQueryKey = ["acp-runtimes"] as const;
export const managedAgentPrereqsQueryKey = ["managed-agent-prereqs"] as const;
export const backendProvidersQueryKey = ["backend-providers"] as const;
export const gitBashPrerequisiteQueryKey = ["git-bash-prerequisite"] as const;

type InvalidateAgentQueriesOptions = {
  refetchChannels?: boolean;
};

async function invalidateAgentQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string | null,
  options: InvalidateAgentQueriesOptions = {},
) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
    queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
    queryClient.invalidateQueries({
      queryKey: channelsQueryKey,
      refetchType: options.refetchChannels === false ? "none" : "active",
    }),
    ...(channelId
      ? [
          queryClient.invalidateQueries({
            queryKey: ["channels", channelId, "members"],
          }),
        ]
      : []),
  ]);
}

function refreshAgentQueriesInBackground(task: () => Promise<unknown>) {
  void task().catch((error) => {
    console.error("Failed to refresh agent queries", error);
  });
}

function invalidateAgentQueriesInBackground(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string | null,
  options?: InvalidateAgentQueriesOptions,
) {
  refreshAgentQueriesInBackground(() =>
    invalidateAgentQueries(queryClient, channelId, options),
  );
}

function isCachedDmChannel(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string | null,
) {
  if (!channelId) {
    return false;
  }

  return Boolean(
    queryClient
      .getQueryData<Channel[]>(channelsQueryKey)
      ?.some(
        (channel) => channel.id === channelId && channel.channelType === "dm",
      ),
  );
}

function invalidateManagedAgentQueriesInBackground(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  refreshAgentQueriesInBackground(() =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
    ]),
  );
}

export function useAcpRuntimesQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: acpRuntimesQueryKey,
    queryFn: discoverAcpRuntimes,
    staleTime: 60_000,
  });
}

export function useAvailableAcpRuntimes(options?: { enabled?: boolean }) {
  const query = useAcpRuntimesQuery(options);
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
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    },
  });
}

export function useGitBashPrerequisiteQuery() {
  return useQuery({
    queryKey: gitBashPrerequisiteQueryKey,
    queryFn: discoverGitBashPrerequisite,
    staleTime: 15_000,
  });
}

export function useBackendProvidersQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
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
    // No refetchInterval: inbound relay changes to personas emit
    // `agents-data-changed`, which `useAgentsDataRefresh` coalesces into an
    // invalidate (200ms window). The 30s poll was belt-and-suspenders on top of
    // that event path — redundant disk-read IPC.
  });
}

export function useManagedAgentPrereqsQuery(
  acpCommand: string,
  mcpCommand: string,
  options?: { enabled?: boolean },
) {
  const normalizedAcpCommand = acpCommand.trim();
  const normalizedMcpCommand = mcpCommand.trim();

  return useQuery({
    enabled: options?.enabled ?? true,
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
    // Relay agent profiles (kind:10100) are near-static and the backing
    // `list_relay_agents` command is an unfiltered relay query for the whole
    // profile set — mounted on ~13 always-live surfaces (channel screen,
    // members bar, mentions, sidebar, profile popovers), so a tight interval
    // re-pulls the full set app-wide. This poll is also the ONLY refresh path:
    // the `agents-data-changed` event fires only for local persona/team/managed
    // reconcile (kinds PERSONA/TEAM/MANAGED_AGENT), never for kind:10100. So we
    // keep polling but at a relaxed cadence and pause it while backgrounded.
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
    enabled: options?.enabled,
  });
}

export function useManagedAgentsQuery(options?: { enabled?: boolean }) {
  const activeRelayUrl = useActiveRelayUrl();
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: managedAgentsQueryKey,
    queryFn: listManagedAgents,
    staleTime: 5_000,
    refetchInterval: (query) => {
      const agents = query.state.data as ManagedAgent[] | undefined;
      // Only local "running" agents need polling: process state can change
      // with no relay event to signal it, so this poll is the only liveness
      // path for them. When nothing is running there IS an event path —
      // `agents-data-changed` (control-plane changes) — so the idle branch
      // drops its poll entirely rather than falling back to 30s. Scoped to
      // the active community's relay: agents left running in other
      // communities render no process state on this community's surfaces,
      // so they must not keep its poll alive.
      return hasRunningAgentInCommunity(agents, activeRelayUrl) ? 5_000 : false;
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

      // The users-batch delta fetch resolves from per-pubkey
      // ["users-batch-entry", pubkey] entries with their own 60s freshness —
      // invalidating the aggregate queries alone would just re-read the stale
      // entry. Evict it first so the re-run refetches this profile.
      evictUsersBatchEntries(queryClient, [lowerPubkey]);

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
    onSettled: async (_data, _error, variables) => {
      // Evict per-pubkey users-batch-entry caches for agents linked to this
      // persona so the subsequent batch invalidation refetches fresh profiles
      // instead of re-reading stale entries (mirrors useUpdateManagedAgentMutation).
      const agents = queryClient.getQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
      );
      if (agents) {
        const linkedPubkeys = agents
          .filter((a) => a.personaId === variables.id)
          .map((a) => a.pubkey.toLowerCase());
        evictUsersBatchEntries(queryClient, linkedPubkeys);
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        // Persona avatar changes re-sync linked agents' relay profiles;
        // invalidate cached user-profile and users-batch queries so the UI
        // picks up the updated kind:0 picture without waiting for staleTime
        // expiry — covers agent cards, message timelines, and member lists.
        queryClient.invalidateQueries({
          predicate: (query) =>
            query.queryKey[0] === "user-profile" ||
            query.queryKey[0] === "users-batch",
        }),
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
    onSuccess: (updated) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          if (!current) return current;
          return current.map((agent) =>
            agent.pubkey === updated.pubkey ? updated : agent,
          );
        },
      );
    },
    onSettled: () => {
      invalidateManagedAgentQueriesInBackground(queryClient);
    },
  });
}

export function useStopManagedAgentMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (pubkey: string) => stopManagedAgent(pubkey),
    onSettled: () => {
      invalidateManagedAgentQueriesInBackground(queryClient);
    },
  });
}

export function useSetManagedAgentAutoRestartMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      pubkey,
      autoRestartOnConfigChange,
    }: {
      pubkey: string;
      autoRestartOnConfigChange: boolean;
    }) => setManagedAgentAutoRestart(pubkey, autoRestartOnConfigChange),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
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
    mutationFn: async (
      input: AttachManagedAgentToChannelInput & { channelId?: string },
    ) => {
      const { channelId: capturedChannelId, ...rest } = input;
      const effectiveChannelId = capturedChannelId ?? channelId;
      if (!effectiveChannelId) {
        throw new Error("No channel selected.");
      }

      return attachManagedAgentToChannel(effectiveChannelId, rest);
    },
    onSuccess: (result, variables) => {
      const effectiveChannelId = variables.channelId ?? channelId;
      if (!effectiveChannelId) {
        return;
      }

      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannelMember(current, effectiveChannelId, {
          membershipAdded: result.membershipAdded,
          name: result.agent.name,
          pubkey: result.agent.pubkey,
        }),
      );
    },
    onSettled: (_data, _err, variables) => {
      // Invalidate the effective channel (the one the server actually mutated)
      // so its membership/agent state stays fresh. Invalidating the live
      // hook-closure channelId when the user has already switched away would
      // leave the compose-time channel stale.
      const effectiveChannelId = variables?.channelId ?? channelId;
      // Stream membership is already applied to channelsQueryKey by onSuccess,
      // so avoid replacing it with a lagged snapshot. DM membership is
      // immutable: adding the agent creates a separate conversation, so refresh
      // the list to discover that target instead of decorating the source DM.
      invalidateAgentQueriesInBackground(queryClient, effectiveChannelId, {
        refetchChannels: isCachedDmChannel(queryClient, effectiveChannelId),
      });
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
    onSettled: () => {
      invalidateAgentQueriesInBackground(queryClient, channelId);
    },
  });
}

export function useCreateChannelManagedAgentMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: CreateChannelManagedAgentInput & { channelId?: string },
    ): Promise<CreateChannelManagedAgentResult> => {
      const { channelId: capturedChannelId, ...rest } = input;
      const effectiveChannelId = capturedChannelId ?? channelId;
      if (!effectiveChannelId) {
        throw new Error("No channel selected.");
      }

      const result = await createChannelManagedAgents(effectiveChannelId, [
        rest,
      ]);
      const success = result.successes[0];
      if (success) {
        return success;
      }

      const failure = result.failures[0];
      throw new Error(failure?.error ?? "Could not create agent.");
    },
    onSuccess: (result, variables) => {
      const effectiveChannelId = variables.channelId ?? channelId;
      if (!effectiveChannelId) {
        return;
      }

      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannelMember(current, effectiveChannelId, {
          membershipAdded: result.membershipAdded,
          name: result.agent.name,
          pubkey: result.agent.pubkey,
        }),
      );
    },
    onSettled: (_data, _err, variables) => {
      const effectiveChannelId = variables?.channelId ?? channelId;
      // Stream membership is already applied to channelsQueryKey by onSuccess,
      // while immutable DM membership produces a separate conversation that
      // must be discovered by refetching the channel list.
      invalidateAgentQueriesInBackground(queryClient, effectiveChannelId, {
        refetchChannels: isCachedDmChannel(queryClient, effectiveChannelId),
      });
    },
  });
}

export function useProvisionChannelManagedAgentMutation(
  channelId: string | null,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: CreateChannelManagedAgentInput & { channelId?: string },
    ): Promise<ProvisionChannelManagedAgentResult> => {
      const { channelId: capturedChannelId, ...rest } = input;
      const effectiveChannelId = capturedChannelId ?? channelId;
      if (!effectiveChannelId) {
        throw new Error("No channel selected.");
      }

      const [managedAgents, members] = await Promise.all([
        listManagedAgents(),
        getChannelMembers(effectiveChannelId),
      ]);
      return provisionChannelManagedAgent(rest, {
        managedAgents,
        channelMemberPubkeys: new Set(
          members.map((member) => normalizePubkey(member.pubkey)),
        ),
      });
    },
    onSuccess: (result) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          const next = current ?? [];
          return [
            result.agent,
            ...next.filter((agent) => agent.pubkey !== result.agent.pubkey),
          ];
        },
      );
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
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
    onSettled: () => {
      invalidateAgentQueriesInBackground(queryClient, channelId);
    },
  });
}

export function useExportAgentSnapshotMutation() {
  return useMutation({
    mutationFn: async ({
      id,
      memoryLevel,
      format,
      memorySourcePubkey,
      avatarUrl,
    }: {
      id: string;
      memoryLevel: SnapshotMemoryLevel;
      format: SnapshotFormat;
      memorySourcePubkey?: string | null;
      avatarUrl?: string | null;
    }) => {
      const avatarPngDataUrl =
        format === "png"
          ? await resolveSnapshotAvatarPng(avatarUrl)
          : undefined;
      return exportAgentSnapshot(
        id,
        memoryLevel,
        format,
        memorySourcePubkey,
        avatarPngDataUrl,
      );
    },
  });
}

export function useEncodeAgentSnapshotForSendMutation() {
  return useMutation({
    mutationFn: ({
      id,
      memoryLevel,
      format,
      memorySourcePubkey,
      avatarPngDataUrl,
    }: {
      id: string;
      memoryLevel: SnapshotMemoryLevel;
      format: SnapshotFormat;
      memorySourcePubkey?: string | null;
      avatarPngDataUrl?: string;
    }) =>
      encodeAgentSnapshotForSend(
        id,
        memoryLevel,
        format,
        memorySourcePubkey,
        avatarPngDataUrl,
      ),
  });
}

export function usePreviewAgentSnapshotImportMutation() {
  return useMutation({
    mutationFn: ({
      fileBytes,
      fileName,
    }: {
      fileBytes: number[];
      fileName: string;
    }) => previewAgentSnapshotImport(fileBytes, fileName),
  });
}

export function useConfirmAgentSnapshotImportMutation() {
  return useMutation({
    mutationFn: (input: AgentSnapshotImportConfirm) =>
      confirmAgentSnapshotImport(input),
  });
}

// Re-export import types for consumers that import from hooks.
export type {
  AgentSnapshotImportPreview,
  AgentSnapshotImportConfirm,
  AgentSnapshotImportResult,
  EncodedSnapshotPayload,
};

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

export const agentConfigSurfaceQueryKey = (pubkey: string) =>
  ["agent-config-surface", pubkey] as const;

export function useAgentConfigSurface(pubkey: string | null) {
  return useQuery({
    queryKey: agentConfigSurfaceQueryKey(pubkey ?? ""),
    queryFn: () => getAgentConfigSurface(pubkey ?? ""),
    enabled: !!pubkey,
    staleTime: 10_000,
    refetchInterval: 30_000,
  });
}

export const runtimeFileConfigQueryKey = (runtimeId: string) =>
  ["runtime-file-config", runtimeId] as const;

/**
 * Query the file-layer config for a runtime (e.g. `~/.config/goose/config.yaml`).
 *
 * Used by Create/Edit/Persona dialogs to know which requirements are already
 * satisfied in the harness config file, so they can show "Set in goose config"
 * rather than surfacing a false required-field marker.
 *
 * Enabled only when `runtimeId` is non-empty and the dialog is open.
 */
export function useRuntimeFileConfigQuery(
  runtimeId: string,
  options?: { enabled?: boolean },
) {
  return useQuery({
    queryKey: runtimeFileConfigQueryKey(runtimeId),
    queryFn: () => getRuntimeFileConfig(runtimeId),
    enabled: (options?.enabled ?? true) && runtimeId.trim().length > 0,
    staleTime: 30_000,
    // File config rarely changes mid-session; no aggressive refetch needed.
    refetchInterval: false,
  });
}

export const bakedBuildEnvKeysQueryKey = ["baked-build-env-keys"] as const;
export const bakedBuildEnvQueryKey = ["baked-build-env"] as const;

/**
 * Query safely displayable baked build env entries. The backend masks secrets,
 * so this is only used for inherited provider/model/effort labels.
 */
export function useBakedBuildEnvQuery(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: bakedBuildEnvQueryKey,
    queryFn: () => getBakedBuildEnv(),
    enabled: options?.enabled ?? true,
    staleTime: Infinity,
    refetchInterval: false,
    retry: false,
  });
}

/**
 * Query the key names of baked build env vars.
 *
 * Internal (Block) builds bake provider credentials into the binary at compile
 * time. This query returns the *key names only* so dialogs can treat baked keys
 * as satisfying their requirements — mirroring the backend readiness gate.
 *
 * The value is a compile-time constant, so `staleTime: Infinity` is correct.
 * In web-dev and E2E contexts where the Tauri command doesn't exist the query
 * fails soft and resolves to `undefined` without crashing (same class as
 * `useRuntimeFileConfigQuery`).
 */
export function useBakedBuildEnvKeysQuery(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: bakedBuildEnvKeysQueryKey,
    queryFn: () => getBakedBuildEnvKeys(),
    enabled: options?.enabled ?? true,
    staleTime: Infinity,
    refetchInterval: false,
    retry: false,
  });
}

export function useTeamsQuery() {
  return useQuery({
    queryKey: teamsQueryKey,
    queryFn: listTeams,
    staleTime: 30_000,
    // No refetchInterval: inbound relay team changes emit `agents-data-changed`
    // (handled by useAgentsDataRefresh). Same redundant-poll removal as
    // usePersonasQuery.
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
