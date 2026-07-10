import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  createInputFromFizzRequest,
  fizzRequestTargetsEditablePersona,
  type FizzAgentManagementRequest,
} from "./fizzAgentManagement";
import { subscribeFizzAgentManagementRequests } from "./observerRelayStore";
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
import { attachManagedAgentToChannel } from "./channelAgents";
import { useChannelsQuery } from "@/features/channels/hooks";
import { resolveManagedAgentAvatarUrl } from "./ui/managedAgentAvatar";
import type { AgentCreateIntent } from "./ui/agentCreateIntent";
import type {
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { meshPrepareRelayMeshClient } from "@/shared/api/tauriMesh";

const FIZZ_PERSONA_ID = "builtin:fizz";

function updateInput(
  request: Extract<FizzAgentManagementRequest, { action: "update" }>,
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
            mcpToolsets: current.behavior?.mcpToolsets,
            parallelism: current.behavior?.parallelism,
          },
        }
      : {}),
  };
}

export function useFizzAgentManagement() {
  const queryClient = useQueryClient();
  const personasQuery = usePersonasQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: true });
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const createAgentMutation = useCreateManagedAgentMutation();
  const [request, setRequest] =
    React.useState<FizzAgentManagementRequest | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const seenRequestIds = React.useRef(new Set<string>());
  const pendingRequestId = React.useRef<string | null>(null);
  const sourceAgentPubkey = React.useRef<string | null>(null);

  React.useEffect(
    () =>
      subscribeFizzAgentManagementRequests((agentPubkey, next) => {
        // Observer frames are owner-scoped and authenticated, but only Fizz gets
        // this extra product authority. Other agents' tool telemetry cannot open
        // a configuration review surface.
        const isFizz = (managedAgentsQuery.data ?? []).some(
          (agent) =>
            agent.pubkey.toLowerCase() === agentPubkey.toLowerCase() &&
            agent.personaId === FIZZ_PERSONA_ID,
        );
        if (!isFizz || seenRequestIds.current.has(next.requestId)) return;
        seenRequestIds.current.add(next.requestId);
        setError(null);
        if (pendingRequestId.current === null) {
          pendingRequestId.current = next.requestId;
          sourceAgentPubkey.current = agentPubkey;
          setRequest(next);
        }
      }),
    [managedAgentsQuery.data],
  );

  const matchingPersonas = React.useMemo(() => {
    if (request?.action !== "update") return [];
    const target = request.request.agentName.trim().toLocaleLowerCase();
    return (personasQuery.data ?? []).filter(
      (persona) =>
        persona.displayName.trim().toLocaleLowerCase() === target &&
        fizzRequestTargetsEditablePersona(persona),
    );
  }, [personasQuery.data, request]);
  const currentPersona =
    matchingPersonas.length === 1 ? matchingPersonas[0] : undefined;

  const isPending =
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    createAgentMutation.isPending;

  function assertFizzCanActFromOrigin(channelId: string) {
    const targetChannel = (channelsQuery.data ?? []).find(
      (channel) => channel.id === channelId,
    );
    const fizzPubkey = sourceAgentPubkey.current?.toLowerCase();
    if (
      !targetChannel?.isMember ||
      !fizzPubkey ||
      !targetChannel.memberPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === fizzPubkey,
      )
    ) {
      throw new Error(
        "Fizz can only manage agents from a channel you both belong to.",
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
      assertFizzCanActFromOrigin(request.request.channelId);
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
        await attachManagedAgentToChannel(request.request.channelId, {
          agent: created.agent,
          role: "bot",
          ensureRunning: true,
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

  async function confirmUpdate() {
    if (request?.action !== "update") return;
    setError(null);
    try {
      assertFizzCanActFromOrigin(request.request.channelId);
      if (!currentPersona) {
        throw new Error(
          matchingPersonas.length > 1
            ? "More than one personal agent has that name. Rename it in Agents, then ask Fizz again."
            : "Fizz can only update a personal agent profile by its current name.",
        );
      }
      const persona = currentPersona;
      const current: UpdatePersonaInput = {
        id: persona.id,
        displayName: persona.displayName,
        avatarUrl: persona.avatarUrl ?? undefined,
        systemPrompt: persona.systemPrompt,
        runtime: persona.runtime ?? undefined,
        provider: persona.provider ?? undefined,
        model: persona.model ?? undefined,
        namePool: persona.namePool,
        // Never route stored environment variables through this feature.
        behavior: {
          respondTo: persona.respondTo ?? undefined,
          respondToAllowlist: persona.respondToAllowlist,
          mcpToolsets: persona.mcpToolsets ?? undefined,
          parallelism: persona.parallelism ?? undefined,
        },
      };
      await updatePersonaMutation.mutateAsync(updateInput(request, current));
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: personasQueryKey }),
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
      ]);
      dismiss();
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not save this agent.",
      );
    }
  }

  function dismiss() {
    pendingRequestId.current = null;
    sourceAgentPubkey.current = null;
    setRequest(null);
  }

  const createInitialValues = React.useMemo(
    () =>
      request?.action === "create" ? createInputFromFizzRequest(request) : null,
    [request],
  );

  return {
    request,
    createInitialValues,
    currentPersona,
    error,
    isPending,
    runtimes: runtimesQuery.data ?? [],
    runtimesLoading: runtimesQuery.isLoading,
    submitCreate,
    confirmUpdate,
    dismiss,
  };
}
