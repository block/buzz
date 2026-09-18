import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  connectAcpRuntime,
  discoverAcpAuthMethods,
} from "@/shared/api/tauriAgentAuth";
import {
  attachManagedAgentToChannel,
  createChannelManagedAgents,
  ensureChannelAgentPresetInChannel,
  provisionChannelManagedAgent,
} from "@/features/agents/channelAgents";
import {
  channelsQueryKey,
  upsertCachedChannelMember,
} from "@/features/channels/hooks";
import { updateCachedChannelMemberDisplayName } from "@/features/channels/channelMemberProfileCache";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import {
  useAppFocused,
  useFocusedRefetchInterval,
} from "@/shared/lib/useDocumentVisible";
import {
  createManagedAgent,
  deleteManagedAgent,
  deleteCustomHarness,
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
  invokeTauri,
  listManagedAgents,
  listRelayAgents,
  saveCustomHarness,
  updateManagedAgent,
} from "@/shared/api/tauri";
import type { HarnessDefinitionInput } from "@/shared/api/tauri";
import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";
import {
  setManagedAgentAutoRestart,
  setManagedAgentStartOnAppLaunch,
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import { bootstrapManagedAgentRuntimePairs } from "@/features/agents/managedAgentRuntimeHooks";
import {
  acpRuntimesQueryKey,
  applyBootWarmGate,
  getBootWarmSnapshot,
  refreshAcpRuntimes,
  subscribeBootWarm,
} from "@/features/agents/acpRuntimesQuery";
export {
  useAcpRuntimesQueryForced,
  useRetryBootWarm,
} from "@/features/agents/acpRuntimesQuery";
import {
  createPersona,
  deletePersona,
  listPersonas,
  setPersonaActive,
  updatePersona,
} from "@/shared/api/tauriPersonas";
import { teamsQueryKey } from "@/features/agents/teamHooks";
import type {
  AcpRuntime,
  AgentPersona,
  Channel,
  CreateManagedAgentInput,
  CreatePersonaInput,
  ManagedAgent,
  UpdateManagedAgentInput,
  UpdatePersonaInput,
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
export {
  teamsQueryKey,
  useCreateTeamMutation,
  useDeleteTeamMutation,
  useTeamsQuery,
  useUpdateTeamMutation,
} from "@/features/agents/teamHooks";
export * from "@/features/agents/snapshotHooks";
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

export const AGENTS_FOCUS_STALE_TIME_MS = 5 * 60_000;

/**
 * Matches the query's 30 s poll so a focus-return refetches anything older
 * than one poll tick. This detail-view query mounts only on the agent-detail
 * surface and is not part of the app-wide focus storm.
 */
export const MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS = 30_000;

/** Focus-refetch policy for relay-agent and managed-agent queries; consumed by focusRefetchPolicy.test.mjs. */
export const agentsFocusRefetchPolicy = {
  staleTime: AGENTS_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;

/** Focus-refetch policy for the managed-agent-log query; consumed by focusRefetchPolicy.test.mjs. */
export const managedAgentLogFocusRefetchPolicy = {
  staleTime: MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;

export const relayAgentsQueryKey = ["relay-agents"] as const;
export const managedAgentsQueryKey = ["managed-agents"] as const;
export const personasQueryKey = ["personas"] as const;
export const acpAuthMethodsQueryKey = ["acp-auth-methods"] as const;
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

/**
 * Discover the ACP runtime catalog.
 *
 * This always serves the **cheap** backend path: the last cached runtime
 * availability + auth statuses, no process spawns, low-millisecond. Hot
 * surfaces (channel switch, composer, member bar) render from cache — a
 * 30-minute `staleTime` keeps channel switches from re-triggering discovery.
 *
 * Fresh auth/version state (Settings, onboarding sign-in, post-mutation) comes
 * from `refreshAcpRuntimes`, which runs the expensive forced path explicitly
 * and writes the result into this same cache. Keeping the query's own
 * `queryFn` cheap guarantees an automatic staleness refetch never re-runs the
 * probe pipeline.
 */
export function useAcpRuntimesQuery(options?: { enabled?: boolean }) {
  const query = useQuery({
    enabled: options?.enabled ?? true,
    queryKey: acpRuntimesQueryKey,
    queryFn: () => discoverAcpRuntimes(),
    staleTime: 30 * 60_000,
  });
  // Overlay the launch boot-warm gate so cheap consumers never present a cold
  // catalog as authoritative: until the first forced pass settles, an un-warmed
  // catalog reads as loading (`pending`) or a retryable error (`failed`) rather
  // than "every harness not installed". `applyBootWarmGate` preserves an
  // already-good list and passes through untouched while idle/settled.
  const bootWarm = React.useSyncExternalStore(
    subscribeBootWarm,
    getBootWarmSnapshot,
    getBootWarmSnapshot,
  );
  return applyBootWarmGate(query, bootWarm);
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

export function useAcpAuthMethodsQuery(
  runtimeId: string,
  options?: { enabled?: boolean },
) {
  return useQuery({
    enabled: (options?.enabled ?? true) && runtimeId.trim().length > 0,
    queryKey: [...acpAuthMethodsQueryKey, runtimeId],
    queryFn: () => discoverAcpAuthMethods(runtimeId),
    staleTime: 30_000,
  });
}

export function useConnectAcpRuntimeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { runtimeId: string; methodId: string }) =>
      connectAcpRuntime(input.runtimeId, input.methodId),
    onSettled: () => {
      void refreshAcpRuntimes(queryClient);
      void queryClient.invalidateQueries({ queryKey: acpAuthMethodsQueryKey });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    },
  });
}

export function useInstallAcpRuntimeMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (runtimeId: string) => installAcpRuntime(runtimeId),
    onSettled: () => {
      void refreshAcpRuntimes(queryClient);
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    },
  });
}

