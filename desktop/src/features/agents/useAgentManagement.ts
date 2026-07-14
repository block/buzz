import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  createInputFromRequest,
  requestTargetsEditablePersona,
  type AgentManagementRequest,
} from "./agentManagement";
import { subscribeAgentManagementRequests } from "./observerRelayStore";
import {
  managedAgentsQueryKey,
  personasQueryKey,
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useUpdatePersonaMutation,
} from "./hooks";
import {
  availableRuntimesForStart,
  buildInstanceInputForDefinition,
  mintDefinitionWithPreflight,
  type BackendIntent,
} from "./lib/instanceInputForDefinition";
import { useCreatedAgentChannelAttachment } from "./useCreatedAgentChannelAttachment";
import { classifyAgentManagementSender } from "./agentManagementBuffer";
import { useChannelsQuery } from "@/features/channels/hooks";
import { resolveManagedAgentAvatarUrl } from "./ui/managedAgentAvatar";
import type { AgentCreateIntent } from "./ui/agentCreateIntent";
import { editPersonaDialogState } from "./ui/personaDialogState";
import type {
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { meshPrepareRelayMeshClient } from "@/shared/api/tauriMesh";

function updateInputFromRequest(
  request: Extract<AgentManagementRequest, { action: "update" }>,
  current: UpdatePersonaInput,
): UpdatePersonaInput {
  const changes = request.request;
  return {
    ...current,
    displayName: changes.displayName ?? current.displayName,
    systemPrompt: changes.systemPrompt ?? current.systemPrompt,
    runtime: changes.runtime ?? current.runtime,
    provider: changes.provider ?? current.provider,
    model: changes.model ?? current.model,
    ...(changes.respondTo
      ? {
          behavior: {
            respondTo: changes.respondTo,
            respondToAllowlist: [],
            parallelism: current.behavior?.parallelism,
          },
        }
      : {}),
  };
}

export function useAgentManagement() {
  const queryClient = useQueryClient();
  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: true });
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const [request, setRequest] = React.useState<AgentManagementRequest | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);
  const createdAgentAttachment = useCreatedAgentChannelAttachment();
  const seenRequestIds = React.useRef(new Set<string>());
  const pendingRequestId = React.useRef<string | null>(null);
  const sourceAgentPubkey = React.useRef<string | null>(null);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const bufferedRequestsRef = React.useRef<
    Array<{ agentPubkey: string; request: AgentManagementRequest }>
  >([]);

  const acceptOwnedRequest = React.useEffectEvent(
    (agentPubkey: string, next: AgentManagementRequest) => {
      if (
        classifyAgentManagementSender(managedAgentsRef.current, agentPubkey) !==
          "accept" ||
        seenRequestIds.current.has(next.requestId)
      ) {
        return;
      }
      seenRequestIds.current.add(next.requestId);
      setError(null);
      if (pendingRequestId.current === null) {
        pendingRequestId.current = next.requestId;
        sourceAgentPubkey.current = agentPubkey;
        setRequest(next);
      }
    },
  );

  React.useEffect(() => {
    managedAgentsRef.current = managedAgentsQuery.data;
    if (managedAgentsQuery.data) {
      const buffered = bufferedRequestsRef.current.splice(0);
      for (const candidate of buffered) {
        acceptOwnedRequest(candidate.agentPubkey, candidate.request);
      }
    }
  }, [managedAgentsQuery.data]);

  React.useEffect(
    () =>
      subscribeAgentManagementRequests((agentPubkey, next) => {
        // Observer frames are owner-scoped and authenticated. Any managed agent
        // this Desktop owns may draft a change; defer the ownership decision
        // until the managed-agent query has initialized so ephemeral requests
        // cannot disappear during startup.
        if (
          classifyAgentManagementSender(
            managedAgentsRef.current,
            agentPubkey,
          ) === "buffer"
        ) {
          bufferedRequestsRef.current.push({ agentPubkey, request: next });
          if (bufferedRequestsRef.current.length > 100) {
            bufferedRequestsRef.current.shift();
          }
          return;
        }
        acceptOwnedRequest(agentPubkey, next);
      }),
    [],
  );

  const matchingPersonas = React.useMemo(() => {
    if (request?.action !== "update") return [];
    const target = request.request.agentName.trim().toLocaleLowerCase();
    return (personasQuery.data ?? []).filter(
      (persona) =>
        persona.displayName.trim().toLocaleLowerCase() === target &&
        requestTargetsEditablePersona(persona),
    );
  }, [personasQuery.data, request]);
  const currentPersona =
    matchingPersonas.length === 1 ? matchingPersonas[0] : undefined;

  const isPending =
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    createAgentMutation.isPending;

  function assertAgentCanActFromOrigin(channelId: string) {
    const targetChannel = (channelsQuery.data ?? []).find(
      (channel) => channel.id === channelId,
    );
    const requestingPubkey = sourceAgentPubkey.current?.toLowerCase();
    if (
      !targetChannel?.isMember ||
      !requestingPubkey ||
      !targetChannel.memberPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === requestingPubkey,
      )
    ) {
      throw new Error(
        "An agent can only manage agents from a channel you both belong to.",
      );
    }
  }

  async function submitCreate(
    input: CreatePersonaInput | UpdatePersonaInput,
    intent: AgentCreateIntent,
    backendIntent: BackendIntent | null,
  ): Promise<boolean> {
    if (request?.action !== "create" || "id" in input) {
      return false;
    }
    setError(null);
    try {
      assertAgentCanActFromOrigin(request.request.channelId);
      const runtimes = await availableRuntimesForStart(runtimesQuery);
      const runtime = runtimes.find(
        (candidate) => candidate.id === input.runtime,
      );
      if (!runtime) {
        throw new Error("Choose an available runtime for this agent.");
      }

      const avatarUrl = await resolveManagedAgentAvatarUrl(
        input.avatarUrl,
        undefined,
        runtime.avatarUrl,
      );
      const persona = await mintDefinitionWithPreflight(
        intent === "definition_start" ? backendIntent : null,
        meshPrepareRelayMeshClient,
        () =>
          createPersonaMutation.mutateAsync({
            ...input,
            avatarUrl,
          }),
      );

      if (intent === "definition_start") {
        const created = await createAgentMutation.mutateAsync(
          await buildInstanceInputForDefinition(
            persona,
            runtime,
            undefined,
            backendIntent ?? undefined,
          ),
        );
        if (created.spawnError) throw new Error(created.spawnError);
        const targetChannel = (channelsQuery.data ?? []).find(
          (channel) => channel.id === request.request.channelId,
        );
        await createdAgentAttachment.presentCreatedAgent(created, {
          id: request.request.channelId,
          name: targetChannel?.name ?? "this channel",
        });
      }

      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
      return true;
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save this agent.",
      );
      return false;
    }
  }

  async function submitUpdate(input: CreatePersonaInput | UpdatePersonaInput) {
    if (request?.action !== "update" || !("id" in input)) {
      return false;
    }
    setError(null);
    try {
      assertAgentCanActFromOrigin(request.request.channelId);
      await updatePersonaMutation.mutateAsync(input);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
      return true;
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save this agent.",
      );
      return false;
    }
  }

  function dismiss() {
    pendingRequestId.current = null;
    sourceAgentPubkey.current = null;
    setRequest(null);
  }

  const createInitialValues = React.useMemo(
    () =>
      request?.action === "create" ? createInputFromRequest(request) : null,
    [request],
  );

  const editInitialValues = React.useMemo(() => {
    if (request?.action !== "update" || !currentPersona) return null;
    return updateInputFromRequest(
      request,
      editPersonaDialogState(currentPersona)
        .initialValues as UpdatePersonaInput,
    );
  }, [currentPersona, request]);

  const editError = React.useMemo(() => {
    if (request?.action !== "update") return error;
    if (error) return error;
    if (matchingPersonas.length > 1) {
      return "More than one personal agent has that name. Rename it in Agents, then ask the agent again.";
    }
    if (!currentPersona) {
      return "Agents can only update a personal agent profile by its current name.";
    }
    return null;
  }, [currentPersona, error, matchingPersonas.length, request]);

  return {
    request,
    createInitialValues,
    editInitialValues,
    editError,
    error,
    ...createdAgentAttachment,
    isPending,
    runtimes: runtimesQuery.data ?? [],
    runtimesLoading: runtimesQuery.isLoading,
    submitCreate,
    submitUpdate,
    dismiss,
  };
}
