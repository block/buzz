import { Search } from "lucide-react";
import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  useContactListQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import {
  useGlobalNotesQuery,
  pulseQueryKeys,
  useLikedNotesQuery,
  useMyNotesQuery,
  usePublishNoteMutation,
  usePulseReactionsQuery,
  useTimelineQuery,
} from "@/features/pulse/hooks";
import { groupAgentNotes } from "@/features/pulse/lib/groupAgentNotes";
import { usePulseNoteActions } from "@/features/pulse/lib/useNoteActions";
import { AgentActivityCard } from "@/features/pulse/ui/AgentActivityCard";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import { NoteCard } from "@/features/pulse/ui/NoteCard";
import { PulseTabBar } from "@/features/pulse/ui/PulseTabBar";
import type { UserNote } from "@/shared/api/socialTypes";
import type { ChannelMember, UserProfileSummary } from "@/shared/api/types";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type PulseTab =
  | "search"
  | "everyone"
  | "people"
  | "liked"
  | "agents"
  | "mine";

const pulsePanelId = (tab: PulseTab) => `pulse-panel-${tab}`;
const pulseTabId = (tab: PulseTab) => `pulse-tab-${tab}`;

type PulseViewProps = {
  currentPubkey?: string;
};

function EmptyState({ message }: { message: string }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-border/60 px-4 py-12 text-center">
      <p className="text-sm text-muted-foreground">{message}</p>
    </div>
  );
}

function TimelineSkeleton() {
  return (
    <div className="space-y-5">
      {[1, 2, 3, 4].map((i) => (
        <div className="flex gap-3 px-1 py-2 sm:px-2" key={i}>
          <Skeleton className="h-8 w-8 shrink-0 rounded-full" />
          <div className="min-w-0 flex-1 space-y-2">
            <Skeleton className="h-3.5 w-32" />
            <Skeleton className="h-4 w-full max-w-md" />
            <Skeleton className="h-4 w-3/4 max-w-sm" />
          </div>
        </div>
      ))}
    </div>
  );
}

