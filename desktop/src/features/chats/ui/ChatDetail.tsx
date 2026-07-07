import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { MessageCircle } from "lucide-react";
import { toast } from "sonner";

import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { scopeByChannel } from "@/features/agents/ui/agentSessionPanelLayout";
import {
  useAgentChatTitle,
  useAgentsTranscript,
} from "@/features/agents/ui/useObserverEvents";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import {
  useSendChatContextMessageMutation,
  useUpdateChatMetadataMutation,
} from "@/features/chats/hooks";
import {
  buildChatActivityPlacement,
  shouldHidePersistedAgentMessage,
} from "@/features/chats/lib/chatActivity";
import { chatProjectForMetadata } from "@/features/chats/lib/chatProjects";
import {
  buildChatCanvasContent,
  deriveBranchTitle,
  buildProjectSetupContext,
  type ChatProject,
  deriveChatTitle,
  deriveConversationTitle,
  NO_PROJECT_SELECTION_ID,
} from "@/features/chats/lib/chatSetup";
import { ChatActivityTranscript } from "@/features/chats/ui/ChatActivityTranscript";
import {
  CHAT_AUTOMATION_TAG,
  chatAutomationTag,
  clearChatPinnedPr,
} from "@/features/chats/lib/chatWorkAutomation";
import {
  buildChatWorkContext,
  markChatWorkContextSent,
  mergeChatWorkBinding,
  shouldShowChatWorkContextContent,
  updateChatWorkBinding,
  useChatWorkBinding,
  wasChatWorkContextSent,
} from "@/features/chats/lib/chatWorkBinding";
import {
  collectBranchSourcesFromAgentMessages,
  collectChatWorkBranchSources,
  deriveBranchSourceFromAgentMessages,
  deriveChatWorkBranchSource,
} from "@/features/chats/lib/chatWorkBranch";
import { collectUnambiguousPullRequestSources } from "@/features/chats/lib/chatWorkLinks";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import { ChatWorkPanel } from "@/features/chats/ui/ChatWorkPanel";
import { isHumanFacingAssistantText } from "@/features/chats/ui/chatActivityText";
import { entranceClassForCreatedAt } from "@/features/chats/ui/messageEntrance";
import {
  AgentActivationCard,
  ChatAutomationRow,
  ChatContextRow,
  ChatMessageRow,
  ChatScrollAnchor,
} from "@/features/chats/ui/ChatConversationRows";
import { ProjectPicker } from "@/features/chats/ui/QuickStartChat";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { setCanvas } from "@/shared/api/tauri";
import type {
  Channel,
  ChannelTemplate,
  ChatMetadata,
  ManagedAgent,
  RelayEvent,
} from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/shared/ui/message-scroller";
import { Spinner } from "@/shared/ui/spinner";

import type { UserProfileLookup } from "@/features/profile/lib/identity";

const CHAT_CONVERSATION_CLASS = "mx-auto w-full max-w-4xl px-4 sm:px-6 lg:px-8";

function eventHasTag(event: RelayEvent, name: string, value?: string) {
  return event.tags.some(
    (tag) => tag[0] === name && (value === undefined || tag[1] === value),
  );
}

function chatContextTimelineRank(event: RelayEvent) {
  if (!eventHasTag(event, "chat_context", "source")) {
    return 0;
  }
  if (event.content.startsWith("Project setup")) {
    return -1;
  }
  return 1;
}

function workContextLine(content: string, label: "Branch" | "Pull request") {
  const match = content.match(new RegExp(`^${label}:\\s*(.+)$`, "m"));
  return match?.[1]?.trim() || null;
}

function chatContextVisualTimestampMs({
  branchAnchors,
  event,
  prAnchors,
}: {
  branchAnchors: ReadonlyMap<string, number>;
  event: RelayEvent;
  prAnchors: ReadonlyMap<string, number>;
}) {
  if (!event.content.startsWith("Work context")) {
    return event.created_at * 1_000;
  }

  const branch = workContextLine(event.content, "Branch");
  if (branch) {
    const branchAnchor = branchAnchors.get(branch);
    if (branchAnchor !== undefined) {
      return branchAnchor;
    }
  }

  const prHref = workContextLine(event.content, "Pull request");
  if (prHref) {
    const prAnchor = prAnchors.get(prHref);
    if (prAnchor !== undefined) {
      return prAnchor;
    }
  }

  return event.created_at * 1_000;
}