export function useSaveCustomHarnessMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      definition,
      originalId,
    }: {
      definition: HarnessDefinitionInput;
      originalId?: string;
    }) => saveCustomHarness(definition, originalId),
    onSettled: () => {
      void refreshAcpRuntimes(queryClient);
    },
  });
}

export function useDeleteCustomHarnessMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteCustomHarness(id),
    onSettled: () => {
      void refreshAcpRuntimes(queryClient);
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

export function usePersonasQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
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
  const refetchInterval = useFocusedRefetchInterval(AGENTS_FOCUS_STALE_TIME_MS);
  return useQuery({
    queryKey: relayAgentsQueryKey,
    queryFn: listRelayAgents,
    // Relay agent discovery is scoped to the viewer's relay-signed channel
    // memberships, then resolves exact agent/profile/policy coordinates in
    // protocol-sized batches. Polling remains the only refresh path for remote
    // changes, so keep it relaxed and pause while backgrounded.
    refetchInterval,
    enabled: options?.enabled,
    ...agentsFocusRefetchPolicy,
  });
}

export function useManagedAgentsQuery(options?: { enabled?: boolean }) {
  const appFocused = useAppFocused();
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: managedAgentsQueryKey,
    queryFn: listManagedAgents,
    refetchInterval: (query) => {
      if (!appFocused) return false;
      const agents = query.state.data as ManagedAgent[] | undefined;
      // Only local "running" agents need polling: process state can change
      // with no relay event to signal it, so this poll is the only liveness
      // path for them. When nothing is running there IS an event path —
      // `agents-data-changed` (control-plane changes) — so the idle branch
      // drops its poll entirely rather than falling back to 30s.
      return agents?.some((agent) => agent.status === "running")
        ? 5_000
        : false;
    },
    ...agentsFocusRefetchPolicy,
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

      // The create command spawns only the active community's pair — kick a
      // reconcile so the new agent gets a lazy pair in every other community.
      if (created.agent.backend.type === "local") {
        bootstrapManagedAgentRuntimePairs(queryClient);
      }
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
    onSuccess: async (result, variables) => {
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          if (!current) return current;
          return current.map((agent) =>
            agent.pubkey === result.agent.pubkey ? result.agent : agent,
          );
        },
      );

      if (variables.name !== undefined && !result.profileSyncError) {
        await updateCachedChannelMemberDisplayName(
          queryClient,
          result.agent.pubkey,
          result.agent.name,
        );
      }
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
    // Accepts a bare pubkey, or an object carrying the tenant scope a
    // long-lived callback captured before its first await (the backend
    // fails closed on a mid-flight community/identity switch).
    mutationFn: (
      input:
        | string
        | {
            pubkey: string;
            expectedRelayUrl?: string;
            expectedSignerPubkey?: string;
            replayFloorUnix?: number;
          },
    ) =>
      typeof input === "string"
        ? startManagedAgent(input)
        : startManagedAgent(input.pubkey, {
            expectedRelayUrl: input.expectedRelayUrl,
            expectedSignerPubkey: input.expectedSignerPubkey,
            replayFloorUnix: input.replayFloorUnix,
          }),
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
      void invokeTauri("sync_agents_to_active_huddle", {
        channelId: effectiveChannelId,
        agentPubkeys: [result.agent.pubkey],
      }).catch((error) => {
        console.warn("Could not sync attached agent into Huddle:", error);
      });
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

      const [managedAgents, members, personas] = await Promise.all([
        listManagedAgents(),
        getChannelMembers(effectiveChannelId),
        rest.personaId && rest.respondTo === undefined
          ? listPersonas()
          : Promise.resolve([]),
      ]);
      return provisionChannelManagedAgent(rest, {
        managedAgents,
        personas,
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

export function useManagedAgentLogQuery(
  pubkey: string | null,
  lineCount = 120,
) {
  const refetchInterval = useFocusedRefetchInterval(
    pubkey ? MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS : false,
  );
  return useQuery({
    queryKey: ["managed-agent-log", pubkey, lineCount],
    queryFn: () => getManagedAgentLog(pubkey as string, lineCount),
    enabled: pubkey !== null,
    retry: false,
    refetchInterval,
    ...managedAgentLogFocusRefetchPolicy,
  });
}

export const agentConfigSurfaceQueryKey = (pubkey: string) =>
  ["agent-config-surface", pubkey] as const;

export function useAgentConfigSurface(pubkey: string | null) {
  const refetchInterval = useFocusedRefetchInterval(30_000);
  return useQuery({
    queryKey: agentConfigSurfaceQueryKey(pubkey ?? ""),
    queryFn: () => getAgentConfigSurface(pubkey ?? ""),
    enabled: !!pubkey,
    refetchInterval,
    ...agentsFocusRefetchPolicy,
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