export function PulseView({ currentPubkey }: PulseViewProps) {
  const [activeTab, setActiveTab] = React.useState<PulseTab>("everyone");
  const [searchQuery, setSearchQuery] = React.useState("");
  const contactListQuery = useContactListQuery(currentPubkey);
  const contacts = contactListQuery.data?.contacts ?? [];
  const contactPubkeys = React.useMemo(
    () => contacts.map((c) => c.pubkey),
    [contacts],
  );
  const contactPubkeySet = React.useMemo(
    () => new Set(contactPubkeys),
    [contactPubkeys],
  );
  const peoplePubkeys = React.useMemo(() => contactPubkeys, [contactPubkeys]);

  const relayAgentsQuery = useRelayAgentsQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgents = React.useMemo(() => {
    const agentsByPubkey = new Map<
      string,
      NonNullable<typeof relayAgentsQuery.data>[number]
    >();
    for (const agent of relayAgentsQuery.data ?? []) {
      agentsByPubkey.set(agent.pubkey, agent);
    }
    for (const agent of managedAgentsQuery.data ?? []) {
      if (!agentsByPubkey.has(agent.pubkey)) {
        agentsByPubkey.set(agent.pubkey, {
          pubkey: agent.pubkey,
          name: agent.name,
          agentType: agent.agentCommand,
          channels: [],
          channelIds: [],
          capabilities: [],
          status:
            agent.status === "running" || agent.status === "deployed"
              ? "online"
              : "offline",
          respondTo: agent.respondTo,
        });
      }
    }
    return [...agentsByPubkey.values()];
  }, [managedAgentsQuery.data, relayAgentsQuery.data]);
  const agentPubkeys = React.useMemo(
    () => relayAgents.map((a) => a.pubkey),
    [relayAgents],
  );
  const agentPubkeySet = React.useMemo(
    () => new Set(agentPubkeys),
    [agentPubkeys],
  );
  const agentStatusMap = React.useMemo(() => {
    const map: Record<string, "online" | "away" | "offline"> = {};
    for (const a of relayAgents) {
      map[a.pubkey] = a.status;
    }
    return map;
  }, [relayAgents]);

  const mentionPubkeys = React.useMemo(
    () =>
      [...new Set([currentPubkey, ...peoplePubkeys, ...agentPubkeys])].filter(
        (pubkey): pubkey is string =>
          typeof pubkey === "string" && pubkey.length > 0,
      ),
    [currentPubkey, peoplePubkeys, agentPubkeys],
  );

  const everyoneQuery = useGlobalNotesQuery(activeTab === "everyone");
  const peopleQuery = useTimelineQuery(peoplePubkeys, activeTab === "people");
  const likedNotesQuery = useLikedNotesQuery(
    currentPubkey,
    activeTab === "liked",
  );
  const agentTimelineQuery = useTimelineQuery(
    agentPubkeys,
    activeTab === "agents",
  );
  const myNotesQuery = useMyNotesQuery(
    activeTab === "mine" ? currentPubkey : undefined,
  );
  const publishMutation = usePublishNoteMutation(currentPubkey);
  const visibleNotes: UserNote[] = React.useMemo(() => {
    if (activeTab === "everyone") {
      return everyoneQuery.data?.notes ?? [];
    }
    if (activeTab === "people") {
      // Filter out agent notes from the people timeline unless the user follows them.
      return (peopleQuery.data?.notes ?? []).filter(
        (n) => !agentPubkeySet.has(n.pubkey) || contactPubkeySet.has(n.pubkey),
      );
    }
    if (activeTab === "liked") {
      return likedNotesQuery.data?.notes ?? [];
    }
    if (activeTab === "agents") {
      return agentTimelineQuery.data?.notes ?? [];
    }
    return myNotesQuery.data?.notes ?? [];
  }, [
    activeTab,
    everyoneQuery.data,
    peopleQuery.data,
    agentTimelineQuery.data,
    likedNotesQuery.data,
    myNotesQuery.data,
    agentPubkeySet,
    contactPubkeySet,
  ]);

  const visibleNoteIds = React.useMemo(
    () => visibleNotes.map((note) => note.id),
    [visibleNotes],
  );
  const reactionsQuery = usePulseReactionsQuery(visibleNoteIds, currentPubkey);
  const reactionQueryKey = React.useMemo(
    () => pulseQueryKeys.reactions(visibleNoteIds),
    [visibleNoteIds],
  );
  const noteActions = usePulseNoteActions({
    currentPubkey,
    reactionQueryKey,
    reactions: reactionsQuery.data ?? new Map(),
  });

  const agentNoteGroups = React.useMemo(
    () => (activeTab === "agents" ? groupAgentNotes(visibleNotes) : []),
    [activeTab, visibleNotes],
  );

  const notePubkeys = React.useMemo(
    () => [...new Set(visibleNotes.map((n) => n.pubkey))],
    [visibleNotes],
  );
  const profilesQuery = useUsersBatchQuery(notePubkeys, {
    enabled: notePubkeys.length > 0,
  });
  const profiles: Record<string, UserProfileSummary> =
    profilesQuery.data?.profiles ?? {};

  const mentionProfilesQuery = useUsersBatchQuery(mentionPubkeys, {
    enabled: mentionPubkeys.length > 0,
  });
  const mentionProfiles = mentionProfilesQuery.data?.profiles ?? {};
  const currentProfile = currentPubkey
    ? (mentionProfiles[currentPubkey.toLowerCase()] ?? null)
    : null;
  const currentDisplayName =
    currentProfile?.displayName ??
    (currentPubkey ? `${currentPubkey.slice(0, 8)}...` : "You");

  const pulseMentionMembers = React.useMemo<ChannelMember[]>(() => {
    const members: ChannelMember[] = [];
    for (const pubkey of mentionPubkeys) {
      const profile = mentionProfiles[pubkey.toLowerCase()];
      members.push({
        pubkey,
        role: "member",
        isAgent: profile?.isAgent ?? false,
        joinedAt: "",
        displayName: profile?.displayName ?? null,
      });
    }
    return members;
  }, [mentionPubkeys, mentionProfiles]);

  const activeQuery =
    activeTab === "everyone"
      ? everyoneQuery
      : activeTab === "people"
        ? peopleQuery
        : activeTab === "liked"
          ? likedNotesQuery
          : activeTab === "agents"
            ? agentTimelineQuery
            : myNotesQuery;
  const isLoading = activeQuery.isLoading;

  const emptyMessages: Record<PulseTab, string> = {
    search: "Search Pulse notes by author or text.",
    everyone: "No public notes yet.",
    people: "No notes yet. Follow people to see their updates here.",
    liked: "No likes yet — tap the heart on a note to save it here.",
    agents:
      agentPubkeys.length === 0
        ? "No agents registered yet."
        : "No agent notes yet. Agents post here when they publish.",
    mine: "You haven't posted any notes yet.",
  };

  function renderTimeline() {
    if (isLoading) return <TimelineSkeleton />;

    if (activeTab === "agents") {
      return agentNoteGroups.length === 0 ? (
        <EmptyState message={emptyMessages.agents} />
      ) : (
        agentNoteGroups.map((group) => (
          <AgentActivityCard
            agentStatus={agentStatusMap[group.pubkey]}
            group={group}
            key={`${group.pubkey}-${group.latestAt}`}
            profile={profiles[group.pubkey.toLowerCase()] ?? null}
          />
        ))
      );
    }

    return visibleNotes.length === 0 ? (
      <EmptyState message={emptyMessages[activeTab]} />
    ) : (
      visibleNotes.map((note) => (
        <NoteCard
          actions={{
            reply: noteActions.reply,
            share: noteActions.share,
            startDm: noteActions.startDm,
            toggleUpvote: noteActions.toggleUpvote,
          }}
          composerProfiles={mentionProfiles}
          currentUserDisplayName={currentDisplayName}
          currentUserProfile={currentProfile}
          isAgent={agentPubkeySet.has(note.pubkey)}
          isOwnNote={note.pubkey === currentPubkey}
          isReplySending={noteActions.isReplySending}
          isUpvotePending={noteActions.isUpvotePending(note.id)}
          isUpvoted={noteActions.isUpvoted(note.id)}
          reactionCount={noteActions.reactionCount(note.id)}
          key={note.id}
          members={pulseMentionMembers}
          note={note}
          profile={profiles[note.pubkey.toLowerCase()] ?? null}
        />
      ))
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <PulseTabBar
        activeTab={activeTab}
        getPanelId={pulsePanelId}
        getTabId={pulseTabId}
        onTabChange={setActiveTab}
        relayAgents={relayAgents}
      />

      <div className="mt-0 min-h-0 flex-1 overflow-y-auto">
        <div
          aria-labelledby={pulseTabId(activeTab)}
          className={`mx-auto flex w-full max-w-2xl flex-col px-4 pb-10 sm:px-6 ${
            activeTab !== "search" && activeTab !== "agents" ? "pt-0" : "pt-7"
          }`}
          id={pulsePanelId(activeTab)}
          role="tabpanel"
        >
          {activeTab === "search" ? (
            <div className="flex min-h-[calc(100vh-96px)] items-center justify-center">
              <div className="relative flex w-full max-w-xl flex-col items-center px-2">
                <h2 className="mb-5 text-center text-2xl font-semibold tracking-tight text-foreground">
                  What are you looking for?
                </h2>
                <div className="relative w-full max-w-lg">
                  <div className="relative rounded-full border border-foreground/10 bg-background/80 p-1 shadow-[0_12px_48px_rgba(0,0,0,0.12)] backdrop-blur-xl dark:border-white/10 dark:bg-white/[0.04] dark:shadow-[0_16px_70px_rgba(0,0,0,0.55)]">
                    <Search className="pointer-events-none absolute left-4 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground dark:text-white/55" />
                    <Input
                      autoFocus
                      className="h-9 rounded-full border-0 bg-transparent pl-10 pr-12 text-sm shadow-none placeholder:text-muted-foreground/80 focus-visible:ring-0 dark:text-white dark:placeholder:text-white/60"
                      onChange={(event) => setSearchQuery(event.target.value)}
                      placeholder="What would you like to know?"
                      type="search"
                      value={searchQuery}
                    />
                    <button
                      aria-label="Search Pulse"
                      className="absolute right-1.5 top-1/2 flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-full bg-foreground/10 text-foreground transition-colors hover:bg-foreground/15 dark:bg-white/85 dark:text-black dark:hover:bg-white"
                      type="button"
                    >
                      <Search className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          ) : activeTab !== "agents" ? (
            <div className="sticky top-0 z-10 mb-7 pb-3 pt-7">
              <div
                aria-hidden="true"
                className="pointer-events-none absolute inset-x-0 top-[-1px] h-8 bg-background"
              />
              {publishMutation.isError && (
                <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  {publishMutation.error instanceof Error
                    ? publishMutation.error.message
                    : "Failed to publish note"}
                </div>
              )}
              <ForumComposer
                autocompleteBelow
                className="pulse-composer overflow-hidden rounded-2xl border-border/50 bg-background/70 p-2 shadow-none backdrop-blur-xl supports-[backdrop-filter]:bg-background/55"
                compact
                header={
                  <div className="flex min-w-0 items-center gap-2">
                    <UserAvatar
                      avatarUrl={currentProfile?.avatarUrl ?? null}
                      className="!h-7 !w-7 shrink-0"
                      displayName={currentDisplayName}
                    />
                    <span className="max-w-32 truncate text-sm font-medium text-foreground">
                      {currentDisplayName}
                    </span>
                  </div>
                }
                members={pulseMentionMembers}
                placeholder="What's on your mind?"
                isSending={publishMutation.isPending}
                onSubmit={(content, mentionPubkeys, mediaTags) =>
                  publishMutation.mutateAsync({
                    content,
                    mentionPubkeys,
                    mediaTags,
                  })
                }
                profiles={mentionProfiles}
              />
            </div>
          ) : null}

          {activeTab !== "search" ? (
            <div className="space-y-4">{renderTimeline()}</div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
