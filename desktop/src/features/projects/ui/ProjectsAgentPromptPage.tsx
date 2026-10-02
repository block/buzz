import { EditorContent } from "@tiptap/react";
import {
  ALargeSmall,
  ChevronDown,
  Loader2,
  SendHorizontal,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import {
  getMentionableAgentPubkeys,
  getSharedChannelIds,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import { normalizeRelayUrl } from "@/features/communities/communityStorage";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useLinkEditor } from "@/features/messages/lib/useLinkEditor";
import {
  type LinkSelectionInfo,
  useRichTextEditor,
} from "@/features/messages/lib/useRichTextEditor";
import { FormattingToolbar } from "@/features/messages/ui/FormattingToolbar";
import { MessageThreadTranscript } from "@/features/messages/ui/MessageThreadTranscript";
import { ThreadRepliesErrorCard } from "@/features/messages/ui/MessageThreadReplyState";
import type { TimelineMessage } from "@/features/messages/types";
import { useThreadRepliesForRoots } from "@/features/messages/useThreadReplies";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import { pickDefaultProjectsAgent } from "@/features/projects/lib/projectAgentSelection";
import { AgentContextPayloadPreview } from "./AgentContextPayloadPreview";
import {
  PROJECT_WORKSPACE_CONTEXT_MARKER,
  splitProjectDetailAgentContext,
  UNTRUSTED_CONTEXT_NOTICE,
  untrustedPromptValue,
} from "@/features/projects/lib/projectDetailAgentContext";
import {
  isAtOrAfterConversationOpener,
  mergeProjectAgentConversationEvents,
  restoreProjectsAgentConversation,
  submitProjectAgentMessage,
} from "@/features/projects/lib/projectAgentConversation";
import {
  clearStoredProjectsAgentConversation,
  type ProjectsConversationOpener,
  projectsConversationScope,
  readStoredProjectsAgentConversation,
  type StoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import type { Channel } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { ProjectAgentSubmittedContextPill } from "./ProjectAgentSubmittedContextPill";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type AgentCandidate = {
  pubkey: string;
  name: string;
  personaId?: string | null;
  /** Managed agents can be auto-started before the prompt is sent. */
  isManaged: boolean;
  isActive: boolean | null;
};

type ProjectAgentConversation = {
  channel: Channel;
  agent: AgentCandidate;
  opener: ProjectsConversationOpener;
};

const MAX_CONTEXT_REPOS = 8;

/** Compact machine-readable footer so the agent can scope git queries
 * (repo announcements are addressable by these coordinates). Only sent
 * with the first message of a conversation. Project and repository names
 * are relay-controlled — each value is neutralized and quoted, and the
 * block carries the shared untrusted-data framing. */
function repoContextBlock(projects: readonly Project[]) {
  if (projects.length === 0) return "";
  const repositories = projects.flatMap((project) =>
    project.repositories.map((repository) => ({
      label:
        project.repositories.length > 1
          ? `${project.name} / ${repository.name}`
          : project.name,
      repoAddress: repository.repoAddress,
    })),
  );
  const listed = repositories
    .slice(0, MAX_CONTEXT_REPOS)
    .map(
      (repository) =>
        `- ${untrustedPromptValue(repository.label)} (address: ${untrustedPromptValue(repository.repoAddress, 400)})`,
    );
  const remaining = repositories.length - listed.length;
  return ["", "---", PROJECT_WORKSPACE_CONTEXT_MARKER, ...listed]
    .concat(remaining > 0 ? [`…and ${remaining} more`] : [])
    .concat([UNTRUSTED_CONTEXT_NOTICE])
    .join("\n");
}

function buildSuggestions(projects: readonly Project[]) {
  const firstRepo = projects[0]?.name;
  return [
    {
      label: "Reviews",
      prompt: "Which reviews need attention today?",
    },
    {
      label: "Release check",
      prompt: firstRepo
        ? `Are we safe to cut a release of ${firstRepo} this week?`
        : "Are we safe to cut a release this week?",
    },
    {
      label: "Tasks",
      prompt: "Summarize the open tasks and flag anything urgent.",
    },
    {
      label: "Activity",
      prompt: firstRepo
        ? `Summarize recent activity in ${firstRepo}.`
        : "Summarize recent repository activity.",
    },
  ];
}

/** Sorts runnable agents first so the default pick can answer immediately. */
export function useAgentCandidates() {
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const channelsQuery = useChannelsQuery();

  return React.useMemo(() => {
    const managed = managedAgentsQuery.data ?? [];
    const relayAgents = relayAgentsQuery.data ?? [];
    const managedByPubkey = new Map(
      managed.map((agent) => [normalizePubkey(agent.pubkey), agent]),
    );
    const mentionable = getMentionableAgentPubkeys({
      currentPubkey: identityQuery.data?.pubkey,
      eligibilityScope: { type: "community" },
      managedAgentPubkeys: managedByPubkey.keys(),
      relayAgents,
      sharedChannelIds: getSharedChannelIds(channelsQuery.data),
    });

    const candidates: AgentCandidate[] = managed.map((agent) => ({
      pubkey: normalizePubkey(agent.pubkey),
      name: agent.name,
      personaId: agent.personaId,
      isManaged: true,
      isActive: isManagedAgentActive(agent),
    }));
    for (const agent of relayAgents) {
      const pubkey = normalizePubkey(agent.pubkey);
      if (managedByPubkey.has(pubkey) || !mentionable.has(pubkey)) continue;
      candidates.push({
        pubkey,
        name: agent.name,
        isManaged: false,
        isActive:
          agent.status === "unknown" ? null : agent.status !== "offline",
      });
    }

    return candidates.sort((left, right) => {
      // Unknown is neither offline nor proof that the agent can answer now.
      const activityRank = (active: boolean | null) =>
        active === true ? 0 : active === null ? 1 : 2;
      const activityOrder =
        activityRank(left.isActive) - activityRank(right.isActive);
      if (activityOrder) return activityOrder;
      if (left.isManaged !== right.isManaged) return left.isManaged ? -1 : 1;
      return left.name.localeCompare(right.name);
    });
  }, [
    channelsQuery.data,
    identityQuery.data?.pubkey,
    managedAgentsQuery.data,
    relayAgentsQuery.data,
  ]);
}

/** Live message feed for the conversation's backing DM channel, reduced to
 * plain chat rows (kind 9 / 40002 only). Machine-appended context stays
 * inspectable beside the user's message, but defaults to a compact disclosure
 * so the transcript foregrounds what the user actually typed. */
export function ConversationThread({
  channel,
  agent,
  agentAvatarUrl,
  currentPubkey,
  selfAvatarUrl,
  opener,
}: {
  channel: Channel;
  agent: AgentCandidate;
  agentAvatarUrl: string | null;
  currentPubkey: string | null;
  selfAvatarUrl: string | null;
  opener: ProjectsConversationOpener;
}) {
  useChannelSubscription(channel);
  const messagesQuery = useChannelMessagesQuery(channel);
  const threadRootIds = React.useMemo(
    () =>
      (messagesQuery.data ?? [])
        .filter(
          (event) =>
            (event.kind === KIND_STREAM_MESSAGE ||
              event.kind === KIND_STREAM_MESSAGE_V2) &&
            isAtOrAfterConversationOpener(event, opener) &&
            getThreadReference(event.tags).parentId === null,
        )
        .map((event) => event.id),
    [messagesQuery.data, opener],
  );
  const threadReplies = useThreadRepliesForRoots(channel, threadRootIds);
  const toggleReactionMutation = useToggleReactionMutation();
  const agentWorking = useAgentWorking(agent.pubkey, channel.id);
  const bottomRef = React.useRef<HTMLDivElement>(null);
  const normalizedCurrent = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const profiles = React.useMemo(
    () => ({
      [normalizePubkey(agent.pubkey)]: {
        avatarUrl: agentAvatarUrl,
        displayName: agent.name,
        isAgent: true,
        name: agent.name,
        nip05Handle: null,
        ownerPubkey: null,
      },
      ...(normalizedCurrent
        ? {
            [normalizedCurrent]: {
              avatarUrl: selfAvatarUrl,
              displayName: "You",
              isAgent: false,
              name: null,
              nip05Handle: null,
              ownerPubkey: null,
            },
          }
        : {}),
    }),
    [
      agent.name,
      agent.pubkey,
      agentAvatarUrl,
      normalizedCurrent,
      selfAvatarUrl,
    ],
  );
  const conversationMessages = React.useMemo(() => {
    const events = mergeProjectAgentConversationEvents(
      messagesQuery.data ?? [],
      threadReplies.events,
    );
    const contexts = new Map<string, string>();
    const messages = formatTimelineMessages(
      events,
      channel,
      currentPubkey ?? undefined,
      selfAvatarUrl,
      profiles,
    )
      .filter(
        (message) =>
          (message.kind === KIND_STREAM_MESSAGE ||
            message.kind === KIND_STREAM_MESSAGE_V2) &&
          isAtOrAfterConversationOpener(
            {
              created_at: message.createdAt,
              id: message.id,
              tags: message.tags,
            },
            opener,
          ),
      )
      .map((message) => {
        const authorPubkey = message.signerPubkey ?? message.pubkey;
        if (
          !normalizedCurrent ||
          !authorPubkey ||
          normalizePubkey(authorPubkey) !== normalizedCurrent
        ) {
          return message;
        }
        const split = splitProjectDetailAgentContext(message.body);
        if (!split.context) return message;
        contexts.set(message.id, split.context);
        return { ...message, body: split.message };
      });
    return { contexts, messages };
  }, [
    channel,
    currentPubkey,
    messagesQuery.data,
    profiles,
    selfAvatarUrl,
    threadReplies.events,
    opener,
    normalizedCurrent,
  ]);
  const messages = conversationMessages.messages;
  const renderSubmittedContext = React.useCallback(
    (message: TimelineMessage) => {
      const payload = conversationMessages.contexts.get(message.id);
      return payload ? (
        <ProjectAgentSubmittedContextPill payload={payload} />
      ) : null;
    },
    [conversationMessages.contexts],
  );
  const lastMessageId = messages[messages.length - 1]?.id ?? null;
  const handleToggleReaction = React.useCallback(
    async (message: TimelineMessage, emoji: string, remove: boolean) => {
      await toggleReactionMutation.mutateAsync({
        emoji,
        eventId: message.id,
        remove,
      });
    },
    [toggleReactionMutation.mutateAsync],
  );
  React.useEffect(() => {
    if (!lastMessageId && !agentWorking.working) return;
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [lastMessageId, agentWorking.working]);

  return (
    <div data-project-agent-channel-id={channel.id}>
      <MessageThreadTranscript
        channelId={channel.id}
        currentPubkey={currentPubkey ?? undefined}
        messages={messages}
        onToggleReaction={handleToggleReaction}
        profiles={profiles}
        renderAfterMessage={renderSubmittedContext}
      />
      {threadReplies.isError ? (
        <ThreadRepliesErrorCard onRetry={threadReplies.refetch} />
      ) : null}
      {agentWorking.working ? (
        <div className="flex items-center gap-2 pl-11 text-sm text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {agent.name} is working…
        </div>
      ) : null}
      <div ref={bottomRef} />
    </div>
  );
}

/** Full-page agent prompt: ask an agent about the workspace repositories.
 * The conversation stays inline on this page — the prompt is delivered
 * through a DM with the agent under the hood, but no navigation happens. */
export function ProjectsAgentPromptPage({
  projects,
  onClose,
  workspaceId,
}: {
  projects: readonly Project[];
  onClose: () => void;
  workspaceId: string | null;
}) {
  const [prompt, setPrompt] = React.useState("");
  const [storedConversation, setStoredConversation] =
    React.useState<StoredProjectsAgentConversation | null>(null);
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    () => storedConversation?.agentPubkey ?? null,
  );
  const [isSending, setIsSending] = React.useState(false);
  const [isFormattingOpen, setIsFormattingOpen] = React.useState(false);
  const [conversation, setConversation] =
    React.useState<ProjectAgentConversation | null>(null);
  const submitPromptRef = React.useRef<() => void>(() => {});
  const onEditLinkRef = React.useRef<
    ((info: LinkSelectionInfo) => void) | null
  >(null);
  const onLinkSelectionChangeRef = React.useRef<
    ((info: LinkSelectionInfo | null) => void) | null
  >(null);
  const onLinkShortcutRef = React.useRef<(() => boolean) | null>(null);

  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const candidates = useAgentCandidates();
  const channelsQuery = useChannelsQuery();
  const openDmMutation = useOpenDmMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const { activeCommunity } = useCommunities();
  // Tenant scope for the submit sequence below: captured per render, so the
  // value the callback closes over is the community that was active when the
  // user pressed Ask — the backing commands fail closed if it changes while
  // the callback is suspended.
  const relayScope = activeCommunity?.relayUrl
    ? normalizeRelayUrl(activeCommunity.relayUrl)
    : null;
  const signerScope = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : null;
  const scopedWorkspaceId = projectsConversationScope(
    "workspace",
    relayScope,
    signerScope,
    workspaceId ?? "",
  );

  const candidatePubkeys = React.useMemo(
    () => candidates.map((candidate) => candidate.pubkey),
    [candidates],
  );
  const candidateProfilesQuery = useUsersBatchQuery(candidatePubkeys);
  const avatarUrlFor = React.useCallback(
    (pubkey: string) =>
      candidateProfilesQuery.data?.profiles[normalizePubkey(pubkey)]
        ?.avatarUrl ?? null,
    [candidateProfilesQuery.data],
  );

  // Restore strictly from the pointer this feature persisted — never infer
  // a conversation from existing agent DMs (their history is unrelated).
  const restorableConversation = React.useMemo(
    () =>
      restoreProjectsAgentConversation({
        candidates,
        channels: channelsQuery.data ?? [],
        currentPubkey: identityQuery.data?.pubkey ?? null,
        stored: storedConversation,
      }),
    [
      candidates,
      channelsQuery.data,
      identityQuery.data?.pubkey,
      storedConversation,
    ],
  );

  React.useEffect(() => {
    if (conversation || !restorableConversation) return;
    setConversation(restorableConversation);
    setSelectedPubkey(restorableConversation.agent.pubkey);
  }, [conversation, restorableConversation]);

  const selectedAgent =
    conversation?.agent ??
    candidates.find((candidate) => candidate.pubkey === selectedPubkey) ??
    pickDefaultProjectsAgent(candidates);
  const richText = useRichTextEditor({
    editable: !isSending,
    onEditLink: (info) => onEditLinkRef.current?.(info),
    onLinkSelectionChange: (info) => onLinkSelectionChangeRef.current?.(info),
    onLinkShortcut: () => onLinkShortcutRef.current?.() ?? false,
    onSubmit: () => submitPromptRef.current(),
    onUpdate: ({ text }) => setPrompt(text),
    placeholder: conversation
      ? `Reply to ${conversation.agent.name}…`
      : "Are we safe to release this week?",
  });
  const linkEditor = useLinkEditor(richText);
  onEditLinkRef.current = linkEditor.openFromClick;
  onLinkSelectionChangeRef.current = linkEditor.showFromCursor;
  onLinkShortcutRef.current = linkEditor.openFromShortcut;

  React.useEffect(() => {
    const stored = readStoredProjectsAgentConversation(scopedWorkspaceId);
    setStoredConversation(stored);
    setConversation(null);
    setSelectedPubkey(stored?.agentPubkey ?? null);
    setPrompt("");
    richText.clearContent();
  }, [richText.clearContent, scopedWorkspaceId]);

  React.useEffect(() => {
    if (!richText.editor) return;
    const frame = window.requestAnimationFrame(richText.focusPreserve);
    return () => window.cancelAnimationFrame(frame);
  }, [richText.editor, richText.focusPreserve]);

  const suggestions = React.useMemo(
    () => buildSuggestions(projects),
    [projects],
  );
  // Computed once so the pre-send preview and the appended opener payload
  // are byte-identical: the user inspects exactly what will be signed under
  // their key. Repo context rides only on the conversation opener.
  const repoContextPayload = React.useMemo(
    () => repoContextBlock(projects),
    [projects],
  );
  const canSubmit = Boolean(prompt.trim() && selectedAgent && !isSending);

  const handleSubmit = React.useCallback(async () => {
    const trimmed = richText.getMarkdown().trim();
    if (!trimmed || !selectedAgent || isSending) return;

    setIsSending(true);
    try {
      // Repo context rides only on the conversation opener.
      const content = conversation
        ? trimmed
        : `${trimmed}${repoContextPayload}`;
      // The awaits inside suspend across a possible community switch;
      // `submitProjectAgentMessage` binds every relay side effect to the
      // scope captured at render (fail closed) and threads follow-ups onto
      // the opener so a same-second follow-up cannot be hidden by id
      // ordering.
      const { channel, sent } = await submitProjectAgentMessage({
        agent: selectedAgent,
        conversation,
        content,
        mentionPubkeys: [selectedAgent.pubkey],
        relayScope,
        signerScope: identityQuery.data?.pubkey ?? null,
        startAgent: (input) => startAgentMutation.mutateAsync(input),
        openDm: (input) => openDmMutation.mutateAsync(input),
        send: (request) =>
          sendChannelMessage(
            request.channelId,
            request.content,
            request.parentEventId,
            undefined,
            request.mentionPubkeys,
            undefined,
            undefined,
            undefined,
            undefined,
            undefined,
            request.expectedRelayUrl,
            request.expectedSignerPubkey,
          ),
      });
      if (!conversation) {
        // Anchor the conversation to the exact accepted opener event: a bare
        // timestamp cannot isolate it from unrelated same-second DM history.
        const opener = {
          createdAt: sent.createdAt,
          eventId: sent.eventId,
        };
        const nextConversation = {
          channel,
          agent: selectedAgent,
          opener,
        };
        const stored = {
          agentPubkey: selectedAgent.pubkey,
          channelId: channel.id,
          opener,
        };
        setConversation(nextConversation);
        setStoredConversation(stored);
        writeStoredProjectsAgentConversation(scopedWorkspaceId, stored);
      }
      setPrompt("");
      richText.clearContent();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to reach the agent",
      );
    } finally {
      setIsSending(false);
    }
  }, [
    conversation,
    identityQuery.data?.pubkey,
    isSending,
    openDmMutation,
    relayScope,
    repoContextPayload,
    richText.clearContent,
    richText.getMarkdown,
    selectedAgent,
    startAgentMutation,
    scopedWorkspaceId,
  ]);
  submitPromptRef.current = () => {
    void handleSubmit();
  };

  const handleClearConversation = React.useCallback(() => {
    clearStoredProjectsAgentConversation(scopedWorkspaceId);
    setStoredConversation(null);
    setConversation(null);
    setSelectedPubkey(null);
    setPrompt("");
    richText.clearContent();
  }, [richText.clearContent, scopedWorkspaceId]);

  const promptBox = (
    <>
      <div className="rounded-2xl border border-border/60 bg-card p-3 shadow-sm">
        {/* biome-ignore lint/a11y/noStaticElementInteractions: Escape closes the full-page prompt while Tiptap owns text input */}
        <div
          className="rich-text-composer relative max-h-40 min-h-10 overflow-y-auto"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onClose();
            }
          }}
        >
          <EditorContent editor={richText.editor} />
        </div>
        <div className="flex items-center justify-between gap-2 pt-2">
          <div className="flex min-w-0 items-center gap-1">
            <Button
              aria-label="Toggle formatting"
              aria-pressed={isFormattingOpen}
              className="h-7 w-7 shrink-0 px-0"
              disabled={isSending}
              onClick={() => setIsFormattingOpen((open) => !open)}
              size="icon"
              title="Formatting"
              type="button"
              variant={isFormattingOpen ? "default" : "ghost"}
            >
              <ALargeSmall className="h-4 w-4" />
            </Button>
            {isFormattingOpen ? (
              <div className="min-w-0 overflow-x-auto">
                <FormattingToolbar
                  disabled={isSending}
                  editor={richText.editor}
                  onLinkButton={linkEditor.openFromToolbar}
                />
              </div>
            ) : (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    className="h-7 max-w-56 gap-1.5 rounded-full px-2.5 text-xs"
                    data-testid="projects-agent-picker"
                    disabled={candidates.length === 0 || conversation !== null}
                    size="sm"
                    type="button"
                    variant="ghost"
                  >
                    {selectedAgent ? (
                      <UserAvatar
                        accent
                        avatarUrl={avatarUrlFor(selectedAgent.pubkey)}
                        className="shrink-0"
                        displayName={selectedAgent.name}
                        shape="squircle"
                        size="xs"
                      />
                    ) : null}
                    <span className="min-w-0 truncate">
                      {selectedAgent?.name ?? "No agents available"}
                    </span>
                    {candidates.length > 0 && conversation === null ? (
                      <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                    ) : null}
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="min-w-52">
                  <DropdownMenuRadioGroup
                    onValueChange={setSelectedPubkey}
                    value={selectedAgent?.pubkey ?? ""}
                  >
                    {candidates.map((candidate) => (
                      <DropdownMenuRadioItem
                        key={candidate.pubkey}
                        value={candidate.pubkey}
                      >
                        <UserAvatar
                          accent
                          avatarUrl={avatarUrlFor(candidate.pubkey)}
                          className="mr-2 shrink-0"
                          displayName={candidate.name}
                          shape="squircle"
                          size="xs"
                        />
                        <span className="min-w-0 truncate">
                          {candidate.name}
                        </span>
                        {candidate.isActive !== null ? (
                          <span
                            className={cn(
                              "ml-2 h-1.5 w-1.5 shrink-0 rounded-full",
                              candidate.isActive
                                ? "bg-emerald-500"
                                : "bg-muted-foreground/40",
                            )}
                          />
                        ) : null}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
          <Button
            className="h-7 gap-1.5 rounded-full px-3 text-xs"
            disabled={!canSubmit}
            onClick={() => void handleSubmit()}
            size="sm"
            type="button"
            variant="default"
          >
            {isSending ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <SendHorizontal className="h-3.5 w-3.5" />
            )}
            Ask
          </Button>
        </div>
        {conversation ? null : (
          <div className="flex justify-end pt-1">
            <AgentContextPayloadPreview
              payload={repoContextPayload}
              triggerLabel="Context appended to your first message"
            />
          </div>
        )}
      </div>
      {linkEditor.card}
      {linkEditor.dialog}
    </>
  );

  if (conversation) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto px-4">
          <div className="w-full pb-6 pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1rem)]">
            <div className="mb-4 flex justify-end">
              <Button
                className="h-8 gap-1.5 rounded-full px-3 text-xs text-muted-foreground"
                onClick={handleClearConversation}
                size="sm"
                type="button"
                variant="ghost"
              >
                <Trash2 className="h-3.5 w-3.5" />
                Clear conversation
              </Button>
            </div>
            <ConversationThread
              agent={conversation.agent}
              agentAvatarUrl={avatarUrlFor(conversation.agent.pubkey)}
              channel={conversation.channel}
              currentPubkey={identityQuery.data?.pubkey ?? null}
              selfAvatarUrl={profileQuery.data?.avatarUrl ?? null}
              opener={conversation.opener}
            />
          </div>
        </div>
        <div className="px-4 pb-4">
          <div className="w-full">{promptBox}</div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-1 items-center justify-center overflow-y-auto px-4">
      <div className="w-full max-w-xl space-y-6 py-10">
        <h2 className="text-center text-lg font-semibold text-foreground">
          Ask an agent about your projects
        </h2>

        {promptBox}

        {candidates.length === 0 ? (
          <p className="text-center text-sm text-muted-foreground">
            No agents available yet — create or start one from the Agents view
            to ask about your repositories.
          </p>
        ) : (
          <div className="space-y-1.5">
            {suggestions.map((suggestion) => (
              <button
                className="flex w-full items-baseline gap-2 rounded-xl border border-border/50 bg-card/60 px-4 py-2 text-left transition-colors duration-150 hover:bg-muted/30"
                key={suggestion.label}
                onClick={() => {
                  setPrompt(suggestion.prompt);
                  richText.setContent(suggestion.prompt);
                  richText.focusEnd();
                }}
                type="button"
              >
                <span className="shrink-0 text-xs font-semibold text-foreground">
                  {suggestion.label}
                </span>
                <span className="min-w-0 truncate text-xs text-muted-foreground">
                  {suggestion.prompt}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
