import { ChevronDown, Loader2, SendHorizontal, Trash2 } from "lucide-react";
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
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import type { Project } from "@/features/projects/hooks";
import {
  markProjectsAgentConversationCleared,
  readProjectsAgentConversationClearedAt,
  readStoredProjectsAgentConversation,
  type StoredProjectsAgentConversation,
  writeStoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
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

function latestAgentConversation({
  candidates,
  channels,
  clearedAt,
  currentPubkey,
}: {
  candidates: readonly AgentCandidate[];
  channels: readonly Channel[];
  clearedAt: number;
  currentPubkey: string | null;
}): ProjectAgentConversation | null {
  if (!currentPubkey) return null;
  const normalizedCurrent = normalizePubkey(currentPubkey);
  const candidatesByPubkey = new Map(
    candidates.map((candidate) => [candidate.pubkey, candidate]),
  );

  let latest: {
    conversation: ProjectAgentConversation;
    timestamp: number;
  } | null = null;
  for (const channel of channels) {
    if (channel.channelType !== "dm" || !channel.lastMessageAt) continue;
    const participantPubkeys = [
      ...new Set(channel.participantPubkeys.map(normalizePubkey)),
    ];
    if (!participantPubkeys.includes(normalizedCurrent)) continue;
    const otherPubkeys = participantPubkeys.filter(
      (pubkey) => pubkey !== normalizedCurrent,
    );
    if (otherPubkeys.length !== 1) continue;
    const agent = candidatesByPubkey.get(otherPubkeys[0]);
    if (!agent) continue;
    const timestamp = Date.parse(channel.lastMessageAt);
    if (
      !Number.isFinite(timestamp) ||
      timestamp <= clearedAt ||
      (latest && timestamp <= latest.timestamp)
    ) {
      continue;
    }
    latest = {
      conversation: {
        agent,
        channel,
        visibleAfter: Math.floor(clearedAt / 1_000),
      },
      timestamp,
    };
  }
  return latest?.conversation ?? null;
}

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
    () =>
      (messagesQuery.data ?? [])
        .filter(
          (event) =>
            (event.kind === KIND_STREAM_MESSAGE ||
              event.kind === KIND_STREAM_MESSAGE_V2) &&
            event.created_at >= visibleAfter,
        )
        .sort((left, right) => left.created_at - right.created_at),
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
  const [clearedAt, setClearedAt] = React.useState(() =>
    readProjectsAgentConversationClearedAt(workspaceId),
  );
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    () => storedConversation?.agentPubkey ?? null,
  );
  const [isSending, setIsSending] = React.useState(false);
  const [conversation, setConversation] =
    React.useState<ProjectAgentConversation | null>(null);
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

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

  const restorableConversation = React.useMemo(() => {
    if (storedConversation) {
      const channel = channelsQuery.data?.find(
        (candidate) => candidate.id === storedConversation.channelId,
      );
      const agent = candidates.find(
        (candidate) =>
          candidate.pubkey === normalizePubkey(storedConversation.agentPubkey),
      );
      return channel && agent
        ? {
            agent,
            channel,
            visibleAfter: storedConversation.visibleAfter,
          }
        : null;
    }
    return latestAgentConversation({
      candidates,
      channels: channelsQuery.data ?? [],
      clearedAt,
      currentPubkey: identityQuery.data?.pubkey ?? null,
    });
  }, [
    candidates,
    channelsQuery.data,
    clearedAt,
    identityQuery.data?.pubkey,
    storedConversation,
  ]);

  React.useEffect(() => {
    if (conversation || !restorableConversation) return;
    setConversation(restorableConversation);
    setSelectedPubkey(restorableConversation.agent.pubkey);
    const stored = {
      agentPubkey: restorableConversation.agent.pubkey,
      channelId: restorableConversation.channel.id,
      visibleAfter: restorableConversation.visibleAfter,
    };
    setStoredConversation(stored);
    writeStoredProjectsAgentConversation(workspaceId, stored);
  }, [conversation, restorableConversation, workspaceId]);

  const selectedAgent =
    conversation?.agent ??
    candidates.find((candidate) => candidate.pubkey === selectedPubkey) ??
    candidates[0] ??
    null;
  const suggestions = React.useMemo(
    () => buildSuggestions(projects),
    [projects],
  );
  const canSubmit = Boolean(prompt.trim() && selectedAgent && !isSending);

  const handleSubmit = React.useCallback(async () => {
    const trimmed = prompt.trim();
    if (!trimmed || !selectedAgent || isSending) return;

    setIsSending(true);
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
          visibleAfter: Math.floor(clearedAt / 1_000),
        };
        const stored = {
          agentPubkey: selectedAgent.pubkey,
          channelId: channel.id,
          visibleAfter: nextConversation.visibleAfter,
        };
        setConversation(nextConversation);
        setStoredConversation(stored);
        writeStoredProjectsAgentConversation(workspaceId, stored);
      }
      setPrompt("");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to reach the agent",
      );
    } finally {
      setIsSending(false);
    }
  }, [
    clearedAt,
    conversation,
    isSending,
    openDmMutation,
    projects,
    prompt,
    selectedAgent,
    startAgentMutation,
    workspaceId,
  ]);

  const handleClearConversation = React.useCallback(() => {
    const nextClearedAt = Date.now();
    markProjectsAgentConversationCleared(workspaceId, nextClearedAt);
    setClearedAt(nextClearedAt);
    setStoredConversation(null);
    setConversation(null);
    setSelectedPubkey(null);
    setPrompt("");
  }, [workspaceId]);

  const promptBox = (
    <div className="rounded-2xl border border-border/60 bg-card p-3 shadow-sm">
      <textarea
        aria-label="Ask an agent"
        // biome-ignore lint/a11y/noAutofocus: this page exists only to type a prompt
        autoFocus
        className="max-h-40 w-full resize-none bg-transparent text-base text-foreground outline-hidden placeholder:text-muted-foreground/55"
        onChange={(event) => setPrompt(event.target.value)}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing
          ) {
            event.preventDefault();
            void handleSubmit();
            return;
          }
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
        placeholder={
          conversation
            ? `Reply to ${conversation.agent.name}…`
            : "Are we safe to release this week?"
        }
        ref={textareaRef}
        rows={2}
        value={prompt}
      />
      <div className="flex items-center justify-between gap-2 pt-2">
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
                  <span className="min-w-0 truncate">{candidate.name}</span>
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
                  textareaRef.current?.focus();
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
