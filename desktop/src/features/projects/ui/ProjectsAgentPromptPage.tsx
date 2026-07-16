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
import {
  useChannelMessagesQuery,
  useChannelSubscription,
} from "@/features/messages/hooks";
import { useLinkEditor } from "@/features/messages/lib/useLinkEditor";
import {
  type LinkSelectionInfo,
  useRichTextEditor,
} from "@/features/messages/lib/useRichTextEditor";
import { FormattingToolbar } from "@/features/messages/ui/FormattingToolbar";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  restoreProjectsAgentConversation,
  visibleConversationMessages,
} from "@/features/projects/lib/projectAgentConversation";
import {
  clearStoredProjectsAgentConversation,
  readStoredProjectsAgentConversation,
  type StoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentCandidate = {
  pubkey: string;
  name: string;
  /** Managed agents can be auto-started before the prompt is sent. */
  isManaged: boolean;
  isActive: boolean;
};

type ProjectAgentConversation = {
  channel: Channel;
  agent: AgentCandidate;
  visibleAfter: number;
};

const MAX_CONTEXT_REPOS = 8;
const REPO_CONTEXT_MARKER = "Workspace repositories:";

/** Compact machine-readable footer so the agent can scope git queries
 * (repo announcements are addressable by these coordinates). Only sent
 * with the first message of a conversation. */
function repoContextBlock(projects: readonly Project[]) {
  if (projects.length === 0) return "";
  const listed = projects
    .slice(0, MAX_CONTEXT_REPOS)
    .map((project) => `- ${project.name} (${project.repoAddress})`);
  const remaining = projects.length - listed.length;
  return ["", "---", REPO_CONTEXT_MARKER, ...listed]
    .concat(remaining > 0 ? [`…and ${remaining} more`] : [])
    .join("\n");
}

/** Hides the machine-readable repo footer when rendering the user's own
 * prompt back in the inline conversation. */
function stripRepoContext(content: string) {
  const markerIndex = content.indexOf(`---\n${REPO_CONTEXT_MARKER}`);
  if (markerIndex === -1) return content;
  return content.slice(0, markerIndex).replace(/\n+$/, "");
}

function buildSuggestions(projects: readonly Project[]) {
  const firstRepo = projects[0]?.name;
  return [
    {
      label: "PR review",
      prompt: "Which pull requests need attention today?",
    },
    {
      label: "Release check",
      prompt: firstRepo
        ? `Are we safe to cut a release of ${firstRepo} this week?`
        : "Are we safe to cut a release this week?",
    },
    {
      label: "Issues",
      prompt: "Summarize the open issues and flag anything urgent.",
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
function useAgentCandidates() {
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
      managedAgentPubkeys: managedByPubkey.keys(),
      relayAgents,
      sharedChannelIds: getSharedChannelIds(channelsQuery.data),
    });

    const candidates: AgentCandidate[] = managed.map((agent) => ({
      pubkey: normalizePubkey(agent.pubkey),
      name: agent.name,
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
        isActive: agent.status !== "offline",
      });
    }

    return candidates.sort((left, right) => {
      if (left.isActive !== right.isActive) return left.isActive ? -1 : 1;
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
 * plain chat rows (kind 9 / 40002 only). */
function ConversationThread({
  channel,
  agent,
  agentAvatarUrl,
  currentPubkey,
  selfAvatarUrl,
  visibleAfter,
}: {
  channel: Channel;
  agent: AgentCandidate;
  agentAvatarUrl: string | null;
  currentPubkey: string | null;
  selfAvatarUrl: string | null;
  visibleAfter: number;
}) {
  useChannelSubscription(channel);
  const messagesQuery = useChannelMessagesQuery(channel);
  const agentWorking = useAgentWorking(agent.pubkey, channel.id);
  const bottomRef = React.useRef<HTMLDivElement>(null);

  const messages = React.useMemo(
    () => visibleConversationMessages(messagesQuery.data ?? [], visibleAfter),
    [messagesQuery.data, visibleAfter],
  );

  const lastMessageId = messages[messages.length - 1]?.id ?? null;
  React.useEffect(() => {
    if (!lastMessageId && !agentWorking.working) return;
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [lastMessageId, agentWorking.working]);

  const normalizedCurrent = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return (
    <div className="space-y-5">
      {messages.map((event) => {
        const isSelf = normalizePubkey(event.pubkey) === normalizedCurrent;
        return (
          <div className="flex gap-3" key={event.localKey ?? event.id}>
            <UserAvatar
              accent={!isSelf}
              avatarUrl={isSelf ? selfAvatarUrl : agentAvatarUrl}
              className="mt-0.5 shrink-0"
              displayName={isSelf ? "You" : agent.name}
              size="sm"
            />
            <div className="min-w-0 flex-1 space-y-0.5">
              <span className="text-xs font-semibold text-muted-foreground">
                {isSelf ? "You" : agent.name}
              </span>
              <Markdown
                className="text-base text-foreground"
                content={
                  isSelf ? stripRepoContext(event.content) : event.content
                }
              />
            </div>
          </div>
        );
      })}
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
    React.useState<StoredProjectsAgentConversation | null>(() =>
      readStoredProjectsAgentConversation(workspaceId),
    );
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
        stored: storedConversation,
      }),
    [candidates, channelsQuery.data, storedConversation],
  );

  React.useEffect(() => {
    if (conversation || !restorableConversation) return;
    setConversation(restorableConversation);
    setSelectedPubkey(restorableConversation.agent.pubkey);
  }, [conversation, restorableConversation]);

  const selectedAgent =
    conversation?.agent ??
    candidates.find((candidate) => candidate.pubkey === selectedPubkey) ??
    candidates[0] ??
    null;
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
    if (!richText.editor) return;
    const frame = window.requestAnimationFrame(richText.focusPreserve);
    return () => window.cancelAnimationFrame(frame);
  }, [richText.editor, richText.focusPreserve]);

  const suggestions = React.useMemo(
    () => buildSuggestions(projects),
    [projects],
  );
  const canSubmit = Boolean(prompt.trim() && selectedAgent && !isSending);

  const handleSubmit = React.useCallback(async () => {
    const trimmed = richText.getMarkdown().trim();
    if (!trimmed || !selectedAgent || isSending) return;

    setIsSending(true);
    // Cutoff captured before sending: the opening prompt lands at or after
    // this instant, while everything a reused DM channel already held stays
    // hidden from the Projects page.
    const visibleAfter =
      conversation?.visibleAfter ?? Math.floor(Date.now() / 1_000);
    try {
      if (selectedAgent.isManaged && !selectedAgent.isActive) {
        await startAgentMutation.mutateAsync(selectedAgent.pubkey);
      }
      const channel =
        conversation?.channel ??
        (await openDmMutation.mutateAsync({
          pubkeys: [selectedAgent.pubkey],
        }));
      // Repo context rides only on the conversation opener.
      const content = conversation
        ? trimmed
        : `${trimmed}${repoContextBlock(projects)}`;
      await sendChannelMessage(channel.id, content, undefined, undefined, [
        selectedAgent.pubkey,
      ]);
      if (!conversation) {
        const nextConversation = {
          channel,
          agent: selectedAgent,
          visibleAfter,
        };
        const stored = {
          agentPubkey: selectedAgent.pubkey,
          channelId: channel.id,
          visibleAfter,
        };
        setConversation(nextConversation);
        setStoredConversation(stored);
        writeStoredProjectsAgentConversation(workspaceId, stored);
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
    isSending,
    openDmMutation,
    projects,
    richText.clearContent,
    richText.getMarkdown,
    selectedAgent,
    startAgentMutation,
    workspaceId,
  ]);
  submitPromptRef.current = () => {
    void handleSubmit();
  };

  const handleClearConversation = React.useCallback(() => {
    clearStoredProjectsAgentConversation(workspaceId);
    setStoredConversation(null);
    setConversation(null);
    setSelectedPubkey(null);
    setPrompt("");
    richText.clearContent();
  }, [richText.clearContent, workspaceId]);

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
                          size="xs"
                        />
                        <span className="min-w-0 truncate">
                          {candidate.name}
                        </span>
                        <span
                          className={cn(
                            "ml-2 h-1.5 w-1.5 shrink-0 rounded-full",
                            candidate.isActive
                              ? "bg-emerald-500"
                              : "bg-muted-foreground/40",
                          )}
                        />
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
              visibleAfter={conversation.visibleAfter}
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