type ChatDetailProps = {
  chat: Channel;
  defaultAgent: ManagedAgent | null;
  identityPubkey?: string;
  isLoadingMessages: boolean;
  isActivatingAgent: boolean;
  isSending: boolean;
  messages: RelayEvent[];
  metadata: ChatMetadata | null;
  onActivateAgent: () => void;
  onProjectCreated: (project: ChatProject) => void;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  projects: ChatProject[];
  shareAction?: React.ReactNode;
  /** Show the top-right work module for this PR (toggled from the header). */
  showWorkPanel?: boolean;
  templates: ChannelTemplate[];
  /** Latest PR link the chat's agent posted, if any. */
  workPanelHref?: string | null;
};

export function ChatDetail({
  chat,
  defaultAgent,
  identityPubkey,
  isActivatingAgent,
  isLoadingMessages,
  isSending,
  messages,
  metadata,
  onActivateAgent,
  onProjectCreated,
  onSend,
  profiles,
  projects,
  shareAction,
  showWorkPanel = true,
  templates,
  workPanelHref = null,
}: ChatDetailProps) {
  const queryClient = useQueryClient();
  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const sendContextMutation = useSendChatContextMessageMutation();
  // Every active managed agent, not just the default: a chat can have
  // several agents working and all of their activity must render.
  const managedAgentsQuery = useManagedAgentsQuery();
  // Key-stabilized: the managed-agents query refetches on a 30s interval and
  // a fresh array identity would resubscribe the transcript store each time
  // even when the active set is unchanged.
  const activeAgentPubkeysKey = React.useMemo(() => {
    const pubkeys = (managedAgentsQuery.data ?? [])
      .filter(isManagedAgentActive)
      .map((agent) => normalizePubkey(agent.pubkey));
    if (defaultAgent && isManagedAgentActive(defaultAgent)) {
      pubkeys.push(normalizePubkey(defaultAgent.pubkey));
    }
    return [...new Set(pubkeys)].sort().join(",");
  }, [defaultAgent, managedAgentsQuery.data]);
  const activeAgentPubkeys = React.useMemo(
    () => activeAgentPubkeysKey.split(",").filter(Boolean),
    [activeAgentPubkeysKey],
  );
  const hasObserver = activeAgentPubkeys.length > 0;
  const activeChannelTurns = useActiveAgentTurnsByChannel();
  // Per-turn ids, not a channel-wide boolean: while a new turn runs, older
  // turn blocks must still render as completed (and never show their own
  // "Working" marker).
  const activeTurnIds = React.useMemo(
    () =>
      new Set(
        activeChannelTurns
          .filter((turn) => turn.channelId === chat.id)
          .flatMap((turn) => turn.turnIds),
      ),
    [activeChannelTurns, chat.id],
  );
  const isChatTurnActive = activeTurnIds.size > 0;
  const transcript = useAgentsTranscript(hasObserver, activeAgentPubkeys);
  const scopedTranscript = React.useMemo(
    () => scopeByChannel(transcript, chat.id),
    [chat.id, transcript],
  );
  const chatActivity = React.useMemo(
    () =>
      buildChatActivityPlacement({
        agentPubkey: defaultAgent?.pubkey,
        messages,
        transcript: scopedTranscript,
      }),
    [defaultAgent?.pubkey, messages, scopedTranscript],
  );
  // Branch the agent is on, straight from its worktree/checkout commands —
  // the work panel shows it live, before any PR exists to report head.ref.
  // Tool activity only exists from subscription time (observer frames are
  // ephemeral), so fall back to the agent's persisted messages, which
  // announce the branch ("…on branch kennylopez-dictation").
  const transcriptWorkBranchSource = React.useMemo(
    () => deriveChatWorkBranchSource(scopedTranscript),
    [scopedTranscript],
  );
  const messageWorkBranchSource = React.useMemo(
    () => deriveBranchSourceFromAgentMessages(messages, defaultAgent?.pubkey),
    [defaultAgent?.pubkey, messages],
  );
  const workBranch =
    (transcriptWorkBranchSource ?? messageWorkBranchSource)?.branch ?? null;
  const branchAnchors = React.useMemo(() => {
    const anchors = new Map<string, number>();
    const rememberEarliest = (branch: string, timestampMs: number | null) => {
      if (timestampMs === null) {
        return;
      }
      const current = anchors.get(branch);
      if (current === undefined || timestampMs < current) {
        anchors.set(branch, timestampMs);
      }
    };
    for (const source of collectBranchSourcesFromAgentMessages(
      messages,
      defaultAgent?.pubkey,
    )) {
      rememberEarliest(source.branch, source.timestampMs);
    }
    for (const source of collectChatWorkBranchSources(scopedTranscript)) {
      rememberEarliest(source.branch, source.timestampMs);
    }
    return anchors;
  }, [defaultAgent?.pubkey, messages, scopedTranscript]);
  const prAnchors = React.useMemo(() => {
    const anchors = new Map<string, number>();
    for (const source of collectUnambiguousPullRequestSources(messages)) {
      if (source.timestampMs === null) {
        continue;
      }
      const current = anchors.get(source.href);
      if (current === undefined || source.timestampMs < current) {
        anchors.set(source.href, source.timestampMs);
      }
    }
    return anchors;
  }, [messages]);
  const handleStopAgent = React.useCallback(() => {
    // Cancel every agent with a live turn in this chat; fall back to the
    // default agent when the turn store hasn't caught up yet.
    const workingPubkeys =
      activeChannelTurns.find((turn) => turn.channelId === chat.id)
        ?.agentPubkeys ?? [];
    const targets =
      workingPubkeys.length > 0
        ? workingPubkeys
        : defaultAgent?.pubkey
          ? [defaultAgent.pubkey]
          : [];
    for (const pubkey of targets) {
      cancelManagedAgentTurn(pubkey, chat.id).catch((error: unknown) => {
        console.error("Failed to stop agent turn", error);
        toast.error("Could not stop the agent");
      });
    }
  }, [activeChannelTurns, chat.id, defaultAgent?.pubkey]);
  const selectedProject = React.useMemo(
    () => chatProjectForMetadata(metadata),
    [metadata],
  );
  const workBinding = useChatWorkBinding(chat.id);
  const projectName =
    metadata?.projectName?.trim() || selectedProject?.name?.trim() || null;
  const projectPath =
    metadata?.projectPath?.trim() || selectedProject?.path?.trim() || null;

  React.useEffect(() => {
    if (metadata === null) {
      return;
    }
    const hadProject = Boolean(
      workBinding?.projectName || workBinding?.projectPath,
    );
    const projectChanged = Boolean(
      workBinding &&
        hadProject &&
        ((workBinding.projectName ?? "") !== (projectName ?? "") ||
          (workBinding.projectPath ?? "") !== (projectPath ?? "")),
    );
    if (projectChanged) {
      clearChatPinnedPr(chat.id);
      updateChatWorkBinding(
        chat.id,
        {
          projectName,
          projectPath,
          branch: null,
          prHref: null,
          prDetached: false,
        },
        { replaceProject: true, replaceBranch: true, replacePr: true },
      );
      return;
    }
    updateChatWorkBinding(
      chat.id,
      { projectName, projectPath },
      { replaceProject: true },
    );
  }, [chat.id, metadata, projectName, projectPath, workBinding]);

  React.useEffect(() => {
    if (!workBranch) {
      return;
    }
    updateChatWorkBinding(chat.id, { branch: workBranch });
  }, [chat.id, workBranch]);

  React.useEffect(() => {
    if (!workPanelHref) {
      return;
    }
    updateChatWorkBinding(chat.id, {
      prHref: workPanelHref,
      prDetached: false,
    });
  }, [chat.id, workPanelHref]);

  const boundBranch = workBinding?.branch ?? workBranch;
  const boundPrHref = workBinding?.prDetached
    ? null
    : (workBinding?.prHref ?? workPanelHref);
  const effectiveWorkBinding = React.useMemo(
    () =>
      mergeChatWorkBinding(
        null,
        {
          projectName,
          projectPath,
          branch: boundBranch,
          prHref: boundPrHref,
          prDetached: workBinding?.prDetached ?? false,
        },
        { replaceProject: true, replaceBranch: true, replacePr: true },
      ),
    [
      boundBranch,
      boundPrHref,
      projectName,
      projectPath,
      workBinding?.prDetached,
    ],
  );
  const workContext = React.useMemo(
    () => buildChatWorkContext(effectiveWorkBinding),
    [effectiveWorkBinding],
  );
  const sendChatContextAsync = sendContextMutation.mutateAsync;
  React.useEffect(() => {
    if (!workContext || wasChatWorkContextSent(chat.id, workContext)) {
      return;
    }
    let cancelled = false;
    void sendChatContextAsync({
      channelId: chat.id,
      content: workContext,
    })
      .then(() => {
        if (!cancelled) {
          markChatWorkContextSent(chat.id, workContext);
        }
      })
      .catch((error) => {
        console.warn("Failed to send chat work context", chat.id, error);
      });
    return () => {
      cancelled = true;
    };
  }, [chat.id, sendChatContextAsync, workContext]);
  const handleSelectProject = React.useCallback(
    async (projectId: string | null) => {
      const nextProject =
        projectId && projectId !== NO_PROJECT_SELECTION_ID
          ? (projects.find((project) => project.id === projectId) ?? null)
          : null;
      const nextTemplate = nextProject?.templateId
        ? (templates.find(
            (template) => template.id === nextProject.templateId,
          ) ?? null)
        : null;
      const title = metadata?.title?.trim() || chat.name;

      try {
        await updateMetadataMutation.mutateAsync({
          channelId: chat.id,
          defaultAgentPubkey:
            metadata?.defaultAgentPubkey ?? defaultAgent?.pubkey ?? undefined,
          projectId: nextProject?.id,
          projectName: nextProject?.name,
          projectPath: nextProject?.path ?? undefined,
          projectTemplateId: nextProject?.templateId ?? undefined,
          source: metadata?.sourceChannelId
            ? {
                channelId: metadata.sourceChannelId,
                eventId: metadata.sourceEventId ?? undefined,
                threadRootId: metadata.sourceThreadRootId ?? undefined,
              }
            : undefined,
          templateId: nextProject?.templateId ?? undefined,
          title,
        });

        const leadingContent = buildProjectSetupContext({
          agent: defaultAgent,
          project: nextProject,
          templateName: nextTemplate?.name ?? null,
        });
        const canvasContent = buildChatCanvasContent({
          channelName: title,
          leadingContent,
          template: nextTemplate,
        });
        await setCanvas({
          channelId: chat.id,
          content: canvasContent ?? "",
        });
        await queryClient.invalidateQueries({
          queryKey: ["channel-canvas", chat.id],
        });

        if (leadingContent) {
          await sendChatContextAsync({
            channelId: chat.id,
            content: leadingContent,
          });
        }

        if (nextProject) {
          onProjectCreated({
            ...nextProject,
            updatedAt: Math.floor(Date.now() / 1_000),
          });
        }
        toast.success(
          nextProject
            ? `Project set to ${nextProject.name}`
            : "Project removed",
        );
      } catch (error) {
        console.error("Failed to update chat project", error);
        toast.error("Could not update project", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [
      chat.id,
      chat.name,
      defaultAgent,
      metadata,
      onProjectCreated,
      projects,
      queryClient,
      sendChatContextAsync,
      templates,
      updateMetadataMutation,
    ],
  );
  const visibleMessages = React.useMemo(
    () =>
      messages
        .filter((message) => {
          // Only true message kinds render: the channel query also delivers
          // reactions/edits/deletions (kind 7 et al.), and a stray agent 👀
          // reaction otherwise renders as a tiny emoji bubble.
          if (
            !CHANNEL_MESSAGE_EVENT_KINDS.includes(
              message.kind as (typeof CHANNEL_MESSAGE_EVENT_KINDS)[number],
            )
          ) {
            return false;
          }
          // NOTE: no narration heuristics here. A persisted agent message was
          // deliberately SENT to the channel — filtering it through the
          // transcript's internal-narration patterns dropped real replies
          // ("Done! I've sent the summary…"), leaving turns that visibly
          // worked but never answered. Narration shaping belongs to the
          // activity transcript; persisted rows only dedup against it.
          return (
            (eventHasTag(message, "chat_context", "source") ||
              message.content.trim().length > 0) &&
            shouldShowChatWorkContextContent(message.content, workContext) &&
            !shouldHidePersistedAgentMessage({
              event: message,
              hiddenAgentMessageIds: chatActivity.hiddenAgentMessageIds,
            })
          );
        })
        .sort((left, right) => {
          const leftTimestampMs = chatContextVisualTimestampMs({
            branchAnchors,
            event: left,
            prAnchors,
          });
          const rightTimestampMs = chatContextVisualTimestampMs({
            branchAnchors,
            event: right,
            prAnchors,
          });
          if (leftTimestampMs !== rightTimestampMs) {
            return leftTimestampMs - rightTimestampMs;
          }
          return chatContextTimelineRank(left) - chatContextTimelineRank(right);
        }),
    [
      branchAnchors,
      chatActivity.hiddenAgentMessageIds,
      messages,
      prAnchors,
      workContext,
    ],
  );
  const hasTranscriptActivity = chatActivity.totalBlockCount > 0;

  // Solo chats (you + one agent) read as a plain stream: agent rows drop
  // their avatar and name. Identities come back as soon as another agent or
  // person participates, so multi-party chats stay attributable.
  const showAgentIdentity = React.useMemo(() => {
    const others = new Set<string>();
    for (const message of messages) {
      // Same message-kind allowlist as the timeline: a reaction event alone
      // (an old harness's 👀) must not flip the solo layout to multi-party.
      if (
        !CHANNEL_MESSAGE_EVENT_KINDS.includes(
          message.kind as (typeof CHANNEL_MESSAGE_EVENT_KINDS)[number],
        )
      ) {
        continue;
      }
      const pubkey = normalizePubkey(message.pubkey);
      if (identityPubkey && pubkey === normalizePubkey(identityPubkey)) {
        continue;
      }
      others.add(pubkey);
    }
    if (defaultAgent?.pubkey) {
      others.add(normalizePubkey(defaultAgent.pubkey));
    }
    return others.size > 1;
  }, [defaultAgent?.pubkey, identityPubkey, messages]);

  // Auto-title: upgrade a still-default title (the first message, verbatim)
  // to a succinct subject line. A branch name is the strongest signal for
  // work chats; otherwise prefer the agent-generated `chat_title` observer
  // frame and fall back to the local heuristic once the conversation develops.
  // Never touches a manually renamed chat, and skips shared chats we don't
  // own.
  const agentChatTitle = useAgentChatTitle(chat.id);
  const heuristicRetitledChatIdsRef = React.useRef(new Set<string>());
  const appliedTitleKeysRef = React.useRef(new Set<string>());
  const updateChatMetadataAsync = updateMetadataMutation.mutateAsync;
  React.useEffect(() => {
    if (!metadata?.title) {
      return;
    }
    if (
      metadata.authorPubkey &&
      identityPubkey &&
      normalizePubkey(metadata.authorPubkey) !== normalizePubkey(identityPubkey)
    ) {
      return;
    }

    const firstOwnMessage = messages.find(
      (message) =>
        identityPubkey != null &&
        normalizePubkey(message.pubkey) === normalizePubkey(identityPubkey) &&
        !eventHasTag(message, "chat_context", "source") &&
        message.content.trim().length > 0,
    );
    if (!firstOwnMessage) {
      return;
    }

    const branchTitle = deriveBranchTitle(boundBranch);
    const branchAutoTitles = new Set<string>();
    if (branchTitle) {
      branchAutoTitles.add(branchTitle);
    }
    for (const branch of branchAnchors.keys()) {
      const candidate = deriveBranchTitle(branch);
      if (candidate) {
        branchAutoTitles.add(candidate);
      }
    }

    // Auto titles are the ones this flow (or chat creation) produced; any
    // other value is a manual rename we must never override.
    const isAutoTitle =
      metadata.title === "New chat" ||
      metadata.title === deriveChatTitle(firstOwnMessage.content) ||
      metadata.title === deriveConversationTitle(firstOwnMessage.content) ||
      branchAutoTitles.has(metadata.title);
    if (!isAutoTitle) {
      return;
    }

    let nextTitle: string | null = null;
    let isHeuristicTitle = false;
    if (branchTitle) {
      nextTitle = branchTitle;
    } else if (agentChatTitle && agentChatTitle.trim().length > 0) {
      nextTitle = agentChatTitle.trim();
    } else if (!heuristicRetitledChatIdsRef.current.has(chat.id)) {
      // Heuristic fallback. Retitling right after the first reply feels
      // premature — wait until the conversation has developed: either a
      // second exchange from the user, or the agent replying twice.
      const agentReplyCount = messages.filter(
        (message) =>
          defaultAgent?.pubkey != null &&
          normalizePubkey(message.pubkey) ===
            normalizePubkey(defaultAgent.pubkey) &&
          isHumanFacingAssistantText(message.content),
      ).length;
      const ownMessageCount = messages.filter(
        (message) =>
          identityPubkey != null &&
          normalizePubkey(message.pubkey) === normalizePubkey(identityPubkey) &&
          !eventHasTag(message, "chat_context", "source") &&
          message.content.trim().length > 0,
      ).length;
      const conversationHasDeveloped =
        agentReplyCount >= 2 || (agentReplyCount >= 1 && ownMessageCount >= 2);
      if (conversationHasDeveloped) {
        nextTitle = deriveConversationTitle(firstOwnMessage.content);
        isHeuristicTitle = true;
      }
    }

    if (!nextTitle || nextTitle === metadata.title) {
      return;
    }
    const applyKey = `${chat.id}:${nextTitle}`;
    if (appliedTitleKeysRef.current.has(applyKey)) {
      return;
    }
    appliedTitleKeysRef.current.add(applyKey);
    if (isHeuristicTitle) {
      heuristicRetitledChatIdsRef.current.add(chat.id);
    }

    updateChatMetadataAsync({
      channelId: chat.id,
      title: nextTitle,
      defaultAgentPubkey:
        metadata.defaultAgentPubkey ?? defaultAgent?.pubkey ?? undefined,
      templateId: metadata.templateId ?? undefined,
      projectId: metadata.projectId ?? undefined,
      projectName: metadata.projectName ?? undefined,
      projectPath: metadata.projectPath ?? undefined,
      projectTemplateId: metadata.projectTemplateId ?? undefined,
      source: metadata.sourceChannelId
        ? {
            channelId: metadata.sourceChannelId,
            eventId: metadata.sourceEventId ?? undefined,
            threadRootId: metadata.sourceThreadRootId ?? undefined,
          }
        : undefined,
    }).catch((error) => {
      console.warn("Failed to auto-title chat", chat.id, error);
      // Retry on the next qualifying render rather than giving up for good.
      appliedTitleKeysRef.current.delete(applyKey);
      if (isHeuristicTitle) {
        heuristicRetitledChatIdsRef.current.delete(chat.id);
      }
    });
  }, [
    agentChatTitle,
    boundBranch,
    branchAnchors,
    chat.id,
    defaultAgent?.pubkey,
    identityPubkey,
    messages,
    metadata,
    updateChatMetadataAsync,
  ]);
  const latestVisibleMessage =
    visibleMessages.length > 0
      ? visibleMessages[visibleMessages.length - 1]
      : null;
  const latestVisibleMessageIsOwn =
    latestVisibleMessage != null &&
    identityPubkey != null &&
    normalizePubkey(latestVisibleMessage.pubkey) ===
      normalizePubkey(identityPubkey);
  const latestMessageActivityBlocks =
    latestVisibleMessage != null
      ? (chatActivity.blocksByMessageId.get(latestVisibleMessage.id) ?? [])
      : [];
  const latestOwnMessageNeedsAgent =
    latestVisibleMessageIsOwn &&
    latestMessageActivityBlocks.length === 0 &&
    !isChatTurnActive;
  // Keep activation pending until a live turn appears or the wait expires.
  const ACTIVATION_PENDING_MS = 60_000;
  const [isActivationPending, setIsActivationPending] = React.useState(false);
  const handleActivateAgent = React.useCallback(() => {
    setIsActivationPending(true);
    onActivateAgent();
  }, [onActivateAgent]);

  // Automation prompts must not fail silently: surface send errors, and if
  // the default agent is stopped, start it too — the backlog replay delivers
  // the prompt once it connects.
  const handleAutomationPrompt = React.useCallback(
    (content: string, kind: "ci" | "comments") => {
      onSend(content, [], [chatAutomationTag(kind)]).catch((error: unknown) => {
        console.error("Failed to send automation prompt", error);
        toast.error("Could not send the automation instructions");
      });
      if (defaultAgent != null && !isManagedAgentActive(defaultAgent)) {
        setIsActivationPending(true);
        onActivateAgent();
      }
    },
    [defaultAgent, onActivateAgent, onSend],
  );

  const defaultAgentActive =
    defaultAgent != null && isManagedAgentActive(defaultAgent);
  React.useEffect(() => {
    if (!isActivationPending) {
      return;
    }
    if (isChatTurnActive || defaultAgentActive) {
      setIsActivationPending(false);
      return;
    }
    const timeout = window.setTimeout(() => {
      setIsActivationPending(false);
    }, ACTIVATION_PENDING_MS);
    return () => window.clearTimeout(timeout);
  }, [defaultAgentActive, isActivationPending, isChatTurnActive]);
  // A stopped default agent always shows the card.
  const defaultAgentInactive = defaultAgent != null && !defaultAgentActive;
  const shouldShowAgentActivationCard =
    (defaultAgentInactive && latestVisibleMessage != null) ||
    (!defaultAgentActive && latestOwnMessageNeedsAgent && !hasObserver);
  const forceScrollSignature = latestVisibleMessageIsOwn
    ? latestVisibleMessage.id
    : null;

  return (
    <>
      <ChatHeader
        actions={shareAction}
        animatedTitle
        description={defaultAgent?.name ?? "Fizz"}
        // Keyed by chat so switching chats swaps the header instantly; only
        // an in-place retitle of the current chat animates. The prefix keeps
        // this key distinct from the sibling MessageScrollerProvider's
        // key={chat.id} — duplicate sibling keys corrupt reconciliation and
        // leak a header per chat switch.
        key={`header:${chat.id}`}
        mode="chats"
        title={metadata?.title || chat.name}
        transparentChrome
      />

      <div className="flex min-h-0 min-w-0 flex-1">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <MessageScrollerProvider
            autoScroll
            defaultScrollPosition="end"
            key={chat.id}
            scrollEdgeThreshold={48}
          >
            <MessageScroller className="bg-background" topFade>
              <MessageScrollerViewport aria-label="Chat messages">
                <MessageScrollerContent
                  className={cn(CHAT_CONVERSATION_CLASS, "py-6")}
                >
                  {isLoadingMessages ? (
                    <MessageScrollerItem messageId="chat:loading">
                      <div className="flex items-center gap-2 px-5 py-1 text-sm text-muted-foreground">
                        <Spinner className="h-4 w-4" />
                        Loading messages
                      </div>
                    </MessageScrollerItem>
                  ) : visibleMessages.length === 0 && !hasTranscriptActivity ? (
                    <MessageScrollerItem
                      className="flex flex-1 items-center justify-center"
                      messageId="chat:empty"
                    >
                      <div className="px-8 py-12 text-center">
                        <MessageCircle className="mx-auto h-5 w-5 text-muted-foreground" />
                        <p className="mt-3 text-sm font-medium">
                          No messages yet
                        </p>
                        <p className="mt-1 text-sm text-muted-foreground">
                          Send a message and Fizz will respond.
                        </p>
                      </div>
                    </MessageScrollerItem>
                  ) : (
                    <>
                      {visibleMessages.map((message) => {
                        const activityBlocks =
                          chatActivity.blocksByMessageId.get(message.id) ?? [];
                        const isContextMessage = eventHasTag(
                          message,
                          "chat_context",
                          "source",
                        );
                        // Automation prompts stay invisible: the agent's
                        // activity and reply anchor here, but no bubble.
                        const isAutomationMessage = eventHasTag(
                          message,
                          CHAT_AUTOMATION_TAG[0],
                          CHAT_AUTOMATION_TAG[1],
                        );
                        const isAgentMessage =
                          defaultAgent?.pubkey != null &&
                          normalizePubkey(message.pubkey) ===
                            normalizePubkey(defaultAgent.pubkey);
                        const isOwnMessage =
                          identityPubkey != null &&
                          normalizePubkey(message.pubkey) ===
                            normalizePubkey(identityPubkey);

                        return (
                          <React.Fragment key={message.localKey ?? message.id}>
                            <MessageScrollerItem
                              className={entranceClassForCreatedAt(
                                message.created_at,
                              )}
                              messageId={message.id}
                            >
                              {isAutomationMessage ? (
                                <ChatAutomationRow
                                  agentName={defaultAgent?.name ?? "Fizz"}
                                  event={message}
                                />
                              ) : isContextMessage ? (
                                <ChatContextRow event={message} />
                              ) : (
                                <ChatMessageRow
                                  event={message}
                                  isAgent={isAgentMessage}
                                  isOwn={isOwnMessage}
                                  profiles={profiles}
                                  showAgentIdentity={showAgentIdentity}
                                />
                              )}
                            </MessageScrollerItem>
                            {activityBlocks.length > 0 ? (
                              <MessageScrollerItem
                                messageId={`chat:activity:${message.id}`}
                              >
                                <ChatActivityTranscript
                                  agent={defaultAgent}
                                  blocks={activityBlocks}
                                  identityPubkey={identityPubkey}
                                  activeTurnIds={activeTurnIds}
                                  showAgentIdentity={showAgentIdentity}
                                  profiles={profiles}
                                />
                              </MessageScrollerItem>
                            ) : null}
                            {shouldShowAgentActivationCard &&
                            latestVisibleMessage?.id === message.id ? (
                              <MessageScrollerItem
                                messageId={`chat:activate-agent:${message.id}`}
                              >
                                <AgentActivationCard
                                  agentName={defaultAgent?.name ?? "Fizz"}
                                  isActivating={
                                    isActivatingAgent || isActivationPending
                                  }
                                  onActivate={handleActivateAgent}
                                />
                              </MessageScrollerItem>
                            ) : null}
                          </React.Fragment>
                        );
                      })}
                      {chatActivity.unplacedBlocks.length > 0 ? (
                        <MessageScrollerItem messageId="chat:activity:unplaced">
                          <ChatActivityTranscript
                            agent={defaultAgent}
                            blocks={chatActivity.unplacedBlocks}
                            identityPubkey={identityPubkey}
                            activeTurnIds={activeTurnIds}
                            profiles={profiles}
                            showAgentIdentity={showAgentIdentity}
                          />
                        </MessageScrollerItem>
                      ) : null}
                    </>
                  )}
                </MessageScrollerContent>
              </MessageScrollerViewport>
              <MessageScrollerButton />
              <ChatScrollAnchor forceSignature={forceScrollSignature} />
            </MessageScroller>
          </MessageScrollerProvider>

          <div className="shrink-0 bg-background">
            <MessageComposer
              autoInviteNonMemberMentions
              channelId={chat.id}
              channelName={chat.name}
              channelType="chat"
              containerClassName={cn(CHAT_CONVERSATION_CLASS, "pb-3")}
              disabled={isSending}
              draftKey={`chat:${chat.id}`}
              isSending={isSending}
              onSend={onSend}
              onStopAgent={isChatTurnActive ? handleStopAgent : null}
              placeholder="Message Fizz..."
              profiles={profiles}
              toolbarControls={{
                emoji: false,
                formatting: false,
                spoiler: false,
              }}
              toolbarExtraActions={
                <ProjectPicker
                  isNoProjectSelected={!selectedProject && metadata !== null}
                  onCreateProject={onProjectCreated}
                  onSelectProject={handleSelectProject}
                  projects={projects}
                  selectedProject={selectedProject}
                  templates={templates}
                />
              }
            />
          </div>
        </div>
        <ChatWorkPanel
          agentName={defaultAgent?.name ?? "Fizz"}
          branch={boundBranch}
          chatId={chat.id}
          isTurnActive={isChatTurnActive}
          onAutomationPrompt={handleAutomationPrompt}
          open={showWorkPanel}
          prHref={boundPrHref}
          projectPath={metadata?.projectPath ?? selectedProject?.path ?? null}
        />
      </div>
    </>
  );
}
