import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useAgentMemoryQuery,
  useIsManagedAgent,
} from "@/features/agent-memory/hooks";
import {
  type AttachManagedAgentToChannelResult,
  useAcpRuntimesQuery,
  useAvailableAcpRuntimes,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeleteManagedAgentMutation,
  useDeletePersonaMutation,
  useExportPersonaJsonMutation,
  useManagedAgentLogQuery,
  useRelayAgentsQuery,
  useManagedAgentsQuery,
  usePersonasQuery,
  useSetManagedAgentStartOnAppLaunchMutation,
  useSetPersonaActiveMutation,
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
  useUpdateManagedAgentMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { AddAgentToChannelDialog } from "@/features/agents/ui/AddAgentToChannelDialog";
import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import {
  isManagedAgentActive,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { describeLogFile } from "@/features/agents/ui/agentUi";
import { EditAgentDialog } from "@/features/agents/ui/EditAgentDialog";
import {
  duplicatePersonaDialogState,
  editPersonaDialogState,
  type PersonaDialogState,
} from "@/features/agents/ui/personaDialogState";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityArchive } from "@/features/identity-archive/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  useContactListQuery,
  useFollowMutation,
  useProfileQuery,
  useUnfollowMutation,
  useUserProfileQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import {
  AgentInfoFocusedView,
  AgentInstructionsFocusedView,
  ChannelsFocusedView,
  DiagnosticsFocusedView,
  MemoryFocusedView,
  ProfileSummaryView,
} from "@/features/profile/ui/UserProfilePanelSections";
import { AgentConfigurationFocusedView } from "@/features/profile/ui/UserProfilePanelAgentDetails";
import { UserProfileAgentSettingsMenuSlot } from "@/features/profile/ui/UserProfileAgentActions";
import { useProfileAgentDeletion } from "@/features/profile/ui/UserProfilePanelDeletion";
import { useProfileFieldBuckets } from "@/features/profile/ui/UserProfilePanelFields";
import { submitProfilePersonaDialog } from "@/features/profile/ui/UserProfilePanelPersonaSubmit";
import { UserProfilePersonaDialogs } from "@/features/profile/ui/UserProfilePersonaDialogs";
import {
  deriveProfileChannels,
  type ProfilePanelTab,
  type ProfilePanelView,
  resolveAgentInstruction,
  resolvePanelProfile,
  resolveProfileDisplayName,
  truncatePubkey,
  type UserProfilePanelProps,
  useRetainedPersona,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useProfileDmAction } from "@/features/profile/ui/useProfileDmAction";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { auxiliaryPanelContentPaddingClass } from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import type {
  AgentPersona,
  Channel,
  CreateManagedAgentInput,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { UserProfilePanelFrame } from "@/features/profile/ui/UserProfilePanelFrame";
import { getUserProfilePanelHeaderContent } from "@/features/profile/ui/UserProfilePanelHeaderContent";
export type { ProfilePanelTab, ProfilePanelView };

export function UserProfilePanel({
  canResetWidth,
  currentPubkey,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onOpenDm,
  onOpenProfile,
  onResetWidth,
  onResizeStart,
  onTabChange,
  onViewChange,
  persona,
  pubkey,
  splitPaneClamp = false,
  tab: controlledTab,
  view: controlledView,
  widthPx,
  transparentChrome = false,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const [internalView, setInternalView] =
    React.useState<ProfilePanelView>("summary");
  const view = controlledView ?? internalView;
  const setView = React.useCallback(
    (nextView: ProfilePanelView, options?: { replace?: boolean }) => {
      if (onViewChange) {
        onViewChange(nextView, options);
        return;
      }
      setInternalView(nextView);
    },
    [onViewChange],
  );
  const [internalTab, setInternalTab] = React.useState<ProfilePanelTab>("info");
  const tab = controlledTab ?? internalTab;
  const setTab = React.useCallback(
    (nextTab: ProfilePanelTab, options?: { replace?: boolean }) => {
      if (onTabChange) {
        onTabChange(nextTab, options);
        return;
      }
      setInternalTab(nextTab);
    },
    [onTabChange],
  );
  const [editAgentOpen, setEditAgentOpen] = React.useState(false);
  const [addToChannelOpen, setAddToChannelOpen] = React.useState(false);
  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);

  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const managedAgent = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    if (pubkey) {
      const pubkeyLower = pubkey.toLowerCase();
      return agents.find((agent) => agent.pubkey.toLowerCase() === pubkeyLower);
    }
    if (persona) {
      return agents.find((agent) => agent.personaId === persona.id);
    }
    return undefined;
  }, [managedAgentsQuery.data, persona, pubkey]);
  const resolvedPersonaFromSource = React.useMemo(() => {
    const personaId = persona?.id ?? managedAgent?.personaId;
    if (personaId) {
      const refreshedPersona = personasQuery.data?.find(
        (candidate) => candidate.id === personaId,
      );
      if (refreshedPersona) {
        return refreshedPersona;
      }
    }
    if (persona) {
      return persona;
    }
    if (!managedAgent?.personaId) {
      return undefined;
    }
    return personasQuery.data?.find(
      (candidate) => candidate.id === managedAgent.personaId,
    );
  }, [managedAgent?.personaId, persona, personasQuery.data]);
  const profileIdentityKey =
    pubkey ?? managedAgent?.pubkey ?? `persona:${persona?.id ?? "unknown"}`;
  const resolvedPersona = useRetainedPersona(
    resolvedPersonaFromSource,
    profileIdentityKey,
  );
  const effectivePubkey = pubkey ?? managedAgent?.pubkey ?? null;
  const pubkeyLower = effectivePubkey?.toLowerCase() ?? "";

  const profileQuery = useUserProfileQuery(effectivePubkey ?? undefined);
  const currentProfileQuery = useProfileQuery(currentPubkey !== undefined);

  React.useEffect(() => {
    if (!effectivePubkey) return;
    void profileQuery.refetch();
  }, [effectivePubkey, profileQuery.refetch]);

  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const acpRuntimesQuery = useAcpRuntimesQuery();
  const createAgentMutation = useCreateManagedAgentMutation();
  const updateManagedAgentMutation = useUpdateManagedAgentMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const stopAgentMutation = useStopManagedAgentMutation();
  const deleteAgentMutation = useDeleteManagedAgentMutation();
  const startOnLaunchMutation = useSetManagedAgentStartOnAppLaunchMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();
  const exportPersonaJsonMutation = useExportPersonaJsonMutation();
  const usersBatchQuery = useUsersBatchQuery(
    effectivePubkey ? [effectivePubkey] : [],
  );
  const channelsQuery = useChannelsQuery();
  const presenceQuery = usePresenceQuery(
    effectivePubkey ? [effectivePubkey] : [],
  );
  const userStatusQuery = useUserStatusQuery(
    effectivePubkey ? [effectivePubkey] : [],
  );
  const contactListQuery = useContactListQuery(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);
  const { onOpenAgentSession } = useAgentSession();
  const { goChannel } = useAppNavigation();
  const profile = resolvePanelProfile({
    managedAgent,
    persona: resolvedPersona,
    profile: profileQuery.data,
  });
  const ownerPubkey = profile?.ownerPubkey ?? null;
  const ownerProfileQuery = useUserProfileQuery(ownerPubkey ?? undefined);
  const presenceStatus = pubkeyLower
    ? presenceQuery.data?.[pubkeyLower]
    : undefined;
  const userStatus = pubkeyLower
    ? userStatusQuery.data?.[pubkeyLower]
    : undefined;

  const relayAgent = relayAgentsQuery.data?.find(
    (agent) => agent.pubkey.toLowerCase() === pubkeyLower,
  );
  const managedAgentLogQuery = useManagedAgentLogQuery(
    (view === "diagnostics" || view === "logs") &&
      managedAgent?.backend.type === "local"
      ? managedAgent.pubkey
      : null,
  );
  const isAgentByOaOwner = Boolean(
    usersBatchQuery.data?.profiles[pubkeyLower]?.isAgent,
  );
  const isBot =
    Boolean(relayAgent || managedAgent || resolvedPersona) || isAgentByOaOwner;
  const managedAgentOwner = useIsManagedAgent(isBot ? effectivePubkey : null);
  // Does THIS desktop hold the agent's seckey (or is this an editable persona)?
  // Gates edit (which needs the key) and grants owner access when managed locally.
  const isOwner = resolvedPersona ? true : managedAgentOwner;
  // Is the viewer the agent's declared owner (NIP-OA `ownerPubkey == me`)? This
  // is the right signal for viewing owner-scoped data (activity feed, memory):
  // the relay routes and the client decrypts those frames with the owner's OWN
  // key, so the agent's seckey is never needed. Computed here (before the gates
  // that consume it) so visibility keys off declared ownership, not key custody.
  const isCurrentUserOwner =
    currentPubkey !== undefined &&
    ownerPubkey !== null &&
    ownerPubkey.toLowerCase() === currentPubkey.toLowerCase();
  // The viewer may see owner-scoped data if they declared-own the agent OR they
  // manage it locally (older agents may not advertise an owner pubkey). Every
  // real boundary is server-side, so this only controls what UI we paint.
  const viewerIsOwner = isCurrentUserOwner || isOwner === true;

  // Populate the active-turns store for this agent so useActiveAgentTurns works
  // even if the Agents page hasn't been visited yet.
  const bridgeAgents = React.useMemo(
    () =>
      managedAgent
        ? [{ pubkey: managedAgent.pubkey, status: managedAgent.status }]
        : [],
    [managedAgent],
  );
  // The observer bridge subscribes on the OWNER's own pubkey and decrypts the
  // agent's telemetry with the owner's key — no agent seckey needed. It only
  // decrypts frames whose agent pubkey is "known", and only subscribes when an
  // agent is running/deployed. For a remote agent we own but don't manage
  // locally, `managedAgent` is undefined, so we seed the bridge from the relay
  // agent (treated as "deployed") when the viewer is the declared owner. This
  // mirrors what the composer-area ingress already does in ChannelScreen.
  const observerBridgeAgents = React.useMemo(() => {
    if (managedAgent) {
      return [{ pubkey: managedAgent.pubkey, status: managedAgent.status }];
    }
    if (viewerIsOwner && relayAgent) {
      return [
        {
          pubkey: relayAgent.pubkey,
          status: "deployed" as ManagedAgent["status"],
        },
      ];
    }
    return [];
  }, [managedAgent, relayAgent, viewerIsOwner]);
  useActiveAgentTurnsBridge(bridgeAgents);
  useManagedAgentObserverBridge(observerBridgeAgents);
  const canEditAgent =
    isOwner === true &&
    (managedAgent !== undefined ||
      (resolvedPersona !== undefined && !resolvedPersona.isBuiltIn));
  const memoryQuery = useAgentMemoryQuery(effectivePubkey, {
    enabled: viewerIsOwner && Boolean(effectivePubkey),
  });
  const isSelf =
    currentPubkey !== undefined &&
    pubkeyLower.length > 0 &&
    pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity =
    viewerIsOwner && Boolean(onOpenAgentSession) && Boolean(effectivePubkey);
  const canOpenAgentLogs =
    isOwner === true && managedAgent?.backend.type === "local";
  const canInstantiateAgent =
    isOwner === true &&
    resolvedPersona !== undefined &&
    managedAgent === undefined;
  const isAgentActionPending =
    createAgentMutation.isPending ||
    updateManagedAgentMutation.isPending ||
    startAgentMutation.isPending ||
    stopAgentMutation.isPending ||
    deleteAgentMutation.isPending ||
    startOnLaunchMutation.isPending ||
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending ||
    exportPersonaJsonMutation.isPending;
  const isFollowing =
    !isSelf &&
    pubkeyLower.length > 0 &&
    (contactListQuery.data?.contacts.some(
      (contact) => contact.pubkey.toLowerCase() === pubkeyLower,
    ) ??
      false);

  const profileChannels = React.useMemo(
    () =>
      deriveProfileChannels(
        pubkeyLower,
        relayAgent,
        managedAgent,
        channelsQuery.data,
      ),
    [pubkeyLower, relayAgent, managedAgent, channelsQuery.data],
  );

  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const targetKey =
    effectivePubkey ?? `persona:${resolvedPersona?.id ?? "unknown"}`;
  const prevTargetKeyRef = React.useRef(targetKey);
  React.useEffect(() => {
    if (prevTargetKeyRef.current === targetKey) return;
    prevTargetKeyRef.current = targetKey;
    setView("summary", { replace: true });
    setTab("info", { replace: true });
  }, [setTab, setView, targetKey]);
  const { handleMessage, isOpeningDm } = useProfileDmAction({
    effectivePubkey,
    onClose,
    onOpenDm,
  });

  const handleEditAgent = React.useCallback(() => {
    if (resolvedPersona && !resolvedPersona.isBuiltIn) {
      setPersonaDialogState(editPersonaDialogState(resolvedPersona));
      return;
    }
    setEditAgentOpen(true);
  }, [resolvedPersona]);

  const { deleteManagedAgentRecord, deleteManagedAgentsForPersona } =
    useProfileAgentDeletion({
      channels: channelsQuery.data,
      deleteManagedAgent: deleteAgentMutation.mutateAsync,
      managedAgent,
      managedAgents: managedAgentsQuery.data,
      presenceLookup: presenceQuery.data,
      relayAgents: relayAgentsQuery.data,
    });

  const createManagedAgentForPersona = React.useCallback(
    async (personaToStart: AgentPersona) => {
      const runtimes = availableRuntimesQuery.data ?? [];
      const defaultRuntime = runtimes[0] ?? null;
      const { runtime, warnings } = resolvePersonaRuntime(
        personaToStart.runtime,
        runtimes,
        defaultRuntime,
      );

      for (const warning of warnings) {
        toast.warning(warning);
      }

      if (!runtime) {
        throw new Error("No available runtime found for this agent.");
      }

      const input: CreateManagedAgentInput = {
        name: personaToStart.displayName,
        acpCommand: "buzz-acp",
        agentCommand: runtime.command,
        agentArgs: runtime.defaultArgs,
        mcpCommand: runtime.mcpCommand ?? "",
        personaId: personaToStart.id,
        systemPrompt: personaToStart.systemPrompt,
        avatarUrl: personaToStart.avatarUrl ?? undefined,
        model: personaToStart.model ?? undefined,
        envVars: personaToStart.envVars,
        spawnAfterCreate: true,
        startOnAppLaunch: true,
        backend: { type: "local" },
      };

      const created = await createAgentMutation.mutateAsync(input);
      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
      return created;
    },
    [
      availableRuntimesQuery.data,
      createAgentMutation.mutateAsync,
      managedAgentsQuery.refetch,
      relayAgentsQuery.refetch,
    ],
  );

  const handleAgentPrimaryAction = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      if (isManagedAgentActive(managedAgent)) {
        const result = await stopManagedAgentWithRules({
          agent: managedAgent,
          channels: channelsQuery.data ?? [],
          relayAgents: relayAgentsQuery.data ?? [],
          stopManagedAgent: stopAgentMutation.mutateAsync,
        });
        toast.success(result.noticeMessage ?? `Stopped ${managedAgent.name}.`);
        return;
      }

      await startManagedAgentWithRules({
        agent: managedAgent,
        startManagedAgent: startAgentMutation.mutateAsync,
      });
      toast.success(
        managedAgent.backend.type === "provider"
          ? `Deploying ${managedAgent.name}.`
          : `Started ${managedAgent.name}.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Agent action failed.",
      );
    }
  }, [
    channelsQuery.data,
    managedAgent,
    relayAgentsQuery.data,
    startAgentMutation.mutateAsync,
    stopAgentMutation.mutateAsync,
  ]);

  const handleInstantiateAgent = React.useCallback(async () => {
    if (!resolvedPersona) return;

    try {
      const created = await createManagedAgentForPersona(resolvedPersona);
      if (created.spawnError) {
        toast.error(created.spawnError);
      } else {
        toast.success(`Started ${created.agent.name}.`);
      }
      if (created.profileSyncError) {
        toast.warning(created.profileSyncError);
      }
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to start agent.",
      );
    }
  }, [createManagedAgentForPersona, resolvedPersona]);

  const handleToggleAgentAutoStart = React.useCallback(async () => {
    if (managedAgent?.backend.type !== "local") return;

    try {
      const updated = await startOnLaunchMutation.mutateAsync({
        pubkey: managedAgent.pubkey,
        startOnAppLaunch: !managedAgent.startOnAppLaunch,
      });
      toast.success(
        updated.startOnAppLaunch
          ? `Will start ${updated.name} automatically.`
          : `${updated.name} will stay manual-start only.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update startup preference.",
      );
    }
  }, [managedAgent, startOnLaunchMutation.mutateAsync]);

  const handleDeleteAgent = React.useCallback(async () => {
    if (!managedAgent) return;

    try {
      const result = await deleteManagedAgentRecord(managedAgent);
      if (result.cancelled) return;

      toast.success(`Deleted ${managedAgent.name}.`);
      onClose();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to delete agent.",
      );
    }
  }, [deleteManagedAgentRecord, managedAgent, onClose]);

  const handleSubmitPersona = React.useCallback(
    async (input: CreatePersonaInput | UpdatePersonaInput) => {
      await submitProfilePersonaDialog({
        createManagedAgentForPersona,
        createPersona: createPersonaMutation.mutateAsync,
        input,
        managedAgent,
        onDone: () => {
          setPersonaDialogState(null);
          void personasQuery.refetch();
        },
        previousPersona: resolvedPersona,
        runtimes: acpRuntimesQuery.data ?? [],
        updateManagedAgent: updateManagedAgentMutation.mutateAsync,
        updatePersona: updatePersonaMutation.mutateAsync,
      });
    },
    [
      createPersonaMutation.mutateAsync,
      createManagedAgentForPersona,
      managedAgent,
      personasQuery.refetch,
      resolvedPersona,
      acpRuntimesQuery.data,
      updateManagedAgentMutation.mutateAsync,
      updatePersonaMutation.mutateAsync,
    ],
  );

  const handleEditPersona = React.useCallback(() => {
    if (!resolvedPersona || resolvedPersona.isBuiltIn) return;
    setPersonaDialogState(editPersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleDuplicatePersona = React.useCallback(() => {
    if (!resolvedPersona) return;
    setPersonaDialogState(duplicatePersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleExportPersona = React.useCallback(() => {
    if (!resolvedPersona) return;
    exportPersonaJsonMutation.mutate(resolvedPersona.id, {
      onSuccess: (saved) => {
        if (saved) {
          toast.success(`Exported ${resolvedPersona.displayName}.`);
        }
      },
      onError: (error) => {
        toast.error(
          error instanceof Error ? error.message : "Failed to export agent.",
        );
      },
    });
  }, [exportPersonaJsonMutation, resolvedPersona]);

  const handleDeletePersona = React.useCallback(async () => {
    if (!resolvedPersona) return;

    if (resolvedPersona.isBuiltIn) {
      try {
        const deletedInstances =
          await deleteManagedAgentsForPersona(resolvedPersona);
        if (deletedInstances.cancelled) return;

        await setPersonaActiveMutation.mutateAsync({
          id: resolvedPersona.id,
          active: false,
        });
        toast.success(`Removed ${resolvedPersona.displayName} from My Agents.`);
        onClose();
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete agent.",
        );
      }
      return;
    }

    if (resolvedPersona.sourceTeam) {
      toast.error("This agent is managed by a team.");
      return;
    }

    setPersonaToDelete(resolvedPersona);
  }, [
    deleteManagedAgentsForPersona,
    onClose,
    resolvedPersona,
    setPersonaActiveMutation.mutateAsync,
  ]);

  const handleConfirmDeletePersona = React.useCallback(
    async (personaToConfirm: AgentPersona) => {
      if (personaToConfirm.sourceTeam) {
        toast.error("This agent is managed by a team.");
        setPersonaToDelete(null);
        return;
      }

      try {
        const deletedInstances =
          await deleteManagedAgentsForPersona(personaToConfirm);
        if (deletedInstances.cancelled) return;

        await deletePersonaMutation.mutateAsync(personaToConfirm.id);
        toast.success(`Deleted ${personaToConfirm.displayName}.`);
        setPersonaToDelete(null);
        onClose();
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete agent.",
        );
      }
    },
    [deleteManagedAgentsForPersona, deletePersonaMutation.mutateAsync, onClose],
  );

  const handleAddedToChannel = React.useCallback(
    (channel: Channel, result: AttachManagedAgentToChannelResult) => {
      if (result.restarted) {
        toast.success(
          `Added ${result.agent.name} to ${channel.name} and restarted it.`,
        );
      } else if (result.started) {
        toast.success(`Added ${result.agent.name} to ${channel.name}.`);
      } else if (result.membershipAdded) {
        toast.success(`Added ${result.agent.name} to ${channel.name}.`);
      } else {
        toast.success(`${result.agent.name} is already in ${channel.name}.`);
      }
      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
      void channelsQuery.refetch();
    },
    [
      channelsQuery.refetch,
      managedAgentsQuery.refetch,
      relayAgentsQuery.refetch,
    ],
  );

  const handleOpenActivity = React.useCallback(() => {
    if (!effectivePubkey) return;
    onOpenAgentSession?.(effectivePubkey);
  }, [effectivePubkey, onOpenAgentSession]);

  const handleOpenChannel = React.useCallback(
    (channelId: string) => {
      void goChannel(channelId);
    },
    [goChannel],
  );

  const displayName = resolveProfileDisplayName({
    persona: resolvedPersona,
    profile,
    pubkey: effectivePubkey,
  });
  const ownerHandle = React.useMemo(() => {
    if (ownerPubkey) {
      const ownerProfile = ownerProfileQuery.data;
      return (
        ownerProfile?.nip05Handle?.trim() ||
        ownerProfile?.displayName?.trim() ||
        truncatePubkey(ownerPubkey)
      );
    }

    if (currentPubkey === undefined || isOwner !== true) {
      return null;
    }

    const currentProfile = currentProfileQuery.data;
    return (
      currentProfile?.nip05Handle?.trim() ||
      currentProfile?.displayName?.trim() ||
      truncatePubkey(currentPubkey)
    );
  }, [
    currentProfileQuery.data,
    currentPubkey,
    isOwner,
    ownerProfileQuery.data,
    ownerPubkey,
  ]);
  const ownerDisplayName = ownerHandle
    ? isCurrentUserOwner || (!ownerPubkey && isOwner === true)
      ? `${ownerHandle} (you)`
      : ownerHandle
    : null;
  const ownerProfilePubkey =
    ownerPubkey ?? (isOwner === true ? (currentPubkey ?? null) : null);
  const ownerAvatarProfile = ownerPubkey
    ? ownerProfileQuery.data
    : currentProfileQuery.data;
  const memoryCount =
    memoryQuery.data &&
    (memoryQuery.data.core ? 1 : 0) + memoryQuery.data.memories.length;
  const agentInstruction = resolveAgentInstruction(
    managedAgent,
    resolvedPersona,
  );
  const canManagePersona = isOwner === true && resolvedPersona !== undefined;
  const canEditPersona =
    canManagePersona && resolvedPersona?.isBuiltIn !== true;
  const canDeletePersona = canManagePersona && !resolvedPersona?.sourceTeam;
  const archiveActions = useIdentityArchive(effectivePubkey);
  const agentSettingsMenu = (
    <UserProfileAgentSettingsMenuSlot
      archiveActions={archiveActions}
      canDeletePersona={canDeletePersona}
      canInstantiateAgent={canInstantiateAgent}
      canManagePersona={canManagePersona}
      isAgentActionPending={isAgentActionPending}
      isBot={isBot}
      managedAgent={managedAgent}
      onDeleteAgent={handleDeleteAgent}
      onDeletePersona={handleDeletePersona}
      onDuplicatePersona={handleDuplicatePersona}
      onExportPersona={handleExportPersona}
      onToggleAutoStart={handleToggleAgentAutoStart}
      personaActionKey={resolvedPersona?.id}
      viewerIsOwner={viewerIsOwner}
    />
  );
  const {
    agentInfoFields,
    agentSettingsFields,
    diagnosticsFields,
    modelLabel,
  } = useProfileFieldBuckets({
    isBot,
    isOwner,
    managedAgent,
    onOpenProfile,
    ownerAvatarUrl: ownerAvatarProfile?.avatarUrl ?? null,
    ownerDisplayName,
    ownerHandle,
    ownerProfilePubkey,
    ownerPubkey,
    persona: resolvedPersona,
    presenceLoaded: presenceQuery.isSuccess,
    presenceStatus,
    profile,
    pubkey: effectivePubkey,
    relayAgent,
  });
  const isDiagnosticsLikeView = view === "diagnostics" || view === "logs";
  const managedAgentLogContent = managedAgentLogQuery.data?.content ?? null;
  const logHeaderSubtitle =
    isDiagnosticsLikeView && managedAgent
      ? `${managedAgent.name} · ${describeLogFile(managedAgent.logPath)}`
      : null;
  const { headerActions, headerLeftContent } = getUserProfilePanelHeaderContent(
    {
      agentSettingsMenu,
      effectivePubkey,
      logCopyValue: isDiagnosticsLikeView ? managedAgentLogContent : null,
      logSubtitle: logHeaderSubtitle,
      onBack: () => setView("summary"),
      onClose,
      view,
      viewerIsOwner,
    },
  );

  const profileBody = (
    <div
      className={cn(
        "min-h-0 flex-1 px-4 pb-6",
        isDiagnosticsLikeView
          ? "flex flex-col overflow-hidden"
          : "overflow-y-auto",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && !isFloatingOverlay && "pt-13",
      )}
    >
      {view === "summary" ? (
        <ProfileSummaryView
          canAddToChannel={managedAgent !== undefined && isOwner === true}
          canEditAgent={canEditAgent}
          canInstantiateAgent={canInstantiateAgent}
          canOpenAgentLogs={canOpenAgentLogs}
          canViewActivity={canViewActivity}
          channelCount={profileChannels.length}
          channelIdToName={channelIdToName}
          channels={profileChannels}
          channelsLoading={channelsQuery.isLoading}
          displayName={displayName}
          followMutation={followMutation}
          agentInstruction={agentInstruction}
          handleAgentPrimaryAction={handleAgentPrimaryAction}
          handleEditAgent={handleEditAgent}
          handleEditPersona={canEditPersona ? handleEditPersona : undefined}
          handleInstantiateAgent={handleInstantiateAgent}
          handleMessage={handleMessage}
          isArchived={archiveActions.isArchived === true}
          isMessagePending={isOpeningDm}
          isBot={isBot}
          isAgentActionPending={isAgentActionPending}
          isFollowing={isFollowing}
          isOwner={viewerIsOwner}
          isSelf={isSelf}
          managedAgent={managedAgent}
          memoriesLoading={memoryQuery.isLoading}
          memoryCount={memoryCount}
          modelLabel={modelLabel}
          agentInfoFields={agentInfoFields}
          agentSettingsFields={agentSettingsFields}
          diagnosticsFields={diagnosticsFields}
          onAddToChannel={() => setAddToChannelOpen(true)}
          onOpenActivity={handleOpenActivity}
          onOpenChannel={handleOpenChannel}
          onOpenDiagnostics={() => setView("diagnostics")}
          onOpenInstructions={() => setView("instructions")}
          onTabChange={setTab}
          onOpenDm={onOpenDm}
          presenceStatus={presenceStatus}
          profile={profile}
          pubkey={effectivePubkey}
          relayAgent={relayAgent}
          tab={tab}
          unfollowMutation={unfollowMutation}
          userStatus={userStatus}
        />
      ) : null}
      {view === "memories" && effectivePubkey ? (
        <MemoryFocusedView
          agentPubkey={effectivePubkey}
          viewerIsOwner={viewerIsOwner}
        />
      ) : null}
      {view === "info" ? (
        <AgentInfoFocusedView metadataFields={agentInfoFields} />
      ) : null}
      {view === "configuration" ? (
        <AgentConfigurationFocusedView
          fields={agentSettingsFields}
          managedAgent={managedAgent}
          modelLabel={modelLabel}
        />
      ) : null}
      {view === "instructions" ? (
        <AgentInstructionsFocusedView instruction={agentInstruction} />
      ) : null}
      {view === "diagnostics" ? (
        <DiagnosticsFocusedView
          canOpenAgentLogs={canOpenAgentLogs}
          fields={diagnosticsFields}
          logContent={managedAgentLogContent}
          logError={
            managedAgentLogQuery.error instanceof Error
              ? managedAgentLogQuery.error
              : null
          }
          logLoading={managedAgentLogQuery.isLoading}
          managedAgent={managedAgent}
        />
      ) : null}
      {view === "channels" ? (
        <ChannelsFocusedView
          canAddToChannel={managedAgent !== undefined && isOwner === true}
          channels={profileChannels}
          isActionPending={isAgentActionPending}
          isLoading={channelsQuery.isLoading}
          onAddToChannel={() => setAddToChannelOpen(true)}
          onOpenChannel={handleOpenChannel}
        />
      ) : null}
      {view === "logs" ? (
        <DiagnosticsFocusedView
          canOpenAgentLogs={canOpenAgentLogs}
          fields={[]}
          logContent={managedAgentLogContent}
          logError={
            managedAgentLogQuery.error instanceof Error
              ? managedAgentLogQuery.error
              : null
          }
          logLoading={managedAgentLogQuery.isLoading}
          managedAgent={managedAgent}
        />
      ) : null}
    </div>
  );
  const editAgentDialog =
    canEditAgent && managedAgent ? (
      <EditAgentDialog
        agent={managedAgent}
        onOpenChange={setEditAgentOpen}
        open={editAgentOpen}
      />
    ) : null;
  const addAgentToChannelDialog = managedAgent ? (
    <AddAgentToChannelDialog
      agent={managedAgent ?? null}
      onAdded={handleAddedToChannel}
      onOpenChange={setAddToChannelOpen}
      open={addToChannelOpen}
    />
  ) : null;
  const personaDialogs = (
    <UserProfilePersonaDialogs
      createError={
        createPersonaMutation.error instanceof Error
          ? createPersonaMutation.error
          : null
      }
      isPending={
        createPersonaMutation.isPending ||
        updatePersonaMutation.isPending ||
        updateManagedAgentMutation.isPending ||
        createAgentMutation.isPending
      }
      personaDialogState={personaDialogState}
      personaToDelete={personaToDelete}
      runtimes={acpRuntimesQuery.data ?? []}
      runtimesLoading={acpRuntimesQuery.isLoading}
      updateError={
        updatePersonaMutation.error instanceof Error
          ? updatePersonaMutation.error
          : null
      }
      onCloseDelete={() => setPersonaToDelete(null)}
      onCloseDialog={() => setPersonaDialogState(null)}
      onConfirmDelete={(selectedPersona) => {
        void handleConfirmDeletePersona(selectedPersona);
      }}
      onSubmit={handleSubmitPersona}
    />
  );
  return (
    <UserProfilePanelFrame
      addAgentToChannelDialog={addAgentToChannelDialog}
      canResetWidth={canResetWidth}
      editAgentDialog={editAgentDialog}
      headerActions={headerActions}
      headerLeftContent={headerLeftContent}
      isFloatingOverlay={isFloatingOverlay}
      isOverlay={isOverlay}
      isSinglePanelView={isSinglePanelView}
      isSplitLayout={isSplitLayout}
      onClose={onClose}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      personaDialogs={personaDialogs}
      profileBody={profileBody}
      splitPaneClamp={splitPaneClamp}
      widthPx={widthPx}
      transparentChrome={transparentChrome}
    />
  );
}
