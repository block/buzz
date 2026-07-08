import * as React from "react";

import {
  type AttachManagedAgentToChannelResult,
  useAvailableAcpRuntimes,
  useCreateManagedAgentMutation,
  useManagedAgentLogQuery,
  useManagedAgentsQuery,
  useRelayAgentsQuery,
  useSetManagedAgentStartOnAppLaunchMutation,
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
  useDeleteManagedAgentMutation,
} from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import type {
  AcpRuntime,
  AcpRuntimeCatalogEntry,
  AgentPersona,
  Channel,
  CreateManagedAgentInput,
  CreateManagedAgentResponse,
  ManagedAgent,
} from "@/shared/api/types";
import { removeChannelMember } from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  deleteManagedAgentWithRules,
  isManagedAgentActive,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "../lib/managedAgentControlActions";
import { resolvePersonaRuntime } from "../lib/resolvePersonaRuntime";

export function useManagedAgentActions() {
  const relayAgentsQuery = useRelayAgentsQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const [shouldLoadChannels, setShouldLoadChannels] = React.useState(false);
  const channelsQuery = useChannelsQuery({ enabled: shouldLoadChannels });
  const startMutation = useStartManagedAgentMutation();
  const stopMutation = useStopManagedAgentMutation();
  const deleteMutation = useDeleteManagedAgentMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const startOnLaunchMutation = useSetManagedAgentStartOnAppLaunchMutation();
  const [isCreateOpen, setIsCreateOpen] = React.useState(false);
  const [agentToAddToChannel, setAgentToAddToChannel] =
    React.useState<ManagedAgent | null>(null);
  const [createdAgent, setCreatedAgent] =
    React.useState<CreateManagedAgentResponse | null>(null);
  const [startingPersonaIds, setStartingPersonaIds] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const startingPersonaIdsRef = React.useRef(new Set<string>());
  const [logAgentPubkey, setLogAgentPubkey] = React.useState<string | null>(
    null,
  );
  const [actionNoticeMessage, setActionNoticeMessage] = React.useState<
    string | null
  >(null);
  const [actionErrorMessage, setActionErrorMessage] = React.useState<
    string | null
  >(null);

  const managedAgentLogQuery = useManagedAgentLogQuery(logAgentPubkey);

  React.useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setShouldLoadChannels(true);
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, []);

  const managedAgents = React.useMemo(
    () =>
      [...(managedAgentsQuery.data ?? [])].sort((left, right) => {
        const activeScore = (s: string) =>
          s === "running" || s === "deployed" ? 1 : 0;
        const diff = activeScore(right.status) - activeScore(left.status);
        if (diff !== 0) return diff;
        return left.name.localeCompare(right.name);
      }),
    [managedAgentsQuery.data],
  );
  // Observer ingestion is owner-global (useAgentObserverIngestion in
  // AppShell); this hook only reads derived state.

  const managedPubkeys = React.useMemo(
    () => new Set(managedAgents.map((agent) => agent.pubkey)),
    [managedAgents],
  );

  const managedPubkeyList = React.useMemo(
    () => managedAgents.map((agent) => agent.pubkey),
    [managedAgents],
  );

  const managedPresenceQuery = usePresenceQuery(managedPubkeyList);

  const channelsByPubkey = React.useMemo(() => {
    const map: Record<string, { id: string; name: string }[]> = {};
    // Seed from relay agent profiles (kind:10100 events).
    for (const ra of relayAgentsQuery.data ?? []) {
      if (ra.channels.length > 0) {
        // Skip entries missing a channel id rather than falling back to the
        // name as id — a misaligned channels/channelIds pairing would otherwise
        // produce a pill that silently navigates to a channel name as if it
        // were an id.
        map[normalizePubkey(ra.pubkey)] = ra.channels.flatMap((name, i) => {
          const id = ra.channelIds[i];
          return id ? [{ id, name }] : [];
        });
      }
    }
    // Fill in from channel member lists (kind:39002) for any managed agents
    // not already covered by relay agent data.
    const normalizedManaged = new Set(
      managedAgents.map((a) => normalizePubkey(a.pubkey)),
    );
    for (const ch of channelsQuery.data ?? []) {
      for (const pk of ch.memberPubkeys) {
        const key = normalizePubkey(pk);
        if (!normalizedManaged.has(key)) continue;
        if (!map[key]) map[key] = [];
        if (!map[key].some((entry) => entry.id === ch.id)) {
          map[key].push({ id: ch.id, name: ch.name });
        }
      }
    }
    return map;
  }, [relayAgentsQuery.data, channelsQuery.data, managedAgents]);

  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const ch of channelsQuery.data ?? []) {
      map[ch.id] = ch.name;
    }
    return map;
  }, [channelsQuery.data]);

  // Clear log selection if the agent was removed
  React.useEffect(() => {
    if (
      logAgentPubkey &&
      !managedAgents.some((agent) => agent.pubkey === logAgentPubkey)
    ) {
      setLogAgentPubkey(null);
    }
  }, [managedAgents, logAgentPubkey]);

  function clearFeedback() {
    setActionNoticeMessage(null);
    setActionErrorMessage(null);
  }

  async function handleStart(pubkey: string) {
    clearFeedback();
    try {
      const agent = managedAgents.find((c) => c.pubkey === pubkey);
      if (!agent) return;
      await startManagedAgentWithRules({
        agent,
        startManagedAgent: startMutation.mutateAsync,
      });
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to start agent.",
      );
    }
  }

  async function getAvailableRuntimesForStart() {
    if (availableRuntimesQuery.isFetched) {
      return availableRuntimesQuery.data ?? [];
    }

    const result = await availableRuntimesQuery.refetch();
    return filterAvailableRuntimes(result.data);
  }

  function setPersonaStartPending(personaId: string, pending: boolean) {
    const next = new Set(startingPersonaIdsRef.current);
    if (pending) {
      next.add(personaId);
    } else {
      next.delete(personaId);
    }
    startingPersonaIdsRef.current = next;
    setStartingPersonaIds(next);
  }

  async function handleStartPersona(persona: AgentPersona) {
    if (startingPersonaIdsRef.current.has(persona.id)) {
      return;
    }
    setPersonaStartPending(persona.id, true);
    clearFeedback();
    try {
      const runtimes = await getAvailableRuntimesForStart();
      const defaultRuntime = runtimes[0] ?? null;
      const { runtime, warnings, isOverridden } = resolvePersonaRuntime(
        persona.runtime,
        runtimes,
        defaultRuntime,
      );

      if (!runtime) {
        throw new Error("No available runtime found for this agent.");
      }
      if (isOverridden) {
        throw new Error(
          warnings[0] ??
            "This agent's configured runtime is not available. Install the runtime or edit the agent before starting it.",
        );
      }

      const input: CreateManagedAgentInput = {
        name: persona.displayName,
        acpCommand: "buzz-acp",
        agentCommand: runtime.command,
        agentArgs: runtime.defaultArgs,
        mcpCommand: runtime.mcpCommand ?? "",
        personaId: persona.id,
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        model: persona.model ?? undefined,
        envVars: persona.envVars,
        spawnAfterCreate: true,
        startOnAppLaunch: true,
        backend: { type: "local" },
        harnessOverride: !persona.runtime || persona.runtime === runtime.id,
      };

      const created = await createAgentMutation.mutateAsync(input);
      setCreatedAgent(created);
      const notices = [...warnings];

      if (created.spawnError) {
        setActionErrorMessage(created.spawnError);
      } else {
        notices.push(`Started ${created.agent.name}.`);
      }

      if (created.profileSyncError) {
        notices.push(created.profileSyncError);
      }
      if (notices.length > 0) {
        setActionNoticeMessage(notices.join(" "));
      }

      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to start agent.",
      );
    } finally {
      setPersonaStartPending(persona.id, false);
    }
  }

  async function getChannelsForAction() {
    if (channelsQuery.data) {
      return channelsQuery.data;
    }

    const result = await channelsQuery.refetch();
    return result.data ?? [];
  }

  async function handleStop(pubkey: string) {
    clearFeedback();
    try {
      const agent = managedAgents.find((a) => a.pubkey === pubkey);
      if (!agent) return;
      const channels = await getChannelsForAction();
      const result = await stopManagedAgentWithRules({
        agent,
        channels,
        relayAgents: relayAgentsQuery.data ?? [],
        stopManagedAgent: stopMutation.mutateAsync,
      });
      if (result.noticeMessage) {
        setActionNoticeMessage(result.noticeMessage);
      }
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to stop agent.",
      );
    }
  }

  function getAgentChannelIds(pubkey: string): string[] {
    const normalized = normalizePubkey(pubkey);
    const relayAgent = (relayAgentsQuery.data ?? []).find(
      (ra) => normalizePubkey(ra.pubkey) === normalized,
    );
    return relayAgent?.channelIds ?? [];
  }

  async function removeAgentFromAllChannels(pubkey: string) {
    const channelIds = getAgentChannelIds(pubkey);
    if (channelIds.length === 0) return;
    await Promise.allSettled(
      channelIds.map((channelId) => removeChannelMember(channelId, pubkey)),
    );
  }

  async function handleDelete(pubkey: string) {
    clearFeedback();
    try {
      const agent = managedAgents.find((a) => a.pubkey === pubkey);
      if (!agent) return;
      const channels = await getChannelsForAction();
      const result = await deleteManagedAgentWithRules({
        agent,
        channels,
        deleteManagedAgent: deleteMutation.mutateAsync,
        presenceLookup: managedPresenceQuery.data,
        relayAgents: relayAgentsQuery.data ?? [],
      });
      if (result.cancelled) return;
      await removeAgentFromAllChannels(pubkey);
      if (logAgentPubkey === pubkey) {
        setLogAgentPubkey(null);
      }
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error ? error.message : "Failed to delete agent.",
      );
    }
  }

  async function handleToggleStartOnAppLaunch(
    pubkey: string,
    startOnAppLaunch: boolean,
  ) {
    clearFeedback();
    try {
      const updated = await startOnLaunchMutation.mutateAsync({
        pubkey,
        startOnAppLaunch,
      });
      setActionNoticeMessage(
        updated.startOnAppLaunch
          ? `Will start ${updated.name} automatically when the desktop app opens.`
          : `${updated.name} will stay manual-start only.`,
      );
    } catch (error) {
      setActionErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to update startup preference.",
      );
    }
  }

  function handleAddedToChannel(
    channel: Channel,
    result: AttachManagedAgentToChannelResult,
  ) {
    setActionErrorMessage(null);
    setActionNoticeMessage(() => {
      if (result.started) {
        return `Added ${result.agent.name} to ${channel.name} and spawned it.`;
      }
      if (result.membershipAdded) {
        return `Added ${result.agent.name} to ${channel.name}.`;
      }
      return `${result.agent.name} is already in ${channel.name}.`;
    });
    void managedAgentsQuery.refetch();
    void relayAgentsQuery.refetch();
  }

  async function runBulkAction(
    targets: ManagedAgent[],
    confirmLabel: string,
    failureNoun: string,
    action: (agent: ManagedAgent) => Promise<unknown>,
  ): Promise<boolean> {
    if (targets.length === 0) return false;
    const confirmed = window.confirm(
      `${confirmLabel} ${targets.length} agent${targets.length === 1 ? "" : "s"}?`,
    );
    if (!confirmed) return false;
    clearFeedback();
    const results = await Promise.allSettled(targets.map(action));
    const failures = results.filter((r) => r.status === "rejected");
    if (failures.length > 0) {
      setActionErrorMessage(
        `${failures.length} of ${targets.length} ${failureNoun}${failures.length === 1 ? "" : "s"} failed.`,
      );
    }
    return true;
  }

  async function handleBulkStopRunning() {
    await runBulkAction(
      managedAgents.filter((a) => isManagedAgentActive(a)),
      "Stop",
      "stop",
      (a) =>
        stopManagedAgentWithRules({
          agent: a,
          channels: channelsQuery.data ?? [],
          relayAgents: relayAgentsQuery.data ?? [],
          stopManagedAgent: stopMutation.mutateAsync,
        }),
    );
  }

  async function handleBulkRemoveStopped() {
    const targets = managedAgents.filter(
      (a) => a.status === "stopped" || a.status === "not_deployed",
    );
    const executed = await runBulkAction(
      targets,
      "Remove",
      "removal",
      async (a) => {
        await deleteManagedAgent(a);
        await removeAgentFromAllChannels(a.pubkey);
      },
    );
    if (
      executed &&
      logAgentPubkey &&
      targets.some((a) => a.pubkey === logAgentPubkey)
    ) {
      setLogAgentPubkey(null);
    }
  }

  async function deleteManagedAgent(agent: ManagedAgent) {
    const isDeployedRemote =
      agent.backend.type === "provider" && agent.backendAgentId;
    await deleteMutation.mutateAsync({
      pubkey: agent.pubkey,
      forceRemoteDelete: isDeployedRemote ? true : undefined,
    });
  }

  const isPending =
    createAgentMutation.isPending ||
    startMutation.isPending ||
    stopMutation.isPending ||
    startOnLaunchMutation.isPending ||
    deleteMutation.isPending;
  const startingAgentPubkey =
    startMutation.isPending && typeof startMutation.variables === "string"
      ? startMutation.variables
      : null;

  return {
    relayAgentsQuery,
    managedAgentsQuery,
    managedAgentLogQuery,
    managedPresenceQuery,
    managedAgents,
    managedPubkeys,
    channelIdToName,
    channelsByPubkey,
    isPending,
    isCreateOpen,
    setIsCreateOpen,
    agentToAddToChannel,
    setAgentToAddToChannel,
    createdAgent,
    setCreatedAgent,
    logAgentPubkey,
    setLogAgentPubkey,
    actionNoticeMessage,
    setActionNoticeMessage,
    actionErrorMessage,
    setActionErrorMessage,
    startingAgentPubkey,
    startingPersonaIds,
    handleStart,
    handleStartPersona,
    handleStop,
    handleDelete,
    handleToggleStartOnAppLaunch,
    handleAddedToChannel,
    handleBulkStopRunning,
    handleBulkRemoveStopped,
    refetchManagedAgents: () => void managedAgentsQuery.refetch(),
    refetchRelayAgents: () => void relayAgentsQuery.refetch(),
  };
}

function filterAvailableRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[] | undefined,
): AcpRuntime[] {
  return (runtimes ?? []).filter(
    (runtime): runtime is AcpRuntime => runtime.availability === "available",
  );
}
