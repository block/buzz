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
  type BackendIntent,
} from "./lib/instanceInputForDefinition";
import { useCreatedAgentChannelAttachment } from "./useCreatedAgentChannelAttachment";
import { classifyAgentManagementOrigin } from "./agentManagementBuffer";
import { useChannelsQuery } from "@/features/channels/hooks";
import { resolveManagedAgentAvatarUrl } from "./ui/managedAgentAvatar";
import type { AgentCreateIntent } from "./ui/agentCreateIntent";
import { editPersonaDialogState } from "./ui/personaDialogState";
import { attachManagedAgentToChannel } from "./channelAgents";
import { hasDirectAgentCreationGrant } from "./directAgentCreationGrant";
import {
  beginDirectAgentCreation,
  getDirectAgentCreationResult,
  recordDirectAgentCreationResult,
} from "./directAgentCreationJournal";
import {
  directAgentCreationResultContent,
  type DirectAgentCreationResult,
} from "./directAgentCreationResult";
import { getDefaultPersonaRuntime } from "./lib/resolvePersonaRuntime";
import { useGlobalAgentConfig } from "./useGlobalAgentConfig";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import type {
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

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
  const identityQuery = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: true });
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const { globalConfig, isLoading: isGlobalConfigLoading } =
    useGlobalAgentConfig();
  const [request, setRequest] = React.useState<AgentManagementRequest | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);
  const createdAgentAttachment = useCreatedAgentChannelAttachment();
  const seenRequestIds = React.useRef(new Set<string>());
  const pendingRequestId = React.useRef<string | null>(null);
  const sourceAgentPubkey = React.useRef<string | null>(null);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const channelsRef = React.useRef(channelsQuery.data);
  const bufferedRequestsRef = React.useRef<
    Array<{ agentPubkey: string; request: AgentManagementRequest }>
  >([]);
  const directRequestInFlight = React.useRef<string | null>(null);

  const dismiss = React.useEffectEvent(() => {
    pendingRequestId.current = null;
    sourceAgentPubkey.current = null;
    directRequestInFlight.current = null;
    setRequest(null);
  });

  const acceptOwnedRequest = React.useEffectEvent(
    (agentPubkey: string, next: AgentManagementRequest) => {
      if (
        classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          agentPubkey,
          next.request.channelId,
        ) !== "accept" ||
        (next.action !== "create_direct" &&
          seenRequestIds.current.has(next.requestId))
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
    channelsRef.current = channelsQuery.data;
    if (managedAgentsQuery.data && channelsQuery.data) {
      const buffered = bufferedRequestsRef.current.splice(0);
      for (const candidate of buffered) {
        acceptOwnedRequest(candidate.agentPubkey, candidate.request);
      }
    }
  }, [channelsQuery.data, managedAgentsQuery.data]);

  React.useEffect(
    () =>
      subscribeAgentManagementRequests((agentPubkey, next) => {
        // Observer frames are owner-scoped and authenticated. Any managed agent
        // this Desktop owns may draft a change; defer the ownership decision
        // until the managed-agent query has initialized so ephemeral requests
        // cannot disappear during startup.
        if (
          classifyAgentManagementOrigin(
            managedAgentsRef.current,
            channelsRef.current,
            agentPubkey,
            next.request.channelId,
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
      const persona = await createPersonaMutation.mutateAsync({
        ...input,
        avatarUrl,
      });

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

  async function publishDirectResult(
    result: DirectAgentCreationResult,
    request: Extract<AgentManagementRequest, { action: "create_direct" }>,
    requesterPubkey: string,
    expectedRelayUrl: string,
    expectedSignerPubkey: string,
  ) {
    await sendChannelMessage(
      request.request.channelId,
      directAgentCreationResultContent(result),
      request.request.replyTo ?? null,
      undefined,
      [requesterPubkey],
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      expectedRelayUrl,
      expectedSignerPubkey,
    );
  }

  async function publishDirectResultWithRetry(
    result: DirectAgentCreationResult,
    request: Extract<AgentManagementRequest, { action: "create_direct" }>,
    requesterPubkey: string,
    expectedRelayUrl: string,
    expectedSignerPubkey: string,
  ) {
    let lastError: unknown;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        await publishDirectResult(
          result,
          request,
          requesterPubkey,
          expectedRelayUrl,
          expectedSignerPubkey,
        );
        return;
      } catch (cause) {
        lastError = cause;
      }
    }
    throw lastError;
  }

  const processDirectCreate = React.useEffectEvent(
    async (
      directRequest: Extract<
        AgentManagementRequest,
        { action: "create_direct" }
      >,
      requesterPubkey: string,
    ) => {
      const expectedRelayUrl = activeCommunity?.relayUrl.trim() ?? "";
      const expectedSignerPubkey =
        identityQuery.data?.pubkey.trim().toLowerCase() ?? "";
      if (!expectedRelayUrl || !expectedSignerPubkey) {
        throw new Error("The active community identity is not ready.");
      }
      const replay = getDirectAgentCreationResult(
        expectedSignerPubkey,
        directRequest.requestId,
      );
      if (replay) {
        await publishDirectResultWithRetry(
          replay,
          directRequest,
          requesterPubkey,
          expectedRelayUrl,
          expectedSignerPubkey,
        );
        dismiss();
        return;
      }

      let result: DirectAgentCreationResult;
      try {
        assertAgentCanActFromOrigin(directRequest.request.channelId);
        if (
          !hasDirectAgentCreationGrant(expectedSignerPubkey, requesterPubkey)
        ) {
          result = {
            requestId: directRequest.requestId,
            status: "denied",
            displayName: directRequest.request.displayName,
            message:
              "This agent does not have a standing direct-creation grant in Settings.",
          };
        } else {
          beginDirectAgentCreation(
            expectedSignerPubkey,
            directRequest.requestId,
            directRequest.request.displayName,
          );
          const runtimes = await availableRuntimesForStart(runtimesQuery);
          const runtime = getDefaultPersonaRuntime(
            runtimes,
            globalConfig.preferred_runtime,
          );
          if (!runtime) {
            throw new Error(
              "No available default runtime is configured for direct creation.",
            );
          }

          const avatarUrl = await resolveManagedAgentAvatarUrl(
            undefined,
            undefined,
            runtime.avatarUrl,
          );
          const persona = await createPersonaMutation.mutateAsync({
            ...createInputFromRequest(directRequest),
            avatarUrl,
            runtime: runtime.id,
            provider: globalConfig.provider ?? undefined,
            model: globalConfig.model ?? undefined,
            behavior: { respondTo: "owner-only" },
          });
          const instanceInput = await buildInstanceInputForDefinition(
            persona,
            runtime,
          );
          const created = await createAgentMutation.mutateAsync({
            ...instanceInput,
            relayUrl: expectedRelayUrl,
            expectedRelayUrl,
            expectedSignerPubkey,
          });
          if (created.spawnError) throw new Error(created.spawnError);
          if (created.profileSyncError)
            throw new Error(created.profileSyncError);
          const attached = await attachManagedAgentToChannel(
            directRequest.request.channelId,
            {
              agent: created.agent,
              role: "bot",
              ensureRunning: true,
              expectedRelayUrl,
              expectedSignerPubkey,
            },
          );
          result = {
            requestId: directRequest.requestId,
            status: "created",
            displayName: attached.agent.name,
            agentPubkey: attached.agent.pubkey,
            message: "Agent created and added to the originating channel.",
          };
        }
      } catch (cause) {
        result = {
          requestId: directRequest.requestId,
          status: "failed",
          displayName: directRequest.request.displayName,
          message:
            cause instanceof Error ? cause.message : "Direct creation failed.",
        };
      }

      recordDirectAgentCreationResult(expectedSignerPubkey, result);
      await publishDirectResultWithRetry(
        result,
        directRequest,
        requesterPubkey,
        expectedRelayUrl,
        expectedSignerPubkey,
      );
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
    },
  );

  React.useEffect(() => {
    if (
      request?.action !== "create_direct" ||
      isGlobalConfigLoading ||
      identityQuery.isLoading ||
      !activeCommunity ||
      directRequestInFlight.current === request.requestId
    ) {
      return;
    }
    const requesterPubkey = sourceAgentPubkey.current;
    if (!requesterPubkey) return;
    directRequestInFlight.current = request.requestId;
    void processDirectCreate(request, requesterPubkey).catch((cause) => {
      console.error("Direct agent creation acknowledgement failed", cause);
      setError(
        cause instanceof Error
          ? cause.message
          : "Direct agent creation acknowledgement failed.",
      );
      dismiss();
    });
  }, [
    activeCommunity,
    identityQuery.isLoading,
    isGlobalConfigLoading,
    request,
  ]);

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
    runtimeCatalogStatus: runtimesQuery.isLoading
      ? ("loading" as const)
      : runtimesQuery.isError
        ? ("error" as const)
        : ("ready" as const),
    submitCreate,
    submitUpdate,
    dismiss,
  };
}
