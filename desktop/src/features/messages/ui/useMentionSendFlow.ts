import * as React from "react";
import { toast } from "sonner";

import {
  useAttachManagedAgentToChannelMutation,
  useAvailableAcpRuntimes,
  useCreateChannelManagedAgentMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { useAddChannelMembersMutation } from "@/features/channels/hooks";
import type { UseChannelLinksResult } from "@/features/messages/lib/useChannelLinks";
import type { UseEmojiAutocompleteResult } from "@/features/messages/lib/useEmojiAutocomplete";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import type { UseDraftsResult } from "@/features/messages/lib/useDrafts";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { getAgentModels } from "@/shared/api/tauri";
import { meshInstalledModels } from "@/shared/api/tauriMesh";
import type {
  AcpRuntime,
  AgentModelInfo,
  AgentModelsResponse,
  AgentPersona,
  ChannelType,
  ManagedAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MENTION_REFERENCE_TAG } from "@/shared/lib/resolveMentionNames";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { AgentMentionModelTarget } from "./AgentMentionModelSelector";

type PendingNonMemberMentionSend = {
  finalContent: string;
  mentionPubkeys: string[];
  nonMemberPubkeys: string[];
  outgoingTags?: string[][];
  readyAgentPubkeys?: string[];
  savedContent: string;
  savedImeta: ImetaMedia[];
  sentDraftKey: string | null | undefined;
};

type PendingAgentMentionModelTarget = AgentMentionModelTarget & {
  existingAgent: ManagedAgent | null;
  existingPubkey: string | null;
  persona: AgentPersona | null;
};

type SendMessageWithMentionFlowInput = {
  pendingImeta: ImetaMedia[];
  sentDraftKey: string | null | undefined;
  trimmed: string;
};

type UseMentionSendFlowOptions = {
  channelId: string | null;
  channelLinks: Pick<UseChannelLinksResult, "clearChannels">;
  channelType: ChannelType | null;
  content: string;
  contentRef: React.MutableRefObject<string>;
  customEmoji: CustomEmoji[];
  drafts: Pick<UseDraftsResult, "clearDraft">;
  emojiAutocomplete: Pick<UseEmojiAutocompleteResult, "clearEmojis">;
  mentions: UseMentionsResult;
  onSendRef: React.MutableRefObject<
    (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => Promise<void>
  >;
  richText: Pick<UseRichTextEditorResult, "clearContent" | "setContent">;
  setContent: React.Dispatch<React.SetStateAction<string>>;
  setIsEmojiPickerOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setPendingImeta: (pendingImeta: ImetaMedia[]) => void;
};

function mergeOutgoingTagsWithReferenceMentions(
  outgoingTags: string[][] | undefined,
  pubkeys: Iterable<string>,
) {
  const normalizedPubkeys = uniqueNormalizedPubkeys(pubkeys);
  if (normalizedPubkeys.length === 0) {
    return outgoingTags;
  }

  return [
    ...(outgoingTags ?? []),
    ...normalizedPubkeys.map((pubkey) => [MENTION_REFERENCE_TAG, pubkey]),
  ];
}

function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

function uniqueNormalizedPubkeys(pubkeys: Iterable<string>) {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

function normalizeModelId(model: string | null | undefined) {
  const trimmed = model?.trim();
  return trimmed ? trimmed : null;
}

function addModelOption(
  options: AgentModelInfo[],
  seen: Set<string>,
  model: string | null | undefined,
) {
  const normalized = normalizeModelId(model);
  if (!normalized || seen.has(normalized)) {
    return;
  }

  seen.add(normalized);
  options.push({
    id: normalized,
    name: null,
    description: null,
  });
}

function buildModelOptions(
  catalogOptions: AgentModelInfo[],
  fallbackModels: Array<string | null | undefined>,
) {
  const seen = new Set<string>();
  const options: AgentModelInfo[] = [];

  for (const model of catalogOptions) {
    if (!model.id || seen.has(model.id)) {
      continue;
    }
    seen.add(model.id);
    options.push(model);
  }

  for (const model of fallbackModels) {
    addModelOption(options, seen, model);
  }

  return options;
}

function didModelSelectionChange(target: PendingAgentMentionModelTarget) {
  if (!target.existingAgent) {
    return true;
  }

  return (
    normalizeModelId(target.selectedModel) !==
    normalizeModelId(target.existingAgent.model)
  );
}

function selectedModelForTarget({
  currentModel,
  hasExistingAgent,
  key,
  modelOptions,
  selections,
}: {
  currentModel: string | null;
  hasExistingAgent: boolean;
  key: string;
  modelOptions: AgentModelInfo[];
  selections: Map<string, string | null>;
}) {
  if (selections.has(key)) {
    return selections.get(key) ?? null;
  }

  if (currentModel) {
    return currentModel;
  }

  if (hasExistingAgent) {
    return null;
  }

  return modelOptions[0]?.id ?? null;
}

function runtimeFromManagedAgent(agent: ManagedAgent): AcpRuntime {
  return {
    id: agent.agentCommand,
    label: agent.agentCommand,
    availability: "available",
    command: agent.agentCommand,
    binaryPath: agent.agentCommand,
    defaultArgs: agent.agentArgs,
    mcpCommand: agent.mcpCommand || null,
    avatarUrl: "",
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
  };
}

function isManagedAgentRunning(agent: ManagedAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

function isProviderBackedAgent(agent: ManagedAgent) {
  return agent.backend.type === "provider";
}

export function useMentionSendFlow({
  channelId,
  channelLinks,
  channelType,
  content,
  contentRef,
  customEmoji,
  drafts,
  emojiAutocomplete,
  mentions,
  onSendRef,
  richText,
  setContent,
  setIsEmojiPickerOpen,
  setPendingImeta,
}: UseMentionSendFlowOptions) {
  const [pendingNonMemberSend, setPendingNonMemberSend] =
    React.useState<PendingNonMemberMentionSend | null>(null);
  const [agentModelTargets, setAgentModelTargets] = React.useState<
    PendingAgentMentionModelTarget[]
  >([]);
  const [isLoadingAgentModelTargets, setIsLoadingAgentModelTargets] =
    React.useState(false);
  const [nonMemberPromptError, setNonMemberPromptError] = React.useState<
    string | null
  >(null);
  const [agentModelPromptError, setAgentModelPromptError] = React.useState<
    string | null
  >(null);
  const [isMentionSendPending, setIsMentionSendPending] = React.useState(false);
  const [isCompleteSendPending, setIsCompleteSendPending] =
    React.useState(false);
  const isMentionSendPendingRef = React.useRef(false);
  const isCompleteSendPendingRef = React.useRef(false);
  const previousChannelIdRef = React.useRef(channelId);
  const agentModelSelectionsRef = React.useRef<Map<string, string | null>>(
    new Map(),
  );
  const agentModelCatalogCacheRef = React.useRef<
    Map<string, Promise<AgentModelsResponse> | AgentModelsResponse>
  >(new Map());
  const globalModelCatalogCacheRef = React.useRef<
    Promise<AgentModelInfo[]> | AgentModelInfo[] | null
  >(null);
  const agentModelTargetsRequestRef = React.useRef(0);

  const addMembersMutation = useAddChannelMembersMutation(channelId);
  const attachAgentMutation = useAttachManagedAgentToChannelMutation(channelId);
  const createPersonaAgentMutation =
    useCreateChannelManagedAgentMutation(channelId);
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const managedAgentsQuery = useManagedAgentsQuery();
  const personasQuery = usePersonasQuery();
  const startAgentMutation = useStartManagedAgentMutation();

  const activeAgentPersonas = React.useMemo(
    () => (personasQuery.data ?? []).filter((persona) => persona.isActive),
    [personasQuery.data],
  );

  const getManagedAgentsByPubkey = React.useCallback(async () => {
    const agents =
      managedAgentsQuery.data ??
      (await managedAgentsQuery.refetch()).data ??
      [];

    return new Map(
      agents.map((agent) => [normalizePubkey(agent.pubkey), agent]),
    );
  }, [managedAgentsQuery.data, managedAgentsQuery.refetch]);

  const getPersonasById = React.useCallback(async () => {
    const personas =
      personasQuery.data ?? (await personasQuery.refetch()).data ?? [];

    return new Map(personas.map((persona) => [persona.id, persona]));
  }, [personasQuery.data, personasQuery.refetch]);

  const getAvailableRuntimes = React.useCallback(async (): Promise<
    AcpRuntime[]
  > => {
    const cached = availableRuntimesQuery.data ?? [];
    if (cached.length > 0 || !availableRuntimesQuery.isLoading) {
      return cached;
    }

    const refetched = await availableRuntimesQuery.refetch();
    return (refetched.data ?? []).filter(
      (runtime): runtime is AcpRuntime =>
        runtime.availability === "available" &&
        runtime.command !== null &&
        runtime.binaryPath !== null,
    );
  }, [
    availableRuntimesQuery.data,
    availableRuntimesQuery.isLoading,
    availableRuntimesQuery.refetch,
  ]);

  const loadAgentModelCatalog = React.useCallback(
    async (agent: ManagedAgent): Promise<AgentModelsResponse> => {
      const pubkey = normalizePubkey(agent.pubkey);
      const cached = agentModelCatalogCacheRef.current.get(pubkey);
      if (cached) {
        return await cached;
      }

      const request = getAgentModels(agent.pubkey);
      agentModelCatalogCacheRef.current.set(pubkey, request);
      try {
        const catalog = await request;
        agentModelCatalogCacheRef.current.set(pubkey, catalog);
        return catalog;
      } catch (error) {
        agentModelCatalogCacheRef.current.delete(pubkey);
        throw error;
      }
    },
    [],
  );

  const loadGlobalModelOptions = React.useCallback(async () => {
    const cached = globalModelCatalogCacheRef.current;
    if (cached) {
      return await cached;
    }

    const request = meshInstalledModels().then((models) =>
      models.map((model) => ({
        id: model.id,
        name: model.name,
        description: null,
      })),
    );
    globalModelCatalogCacheRef.current = request;
    try {
      const models = await request;
      globalModelCatalogCacheRef.current = models;
      return models;
    } catch (error) {
      globalModelCatalogCacheRef.current = null;
      throw error;
    }
  }, []);

  const buildAgentModelTarget = React.useCallback(
    async ({
      displayName,
      existingAgent,
      key,
      persona,
    }: {
      displayName: string;
      existingAgent: ManagedAgent | null;
      key: string;
      persona: AgentPersona | null;
    }): Promise<PendingAgentMentionModelTarget> => {
      let catalogOptions: AgentModelInfo[] = [];
      let globalModelOptions: AgentModelInfo[] = [];
      let defaultModel: string | null = null;
      let loadError: string | null = null;

      if (existingAgent) {
        try {
          const catalog = await loadAgentModelCatalog(existingAgent);
          catalogOptions = catalog.models;
          defaultModel = normalizeModelId(catalog.agentDefaultModel);
        } catch (error) {
          loadError =
            error instanceof Error
              ? error.message
              : "Could not load model list.";
        }
      }

      try {
        globalModelOptions = await loadGlobalModelOptions();
      } catch {
        // Global model discovery is only an enhancement for first-time persona
        // mentions. Existing agents still rely on their own ACP model catalog.
      }

      const modelOptions = buildModelOptions(
        [...catalogOptions, ...globalModelOptions],
        [existingAgent?.model, persona?.model, defaultModel],
      );
      const currentModel = existingAgent
        ? normalizeModelId(existingAgent.model)
        : (normalizeModelId(persona?.model) ?? defaultModel);
      const selectedModel = selectedModelForTarget({
        currentModel,
        hasExistingAgent: existingAgent !== null,
        key,
        modelOptions,
        selections: agentModelSelectionsRef.current,
      });
      const hasExplicitModelSelection =
        agentModelSelectionsRef.current.has(key);
      const existingPubkey = existingAgent
        ? normalizePubkey(existingAgent.pubkey)
        : null;
      const isNewMention =
        !existingPubkey || !mentions.memberPubkeys.has(existingPubkey);

      const target: PendingAgentMentionModelTarget = {
        key,
        displayName,
        personaId: persona?.id ?? existingAgent?.personaId ?? null,
        avatarUrl: persona?.avatarUrl ?? null,
        currentModel,
        defaultModel,
        selectedModel,
        modelOptions,
        loadError,
        isNewMention,
        showModelInTrigger: existingAgent !== null || hasExplicitModelSelection,
        willCreateNewInstance: true,
        existingAgent,
        existingPubkey,
        persona,
      };
      target.willCreateNewInstance = didModelSelectionChange(target);
      return target;
    },
    [loadAgentModelCatalog, loadGlobalModelOptions, mentions.memberPubkeys],
  );

  const collectAgentModelTargets = React.useCallback(
    async (trimmed: string): Promise<PendingAgentMentionModelTarget[]> => {
      const personaMentions = mentions.extractMentionPersonas(trimmed);
      const mentionedPubkeys = uniqueNormalizedPubkeys(
        mentions.extractMentionPubkeys(trimmed),
      );
      if (personaMentions.length === 0 && mentionedPubkeys.length === 0) {
        return [];
      }

      const [managedAgentsByPubkey, personasById] = await Promise.all([
        getManagedAgentsByPubkey(),
        getPersonasById(),
      ]);
      const managedAgents = [...managedAgentsByPubkey.values()];
      const targets: PendingAgentMentionModelTarget[] = [];
      const seenKeys = new Set<string>();

      for (const { displayName, persona } of personaMentions) {
        const existingInChannel =
          managedAgents.find(
            (agent) =>
              agent.personaId === persona.id &&
              mentions.memberPubkeys.has(normalizePubkey(agent.pubkey)),
          ) ?? null;
        const key = `persona:${persona.id}:${displayName}`;
        if (seenKeys.has(key)) {
          continue;
        }
        seenKeys.add(key);
        targets.push(
          await buildAgentModelTarget({
            displayName,
            existingAgent: existingInChannel,
            key,
            persona,
          }),
        );
      }

      for (const pubkey of mentionedPubkeys) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) {
          continue;
        }
        const key = `agent:${pubkey}`;
        if (seenKeys.has(key)) {
          continue;
        }
        seenKeys.add(key);
        const persona = agent.personaId
          ? (personasById.get(agent.personaId) ?? null)
          : null;
        targets.push(
          await buildAgentModelTarget({
            displayName:
              mentions.getMentionDisplayName(pubkey) ??
              persona?.displayName ??
              agent.name,
            existingAgent: agent,
            key,
            persona,
          }),
        );
      }

      return targets;
    },
    [
      buildAgentModelTarget,
      getManagedAgentsByPubkey,
      getPersonasById,
      mentions.extractMentionPersonas,
      mentions.extractMentionPubkeys,
      mentions.getMentionDisplayName,
      mentions.memberPubkeys,
    ],
  );

  const ensureManagedAgentMentionsReady = React.useCallback(
    async (mentionPubkeys: string[]) => {
      if (!channelId || mentionPubkeys.length === 0) {
        return [];
      }

      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      const errors: string[] = [];

      for (const pubkey of uniqueNormalizedPubkeys(mentionPubkeys)) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) {
          continue;
        }

        try {
          if (mentions.memberPubkeys.has(pubkey)) {
            if (isProviderBackedAgent(agent)) {
              if (agent.status !== "deployed") {
                await startAgentMutation.mutateAsync(agent.pubkey);
              }
            } else if (!isManagedAgentRunning(agent)) {
              await startAgentMutation.mutateAsync(agent.pubkey);
            }
          } else {
            await attachAgentMutation.mutateAsync({
              agent,
              role: "bot",
            });
          }
        } catch (error) {
          errors.push(
            `${agent.name}: ${getErrorMessage(
              error,
              "Could not prepare agent.",
            )}`,
          );
        }
      }

      return errors;
    },
    [
      attachAgentMutation,
      channelId,
      getManagedAgentsByPubkey,
      mentions.memberPubkeys,
      startAgentMutation,
    ],
  );

  const createMentionedPersonaAgents = React.useCallback(
    async (
      trimmed: string,
      modelTargets: readonly PendingAgentMentionModelTarget[] = [],
    ) => {
      const personaMentions = mentions.extractMentionPersonas(trimmed);
      if (!channelId || personaMentions.length === 0) {
        return {
          errors: [] as string[],
          pubkeys: [] as string[],
        };
      }

      const runtimes = await getAvailableRuntimes();
      const defaultRuntime = runtimes[0] ?? null;
      const errors: string[] = [];
      const pubkeys: string[] = [];
      const seenPersonaIds = new Set<string>();
      const targetByPersonaId = new Map(
        modelTargets
          .filter((target) => target.persona)
          .map((target) => [target.persona?.id, target]),
      );

      for (const { displayName, persona } of personaMentions) {
        if (seenPersonaIds.has(persona.id)) {
          continue;
        }
        seenPersonaIds.add(persona.id);

        const target = targetByPersonaId.get(persona.id);
        if (target?.existingAgent && !didModelSelectionChange(target)) {
          const pubkey = normalizePubkey(target.existingAgent.pubkey);
          pubkeys.push(pubkey);
          mentions.registerMentionPubkey(displayName, pubkey, {
            isAgent: true,
          });
          continue;
        }

        const { runtime } = target?.existingAgent
          ? { runtime: runtimeFromManagedAgent(target.existingAgent) }
          : resolvePersonaRuntime(persona.runtime, runtimes, defaultRuntime);
        if (!runtime) {
          errors.push(`${displayName}: No agent runtime available.`);
          continue;
        }

        try {
          const result = await createPersonaAgentMutation.mutateAsync({
            runtime,
            name: persona.displayName,
            personaId: persona.id,
            systemPrompt: persona.systemPrompt,
            avatarUrl: persona.avatarUrl ?? undefined,
            model:
              normalizeModelId(target ? target.selectedModel : persona.model) ??
              undefined,
            role: "bot",
            ensureRunning: true,
            forceNewInstance: true,
          });
          const pubkey = normalizePubkey(result.agent.pubkey);
          pubkeys.push(pubkey);
          mentions.registerMentionPubkey(displayName, pubkey, {
            isAgent: true,
          });
        } catch (error) {
          errors.push(
            `${displayName}: ${getErrorMessage(
              error,
              "Could not create agent.",
            )}`,
          );
        }
      }

      return {
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
      };
    },
    [
      channelId,
      createPersonaAgentMutation,
      getAvailableRuntimes,
      mentions.extractMentionPersonas,
      mentions.registerMentionPubkey,
    ],
  );

  const createChangedModelAgentMentions = React.useCallback(
    async (modelTargets: readonly PendingAgentMentionModelTarget[] = []) => {
      if (!channelId || modelTargets.length === 0) {
        return {
          errors: [] as string[],
          pubkeys: [] as string[],
        };
      }

      const errors: string[] = [];
      const pubkeys: string[] = [];

      for (const target of modelTargets) {
        if (!target.existingAgent || !didModelSelectionChange(target)) {
          continue;
        }

        // Persona mentions are handled by createMentionedPersonaAgents so they
        // can resolve runtime defaults from the persona catalog.
        if (target.key.startsWith("persona:")) {
          continue;
        }

        const selectedModel = normalizeModelId(target.selectedModel);
        const persona = target.persona;
        const existingAgent = target.existingAgent;

        try {
          const result = await createPersonaAgentMutation.mutateAsync({
            runtime: runtimeFromManagedAgent(existingAgent),
            name: persona?.displayName ?? existingAgent.name,
            personaId: persona?.id ?? existingAgent.personaId ?? undefined,
            systemPrompt:
              persona?.systemPrompt ?? existingAgent.systemPrompt ?? undefined,
            avatarUrl: persona?.avatarUrl ?? undefined,
            model: selectedModel ?? undefined,
            role: "bot",
            ensureRunning: true,
            backend: existingAgent.backend,
            respondTo: existingAgent.respondTo,
            respondToAllowlist:
              existingAgent.respondTo === "allowlist"
                ? existingAgent.respondToAllowlist
                : undefined,
            forceNewInstance: true,
          });
          const pubkey = normalizePubkey(result.agent.pubkey);
          pubkeys.push(pubkey);
          mentions.registerMentionPubkey(target.displayName, pubkey, {
            isAgent: true,
          });
        } catch (error) {
          errors.push(
            `${target.displayName}: ${getErrorMessage(
              error,
              "Could not create agent.",
            )}`,
          );
        }
      }

      return {
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
      };
    },
    [channelId, createPersonaAgentMutation, mentions.registerMentionPubkey],
  );

  const clearComposer = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setAgentModelTargets([]);
    agentModelSelectionsRef.current.clear();
    setNonMemberPromptError(null);
    setAgentModelPromptError(null);
    setContent("");
    contentRef.current = "";
    richText.clearContent();
    setPendingImeta([]);
    mentions.clearMentions();
    channelLinks.clearChannels();
    emojiAutocomplete.clearEmojis();
    setIsEmojiPickerOpen(false);
  }, [
    channelLinks.clearChannels,
    contentRef,
    emojiAutocomplete.clearEmojis,
    mentions.clearMentions,
    richText.clearContent,
    setContent,
    setIsEmojiPickerOpen,
    setPendingImeta,
  ]);

  React.useEffect(() => {
    if (previousChannelIdRef.current === channelId) {
      return;
    }

    previousChannelIdRef.current = channelId;
    setPendingNonMemberSend(null);
    setAgentModelTargets([]);
    agentModelSelectionsRef.current.clear();
    setNonMemberPromptError(null);
    setAgentModelPromptError(null);
  }, [channelId]);

  React.useEffect(() => {
    const requestId = agentModelTargetsRequestRef.current + 1;
    agentModelTargetsRequestRef.current = requestId;
    const trimmed = content.trim();

    if (!trimmed) {
      setAgentModelTargets([]);
      setIsLoadingAgentModelTargets(false);
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setIsLoadingAgentModelTargets(true);
      void collectAgentModelTargets(trimmed)
        .then((targets) => {
          if (agentModelTargetsRequestRef.current !== requestId) {
            return;
          }

          setAgentModelTargets(targets);
          setAgentModelPromptError(null);
        })
        .catch((error) => {
          if (agentModelTargetsRequestRef.current !== requestId) {
            return;
          }

          setAgentModelTargets([]);
          setAgentModelPromptError(
            error instanceof Error
              ? error.message
              : "Could not load agent models.",
          );
        })
        .finally(() => {
          if (agentModelTargetsRequestRef.current === requestId) {
            setIsLoadingAgentModelTargets(false);
          }
        });
    }, 120);

    return () => window.clearTimeout(timeoutId);
  }, [collectAgentModelTargets, content]);

  const completeSend = React.useCallback(
    async (
      draft: PendingNonMemberMentionSend,
      mentionPubkeys: string[],
      outgoingTags = draft.outgoingTags,
    ) => {
      if (isCompleteSendPendingRef.current) {
        return;
      }

      isCompleteSendPendingRef.current = true;
      setIsCompleteSendPending(true);
      try {
        const readyAgentPubkeys = new Set(
          (draft.readyAgentPubkeys ?? []).map(normalizePubkey),
        );
        const agentReadinessErrors = await ensureManagedAgentMentionsReady(
          mentionPubkeys.filter(
            (pubkey) => !readyAgentPubkeys.has(normalizePubkey(pubkey)),
          ),
        );
        if (agentReadinessErrors.length > 0) {
          const message =
            agentReadinessErrors.length === 1
              ? `Could not start agent mention: ${agentReadinessErrors[0]}`
              : `Could not start agent mentions: ${agentReadinessErrors.join(
                  "; ",
                )}`;
          setNonMemberPromptError(message);
          toast.error(message);
          return;
        }

        clearComposer();

        try {
          await onSendRef.current(
            draft.finalContent,
            mentionPubkeys,
            outgoingTags,
          );
          if (draft.sentDraftKey) {
            drafts.clearDraft(draft.sentDraftKey);
          }
        } catch {
          setContent(draft.savedContent);
          contentRef.current = draft.savedContent;
          richText.setContent(draft.savedContent);
          setPendingImeta(draft.savedImeta);
        }
      } finally {
        isCompleteSendPendingRef.current = false;
        setIsCompleteSendPending(false);
      }
    },
    [
      clearComposer,
      contentRef,
      drafts,
      ensureManagedAgentMentionsReady,
      onSendRef,
      richText.setContent,
      setContent,
      setPendingImeta,
    ],
  );

  const getNonMemberMentionPubkeys = React.useCallback(
    (pubkeys: string[]) => {
      if (
        channelType === null ||
        channelType === "dm" ||
        !mentions.hasResolvedMembers
      ) {
        return [];
      }

      return uniqueNormalizedPubkeys(pubkeys).filter(
        (pubkey) => !mentions.memberPubkeys.has(pubkey),
      );
    },
    [channelType, mentions.hasResolvedMembers, mentions.memberPubkeys],
  );

  const continueSendWithAgentModels = React.useCallback(
    async (
      { pendingImeta, sentDraftKey, trimmed }: SendMessageWithMentionFlowInput,
      modelTargets: readonly PendingAgentMentionModelTarget[] = [],
    ) => {
      const personaMentionResult = await createMentionedPersonaAgents(
        trimmed,
        modelTargets,
      );
      const changedModelMentionResult =
        await createChangedModelAgentMentions(modelTargets);
      const agentMentionErrors = [
        ...personaMentionResult.errors,
        ...changedModelMentionResult.errors,
      ];
      if (agentMentionErrors.length > 0) {
        const message =
          agentMentionErrors.length === 1
            ? `Could not create agent mention: ${agentMentionErrors[0]}`
            : `Could not create agent mentions: ${agentMentionErrors.join(
                "; ",
              )}`;
        if (modelTargets.length > 0) {
          setAgentModelPromptError(message);
        } else {
          setNonMemberPromptError(message);
        }
        toast.error(message);
        return false;
      }

      const readyAgentPubkeys = uniqueNormalizedPubkeys([
        ...personaMentionResult.pubkeys,
        ...changedModelMentionResult.pubkeys,
      ]);
      const readyAgentPubkeySet = new Set(
        readyAgentPubkeys.map(normalizePubkey),
      );
      const pubkeys = uniqueNormalizedPubkeys([
        ...mentions.extractMentionPubkeys(trimmed),
        ...readyAgentPubkeys,
      ]);
      const { content: finalContent, mediaTags } = buildOutgoingMessage(
        trimmed,
        pendingImeta,
      );
      const outgoingTags = mergeOutgoingTags(
        mediaTags,
        buildCustomEmojiTags(finalContent, customEmoji),
      );
      const nonMemberPubkeys = getNonMemberMentionPubkeys(pubkeys);
      let promptNonMemberPubkeys = nonMemberPubkeys.filter(
        (pubkey) =>
          !mentions.isManagedAgentPubkey(pubkey) &&
          !readyAgentPubkeySet.has(normalizePubkey(pubkey)),
      );

      if (promptNonMemberPubkeys.length > 0) {
        try {
          const managedAgentsByPubkey = await getManagedAgentsByPubkey();
          promptNonMemberPubkeys = promptNonMemberPubkeys.filter(
            (pubkey) => !managedAgentsByPubkey.has(normalizePubkey(pubkey)),
          );
        } catch {
          // Keep the hook-based managed-agent filtering even if the query
          // fallback misses; ordinary non-members still get prompted.
        }
      }

      const pendingDraft: PendingNonMemberMentionSend = {
        finalContent,
        mentionPubkeys: pubkeys,
        nonMemberPubkeys: promptNonMemberPubkeys,
        outgoingTags,
        readyAgentPubkeys,
        savedContent: trimmed,
        savedImeta: [...pendingImeta],
        sentDraftKey,
      };

      if (promptNonMemberPubkeys.length > 0) {
        setNonMemberPromptError(null);
        setPendingNonMemberSend(pendingDraft);
        return true;
      }

      await completeSend(pendingDraft, pubkeys);
      return true;
    },
    [
      completeSend,
      createChangedModelAgentMentions,
      createMentionedPersonaAgents,
      customEmoji,
      getManagedAgentsByPubkey,
      getNonMemberMentionPubkeys,
      mentions.extractMentionPubkeys,
      mentions.isManagedAgentPubkey,
    ],
  );

  const sendMessageWithMentionFlow = React.useCallback(
    async (input: SendMessageWithMentionFlowInput) => {
      if (isMentionSendPendingRef.current) {
        return;
      }

      isMentionSendPendingRef.current = true;
      setIsMentionSendPending(true);
      try {
        const modelTargets = await collectAgentModelTargets(input.trimmed);
        setAgentModelPromptError(null);
        await continueSendWithAgentModels(input, modelTargets);
      } finally {
        isMentionSendPendingRef.current = false;
        setIsMentionSendPending(false);
      }
    },
    [collectAgentModelTargets, continueSendWithAgentModels],
  );

  const pendingNonMemberNames = React.useMemo(() => {
    if (!pendingNonMemberSend) return [];

    return pendingNonMemberSend.nonMemberPubkeys.map(
      (pubkey) => mentions.getMentionDisplayName(pubkey) ?? pubkey.slice(0, 8),
    );
  }, [mentions.getMentionDisplayName, pendingNonMemberSend]);

  const handleAgentModelChange = React.useCallback(
    (key: string, model: string | null) => {
      const selectedModel = normalizeModelId(model);
      agentModelSelectionsRef.current.set(key, selectedModel);

      setAgentModelTargets((current) => {
        return current.map((target) => {
          if (target.key !== key) {
            return target;
          }
          const nextTarget = {
            ...target,
            selectedModel,
            showModelInTrigger: true,
          };
          return {
            ...nextTarget,
            willCreateNewInstance: didModelSelectionChange(nextTarget),
          };
        });
      });
      setAgentModelPromptError(null);
    },
    [],
  );

  const handleSendWithoutInviting = React.useCallback(() => {
    if (!pendingNonMemberSend) return;

    const nonMemberPubkeys = new Set(
      pendingNonMemberSend.nonMemberPubkeys.map((pubkey) =>
        normalizePubkey(pubkey),
      ),
    );
    const mentionPubkeys = pendingNonMemberSend.mentionPubkeys.filter(
      (pubkey) => !nonMemberPubkeys.has(normalizePubkey(pubkey)),
    );
    const outgoingTags = mergeOutgoingTagsWithReferenceMentions(
      pendingNonMemberSend.outgoingTags,
      nonMemberPubkeys,
    );
    void completeSend(pendingNonMemberSend, mentionPubkeys, outgoingTags);
  }, [completeSend, pendingNonMemberSend]);

  const handleInviteNonMembers = React.useCallback(() => {
    if (!pendingNonMemberSend) return;

    const invitedPubkeys = new Set(
      pendingNonMemberSend.nonMemberPubkeys.map(normalizePubkey),
    );
    const mentionPubkeys = uniqueNormalizedPubkeys([
      ...pendingNonMemberSend.mentionPubkeys,
      ...pendingNonMemberSend.nonMemberPubkeys,
    ]);
    const outgoingTags = (pendingNonMemberSend.outgoingTags ?? []).filter(
      (tag) =>
        tag[0] !== MENTION_REFERENCE_TAG ||
        !invitedPubkeys.has(normalizePubkey(tag[1] ?? "")),
    );

    setNonMemberPromptError(null);
    void (async () => {
      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      const peoplePubkeys: string[] = [];
      const relayAgentPubkeys: string[] = [];

      for (const pubkey of uniqueNormalizedPubkeys(
        pendingNonMemberSend.nonMemberPubkeys,
      )) {
        if (managedAgentsByPubkey.has(pubkey)) {
          continue;
        }

        if (mentions.isAgentPubkey(pubkey)) {
          relayAgentPubkeys.push(pubkey);
        } else {
          peoplePubkeys.push(pubkey);
        }
      }

      const errors: string[] = [];
      if (peoplePubkeys.length > 0) {
        const result = await addMembersMutation.mutateAsync({
          pubkeys: peoplePubkeys,
          role: "member",
        });
        errors.push(...result.errors.map((error) => error.error));
      }

      if (relayAgentPubkeys.length > 0) {
        const result = await addMembersMutation.mutateAsync({
          pubkeys: relayAgentPubkeys,
          role: "bot",
        });
        errors.push(...result.errors.map((error) => error.error));
      }

      if (errors.length > 0) {
        setNonMemberPromptError(errors.join("; "));
        return;
      }

      await completeSend(
        {
          ...pendingNonMemberSend,
          mentionPubkeys,
          outgoingTags,
        },
        mentionPubkeys,
        outgoingTags,
      );
    })().catch((error) => {
      setNonMemberPromptError(
        error instanceof Error ? error.message : "Could not invite members.",
      );
    });
  }, [
    addMembersMutation,
    completeSend,
    getManagedAgentsByPubkey,
    mentions.isAgentPubkey,
    pendingNonMemberSend,
  ]);

  const dismissNonMemberPrompt = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
  }, []);

  return {
    agentModelPromptError,
    dismissNonMemberPrompt,
    agentModelPersonas: activeAgentPersonas,
    agentModelTargets,
    isLoadingAgentModelPersonas: personasQuery.isLoading,
    isLoadingAgentModelTargets,
    isInvitePending:
      isMentionSendPending ||
      isCompleteSendPending ||
      addMembersMutation.isPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending ||
      startAgentMutation.isPending,
    isPreparingMentionSend:
      isMentionSendPending ||
      isCompleteSendPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending ||
      startAgentMutation.isPending,
    onAgentModelChange: handleAgentModelChange,
    nonMemberPromptError,
    pendingNonMemberNames,
    pendingNonMemberSend,
    sendMessageWithMentionFlow,
    sendWithoutInviting: handleSendWithoutInviting,
    inviteNonMembers: handleInviteNonMembers,
  };
}
