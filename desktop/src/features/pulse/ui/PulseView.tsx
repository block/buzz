import { Check, Filter, Search } from "lucide-react";
import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  useContactListQuery,
  useFollowMutation,
  useMyNotesQuery,
  usePublishNoteMutation,
  useTimelineQuery,
  useUnfollowMutation,
} from "@/features/pulse/hooks";
import { groupAgentNotes } from "@/features/pulse/lib/groupAgentNotes";
import { AgentActivityCard } from "@/features/pulse/ui/AgentActivityCard";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import { NoteCard } from "@/features/pulse/ui/NoteCard";
import type { UserNote } from "@/shared/api/socialTypes";
import type {
  ChannelMember,
  RelayAgent,
  UserProfileSummary,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type PulseTab = "search" | "foryou" | "people" | "agents" | "mine";

const tabTriggerClassName =
  "h-7 rounded-full px-3.5 py-0 text-xs font-semibold shadow-none transition-colors !text-muted-foreground hover:!text-foreground data-[state=active]:bg-background data-[state=active]:!text-foreground data-[state=active]:shadow-sm dark:!text-white/35 dark:hover:!text-white/70 dark:data-[state=active]:!text-white";

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

// ── Agent filter dropdown ──────────────────────────────────────────────────

function AgentFilter({
  agents,
  profiles,
  selectedPubkey,
  onSelect,
}: {
  agents: RelayAgent[];
  profiles: Record<string, UserProfileSummary>;
  selectedPubkey: string | null;
  onSelect: (pubkey: string | null) => void;
}) {
  const selectedName = selectedPubkey
    ? (profiles[selectedPubkey.toLowerCase()]?.displayName ??
      agents.find((a) => a.pubkey === selectedPubkey)?.name ??
      `${selectedPubkey.slice(0, 8)}...`)
    : null;

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          className="h-7 gap-1.5 px-2 text-xs"
          size="sm"
          variant={selectedPubkey ? "secondary" : "ghost"}
        >
          <Filter className="h-3 w-3" />
          {selectedName ?? "All agents"}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-48 overflow-y-auto">
        <DropdownMenuItem onClick={() => onSelect(null)}>
          {!selectedPubkey ? (
            <Check className="h-3.5 w-3.5" />
          ) : (
            <span className="h-3.5 w-3.5" />
          )}
          All agents
        </DropdownMenuItem>
        {agents.map((agent) => {
          const name =
            profiles[agent.pubkey.toLowerCase()]?.displayName ??
            agent.name ??
            `${agent.pubkey.slice(0, 8)}...`;
          const isSelected = selectedPubkey === agent.pubkey;
          return (
            <DropdownMenuItem
              key={agent.pubkey}
              onClick={() => onSelect(agent.pubkey)}
            >
              {isSelected ? (
                <Check className="h-3.5 w-3.5" />
              ) : (
                <span className="h-3.5 w-3.5" />
              )}
              {name}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// ── Main PulseView ─────────────────────────────────────────────────────────

export function PulseView({ currentPubkey }: PulseViewProps) {
  const [activeTab, setActiveTab] = React.useState<PulseTab>("foryou");
  const [agentFilter, setAgentFilter] = React.useState<string | null>(null);
  const [searchQuery, setSearchQuery] = React.useState("");

  // ── Contact list & follow state ────────────────────────────────────────
  const contactListQuery = useContactListQuery(currentPubkey);
  const contacts = contactListQuery.data?.contacts ?? [];
  const contactPubkeys = React.useMemo(
    () => contacts.map((c) => c.pubkey),
    [contacts],
  );
  const followingSet = React.useMemo(
    () => new Set(contactPubkeys),
    [contactPubkeys],
  );

  // People-only pubkeys (contacts + self, no agents)
  const peoplePubkeys = React.useMemo(
    () =>
      currentPubkey
        ? [...new Set([currentPubkey, ...contactPubkeys])]
        : contactPubkeys,
    [currentPubkey, contactPubkeys],
  );

  // ── Agents ─────────────────────────────────────────────────────────────
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = relayAgentsQuery.data ?? [];
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

  // ── "For You" combined pubkeys (contacts + agents + self) ──────────────
  const forYouPubkeys = React.useMemo(
    () => [...new Set([...peoplePubkeys, ...agentPubkeys])],
    [peoplePubkeys, agentPubkeys],
  );

  // ── Queries per tab ────────────────────────────────────────────────────
  const forYouQuery = useTimelineQuery(forYouPubkeys, activeTab === "foryou");
  const peopleQuery = useTimelineQuery(peoplePubkeys, activeTab === "people");
  const agentTimelineQuery = useTimelineQuery(
    agentFilter ? [agentFilter] : agentPubkeys,
    activeTab === "agents",
  );
  const myNotesQuery = useMyNotesQuery(
    activeTab === "mine" ? currentPubkey : undefined,
  );
  const publishMutation = usePublishNoteMutation(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);

  // ── Visible notes per tab ──────────────────────────────────────────────
  const visibleNotes: UserNote[] = React.useMemo(() => {
    if (activeTab === "foryou") {
      return forYouQuery.data?.notes ?? [];
    }
    if (activeTab === "people") {
      // Filter out agent notes from the people timeline.
      return (peopleQuery.data?.notes ?? []).filter(
        (n) => !agentPubkeySet.has(n.pubkey),
      );
    }
    if (activeTab === "agents") {
      return agentTimelineQuery.data?.notes ?? [];
    }
    return myNotesQuery.data?.notes ?? [];
  }, [
    activeTab,
    forYouQuery.data,
    peopleQuery.data,
    agentTimelineQuery.data,
    myNotesQuery.data,
    agentPubkeySet,
  ]);

  // Agent note groups for the agents tab.
  const agentNoteGroups = React.useMemo(
    () => (activeTab === "agents" ? groupAgentNotes(visibleNotes) : []),
    [activeTab, visibleNotes],
  );

  // ── Profile lookups ────────────────────────────────────────────────────
  const notePubkeys = React.useMemo(
    () => [...new Set(visibleNotes.map((n) => n.pubkey))],
    [visibleNotes],
  );
  const profilesQuery = useUsersBatchQuery(notePubkeys, {
    enabled: notePubkeys.length > 0,
  });
  const profiles: Record<string, UserProfileSummary> =
    profilesQuery.data?.profiles ?? {};

  // ── Mention members for ForumComposer ─────────────────────────────────
  const mentionProfilesQuery = useUsersBatchQuery(forYouPubkeys, {
    enabled: forYouPubkeys.length > 0,
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
    for (const pubkey of forYouPubkeys) {
      const profile = mentionProfiles[pubkey.toLowerCase()];
      members.push({
        pubkey,
        role: "member",
        joinedAt: "",
        displayName: profile?.displayName ?? null,
      });
    }
    return members;
  }, [forYouPubkeys, mentionProfiles]);

  // ── Loading / refresh state ────────────────────────────────────────────
  const activeQuery =
    activeTab === "foryou"
      ? forYouQuery
      : activeTab === "people"
        ? peopleQuery
        : activeTab === "agents"
          ? agentTimelineQuery
          : myNotesQuery;
  const isLoading = activeQuery.isLoading;

  function handleFollow(pubkey: string) {
    followMutation.mutate(pubkey);
  }

  function handleUnfollow(pubkey: string) {
    unfollowMutation.mutate(pubkey);
  }

  // ── Render ─────────────────────────────────────────────────────────────
  const emptyMessages: Record<PulseTab, string> = {
    search: "Search Pulse notes by author or text.",
    foryou:
      "No notes yet. Follow people and agents to build your personalized feed.",
    people: "No notes yet. Follow people to see their updates here.",
    agents:
      agentPubkeys.length === 0
        ? "No agents registered yet."
        : "No agent notes yet. Agents will post updates as they work.",
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
          currentUserDisplayName={currentDisplayName}
          currentUserProfile={currentProfile}
          isAgent={agentPubkeySet.has(note.pubkey)}
          isFollowing={followingSet.has(note.pubkey)}
          isOwnNote={note.pubkey === currentPubkey}
          key={note.id}
          note={note}
          onFollow={handleFollow}
          onUnfollow={handleUnfollow}
          profile={profiles[note.pubkey.toLowerCase()] ?? null}
        />
      ))
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Tabs
        value={activeTab}
        onValueChange={(v) => setActiveTab(v as PulseTab)}
        className="flex min-h-0 flex-1 flex-col overflow-hidden"
      >
        {/* Tab bar */}
        <div className="relative z-40 shrink-0 px-4 pt-2 sm:px-6">
          <div className="relative mx-auto flex w-full max-w-2xl items-center justify-center">
            <div className="flex items-center">
              <TabsList className="h-8 gap-0.5 rounded-full border border-border/50 bg-muted/40 p-0.5">
                <TabsTrigger
                  aria-label="Search Pulse"
                  value="search"
                  className="h-7 w-7 rounded-full p-0 !text-muted-foreground shadow-none transition-colors hover:!text-foreground data-[state=active]:bg-background data-[state=active]:!text-foreground data-[state=active]:shadow-sm dark:!text-white/35 dark:hover:!text-white/70 dark:data-[state=active]:!text-white"
                >
                  <Search className="h-4 w-4" />
                </TabsTrigger>
                <TabsTrigger value="foryou" className={tabTriggerClassName}>
                  Everyone
                </TabsTrigger>
                <TabsTrigger value="people" className={tabTriggerClassName}>
                  Following
                </TabsTrigger>
                <TabsTrigger value="agents" className={tabTriggerClassName}>
                  Agents
                  {relayAgents.length > 0 ? (
                    <span className="ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-muted px-1 text-[10px] font-medium text-muted-foreground dark:text-white/45">
                      {relayAgents.length}
                    </span>
                  ) : null}
                </TabsTrigger>
                <TabsTrigger value="mine" className={tabTriggerClassName}>
                  Mine
                </TabsTrigger>
              </TabsList>
            </div>

            <div className="absolute right-0 flex items-center gap-1">
              {activeTab === "agents" && relayAgents.length > 1 ? (
                <AgentFilter
                  agents={relayAgents}
                  onSelect={setAgentFilter}
                  profiles={profiles}
                  selectedPubkey={agentFilter}
                />
              ) : null}
            </div>
          </div>
        </div>

        {/* Timeline — TabsContent with forceMount so aria-controls resolves */}
        <TabsContent
          value={activeTab}
          forceMount
          className="mt-0 min-h-0 flex-1 overflow-y-auto"
        >
          <div
            className={`mx-auto flex w-full max-w-2xl flex-col px-4 pb-10 sm:px-6 ${
              activeTab !== "search" && activeTab !== "agents" ? "pt-0" : "pt-7"
            }`}
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
                        <Search className="h-3.5 w-3.5" />
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
        </TabsContent>
      </Tabs>
    </div>
  );
}
