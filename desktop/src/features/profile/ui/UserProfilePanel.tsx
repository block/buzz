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
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { AddAgentToChannelDialog } from "@/features/agents/ui/AddAgentToChannelDialog";
import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
  resolveStartRuntimeForDefinition,
} from "@/features/agents/lib/instanceInputForDefinition";
import {
  isManagedAgentActive,
  startManagedAgentWithRules,
  stopManagedAgentWithRules,
} from "@/features/agents/lib/managedAgentControlActions";
import { describeLogFile } from "@/features/agents/ui/agentUi";
import { AgentDialog } from "@/features/agents/ui/AgentDialog";
import {
  consumePendingOpenEditAgent,
  type EditAgentFocusTarget,
  subscribeOpenEditAgent,
} from "@/features/agents/openEditAgentEvent";
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
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { resolveProfileActivityAgent } from "@/features/profile/lib/profileActivityAgent";
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
import { UserProfileSnapshotExportDialog } from "@/features/profile/ui/UserProfileSnapshotExportDialog";
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
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { AuxiliaryPanelBody } from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";
import type {
  AgentPersona,
  Channel,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { UserProfilePanelFrame } from "@/features/profile/ui/UserProfilePanelFrame";
import { getUserProfilePanelHeaderContent } from "@/features/profile/ui/UserProfilePanelHeaderContent";
export type { ProfilePanelTab, ProfilePanelView };

export function UserProfilePanel({
  callerChannelId = null,
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
  const { globalConfig } = useGlobalAgentConfig();
  const isOverlay = useIsThreadPanelOverlay();
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
  const [editAgentFocus, setEditAgentFocus] = React.useState<
    EditAgentFocusTarget | undefined
  >(undefined);

  // Open the Edit Agent dialog when `requestOpenEditAgent(pubkey)` fires from
  // a card or other non-panel surface (e.g. `ConfigNudgeCard`). Mirrors the
  // `subscribeOpenCreateAgent` pattern in AgentsView.
  React.useEffect(() => {
    if (!pubkey) return;
    // Consume any pending request that arrived before this panel mounted.
    const pending = consumePendingOpenEditAgent(pubkey);
    if (pending !== false) {
      setEditAgentFocus(pending === true ? undefined : pending);
      setEditAgentOpen(true);
    }
    // Subscribe for events that arrive while the panel is mounted.
    return subscribeOpenEditAgent(pubkey, (focus) => {
      setEditAgentFocus(focus);
      setEditAgentOpen(true);
    });
  }, [pubkey]);
  const [addToChannelOpen, setAddToChannelOpen] = React.useState(false);
  const [personaDialogState, setPersonaDialogState] =
    React.useState<PersonaDialogState | null>(null);
  const [personaToDelete, setPersonaToDelete] =
    React.useState<AgentPersona | null>(null);
  const [personaToExportSnapshot, setPersonaToExportSnapshot] =
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
  const personaInstances = React.useMemo(() => {
    if (!managedAgent?.personaId) return managedAgent ? [managedAgent] : [];
    return (managedAgentsQuery.data ?? []).filter(
      (agent) => agent.personaId === managedAgent.personaId,
    );
  }, [managedAgent, managedAgentsQuery.data]);
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
  const { canOpenAgentActivity, openAgentActivity } = useOpenAgentActivity();
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
  const isCurrentUserOwner = ownsAuthorAgent(profile, currentPubkey);
  // The viewer may see owner-scoped data if they declared-own the agent OR they
  // manage it locally (older agents may not advertise an owner pubkey). Every
  // real boundary is server-side, so this only controls what UI we paint.
  const viewerIsOwner = isCurrentUserOwner || isOwner === true;

  const activityAgent = React.useMemo(
    () =>
      resolveProfileActivityAgent({
        effectivePubkey,
        isBot,
        managedAgent,
        profile: profile ?? null,
        relayAgent,
        viewerIsOwner,
      }),
    [effectivePubkey, isBot, managedAgent, profile, relayAgent, viewerIsOwner],
  );
  // Observer ingestion (frame decryption + derived active-turn liveness) is
  // owner-global — mounted once in AppShell via useAgentObserverIngestion —
  // covering both locally managed agents and declared-owned relay agents.
  const canEditAgent =
    isOwner === true &&
    (managedAgent !== undefined || resolvedPersona !== undefined);
  const memoryQuery = useAgentMemoryQuery(effectivePubkey, {
    enabled: viewerIsOwner && Boolean(effectivePubkey),
  });
  const isSelf =
    currentPubkey !== undefined &&
    pubkeyLower.length > 0 &&
    pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity =
    viewerIsOwner &&
    Boolean(effectivePubkey) &&
    canOpenAgentActivity(effectivePubkey);
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
    setPersonaActiveMutation.isPending;
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
    if (resolvedPersona) {
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
      const runtimes = await availableRuntimesForStart(availableRuntimesQuery);
      const { runtime, warnings } = resolveStartRuntimeForDefinition(
        personaToStart,
        runtimes,
        globalConfig.preferred_runtime,
      );

      for (const warning of warnings) {
        toast.warning(warning);
      }

      const input = await buildInstanceInputForDefinition(
        personaToStart,
        runtime,
      );

      const created = await createAgentMutation.mutateAsync(input);
      void managedAgentsQuery.refetch();
      void relayAgentsQuery.refetch();
      return created;
    },
    [
      availableRuntimesQuery,
      createAgentMutation.mutateAsync,
      globalConfig.preferred_runtime,
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
    if (!resolvedPersona) return;
    setPersonaDialogState(editPersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleDuplicatePersona = React.useCallback(() => {
    if (!resolvedPersona) return;
    setPersonaDialogState(duplicatePersonaDialogState(resolvedPersona));
  }, [resolvedPersona]);

  const handleExportPersona = React.useCallback(() => {
    if (resolvedPersona) {
      setPersonaToExportSnapshot(resolvedPersona);
    }
  }, [resolvedPersona]);

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
    [deletePersonaMutation.mutateAsync, onClose],
  );

  // Count of managed-agent instances backed by the persona being deleted.
  // Shown in the confirm dialog so the user knows what will be cascade-deleted.
  const personaDeleteInstanceCount = React.useMemo(
    () =>
      personaToDelete
        ? (managedAgentsQuery.data ?? []).filter(
            (a) => a.personaId === personaToDelete.id,
          ).length
        : 0,
    [managedAgentsQuery.data, personaToDelete],
  );

  const handleAddedToChannel = React.useCallback(
    (channel: Channel, result: AttachManagedAgentToChannelResult) => {
      if (result.started) {
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

  const handleOpenActivity = React.useCallback(
    (channelId?: string | null) => {
      if (!effectivePubkey) return;
      openAgentActivity(effectivePubkey, { channelId: channelId ?? null });
    },
    [effectivePubkey, openAgentActivity],
  );

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
  const canEditPersona = canManagePersona;
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
  const { agentInfoFields, agentSettingsFields, diagnosticsFields } =
    useProfileFieldBuckets({
      isBot,
      isOwner: viewerIsOwner,
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
      view,
      viewerIsOwner,
    },
  );

  const profileBody = (
    <AuxiliaryPanelBody
      className={cn(
        "px-4 pb-6",
        isDiagnosticsLikeView
          ? "flex flex-col overflow-hidden"
          : "overflow-y-auto",
      )}
    >
      {view === "summary" ? (
        <ProfileSummaryView
          canAddToChannel={managedAgent !== undefined && isOwner === true}
          canEditAgent={canEditAgent}
          canInstantiateAgent={canInstantiateAgent}
          canOpenAgentLogs={canOpenAgentLogs}
          canViewActivity={canViewActivity}
          callerChannelId={callerChannelId}
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
          instances={personaInstances}
          activityAgent={activityAgent}
          managedAgent={managedAgent}
          memoriesLoading={memoryQuery.isLoading}
          memoryCount={memoryCount}
          agentInfoFields={agentInfoFields}
          agentSettingsFields={agentSettingsFields}
          diagnosticsFields={diagnosticsFields}
          onAddToChannel={() => setAddToChannelOpen(true)}
          onOpenInstance={(instancePubkey) => onOpenProfile?.(instancePubkey)}
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
        <AgentConfigurationFocusedView fields={agentSettingsFields} />
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
    </AuxiliaryPanelBody>
  );
  const editAgentDialog =
    canEditAgent && managedAgent ? (
      <AgentDialog
        agent={managedAgent}
        mode="instance-edit"
        initialFocus={editAgentFocus}
        onEditLinkedPersona={
          resolvedPersona && !resolvedPersona.isBuiltIn
            ? () => {
                setEditAgentOpen(false);
                setEditAgentFocus(undefined);
                setPersonaDialogState(editPersonaDialogState(resolvedPersona));
              }
            : undefined
        }
        onOpenChange={(next) => {
          setEditAgentOpen(next);
          if (!next) setEditAgentFocus(undefined);
        }}
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
    <>
      <UserProfilePersonaDialogs
        createError={
          createPersonaMutation.error instanceof Error
            ? createPersonaMutation.error
            : null
        }
        instanceCount={personaDeleteInstanceCount}
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
      {personaToExportSnapshot ? (
        <UserProfileSnapshotExportDialog
          linkedAgentPubkey={managedAgent?.pubkey ?? null}
          onOpenChange={(open) => {
            if (!open) setPersonaToExportSnapshot(null);
          }}
          persona={personaToExportSnapshot}
        />
      ) : null}
    </>
  );
  return (
    <UserProfilePanelFrame
      addAgentToChannelDialog={addAgentToChannelDialog}
      canResetWidth={canResetWidth}
      editAgentDialog={editAgentDialog}
      headerActions={headerActions}
      headerLeftContent={headerLeftContent}
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
