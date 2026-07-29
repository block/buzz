import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

import { useQueryClient } from "@tanstack/react-query";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  invalidateChannelState,
  useAddChannelMembersMutation,
  useArchiveChannelMutation,
  useChannelMembersQuery,
  useLeaveChannelMutation,
  useUpdateChannelMutation,
} from "@/features/channels/hooks";
import {
  AUTHOR_COLOR_PALETTE,
  defaultAuthorColor,
  normalizeHexColor,
  setNameColorOverride,
  useAuthorColorResolver,
} from "@/features/dev-mode/lib/authorColors";
import { setDisplayStyle } from "@/features/dev-mode/lib/displayStylePreference";
import {
  toggleChannelPinned,
  usePinnedChannels,
} from "@/features/dev-mode/lib/pinnedChannels";
import {
  sanitizeChannelName,
  uniqueChannelName,
} from "@/features/dev-mode/lib/sessionNaming";
import {
  parseSubChannelName,
  subChannelName,
} from "@/features/dev-mode/lib/subChannels";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
} from "@/features/profile/hooks";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
import { joinChannel } from "@/shared/api/tauriChannels";
import type { Channel, UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

type PaletteEntry = {
  id: string;
  label: string;
  detail?: string;
  /** Swatch color for color-picker entries. */
  swatch?: string;
  run: () => void;
};

type PaletteMode = "root" | "color" | "add-member" | "members" | "rename";

/**
 * Channels can have 1000+ members; the browser renders at most this many
 * rows and tells the user to narrow by typing instead of scrolling.
 */
const MEMBERS_RENDER_CAP = 40;

function formatCandidateName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

const SETTINGS_ENTRIES: { section: SettingsSection; label: string }[] = [
  { section: "agents", label: "configure agents" },
  { section: "appearance", label: "appearance settings" },
  { section: "profile", label: "profile settings" },
  { section: "notifications", label: "notification settings" },
  { section: "shortcuts", label: "keyboard shortcuts" },
  { section: "experimental", label: "experimental features" },
  { section: "channel-templates", label: "channel templates" },
  { section: "compute", label: "compute settings" },
  { section: "updates", label: "check for updates" },
];

/**
 * Amp-style command palette for developer mode: channel search across every
 * session plus management/configuration actions. Opened with ⌘K anywhere
 * in the shell, or `/` in an empty composer.
 */
export function DevCommandPalette({
  channels,
  discoverableChannels,
  activeChannel,
  parentOfActive,
  myPubkey,
  initialMode = "root",
  onOpenChannel,
  onNewSession,
  onNewSubChannel,
  onChannelLeft,
  onClose,
}: {
  /** All session channels, newest first. */
  channels: Channel[];
  /** Open channels the user has not joined; searchable, enter joins. */
  discoverableChannels: Channel[];
  /** Channel that add-member/leave actions apply to (focused or previewed). */
  activeChannel: Channel | null;
  /** Set when the active channel is a `parent--sub` of an existing parent. */
  parentOfActive: Channel | null;
  myPubkey: string | null;
  /** Open directly into a sub-view, e.g. the top bar's member count. */
  initialMode?: "root" | "members";
  onOpenChannel: (channelId: string) => void;
  onNewSession: () => void;
  /** Starts a sub-channel draft in the open channel; null when unavailable. */
  onNewSubChannel: (() => void) | null;
  onChannelLeft: (channelId: string) => void;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [query, setQuery] = React.useState("");
  const [mode, setMode] = React.useState<PaletteMode>(initialMode);
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const [actionError, setActionError] = React.useState<string | null>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const activeChannelId = activeChannel?.id ?? null;
  const pinnedIds = usePinnedChannels();
  const addMembersMutation = useAddChannelMembersMutation(activeChannelId);
  const leaveMutation = useLeaveChannelMutation(activeChannelId);
  const archiveMutation = useArchiveChannelMutation(activeChannelId);
  const updateChannelMutation = useUpdateChannelMutation(activeChannelId);
  const membersQuery = useChannelMembersQuery(activeChannelId);
  const resolveColor = useAuthorColorResolver();

  // The relay refuses to remove a channel's last owner, so when no other
  // human would remain, "leave" archives the channel instead.
  const isLastHumanMember = React.useMemo(() => {
    const members = membersQuery.data;
    if (!members || !myPubkey) return false;
    const self = normalizePubkey(myPubkey);
    return !members.some(
      (member) =>
        normalizePubkey(member.pubkey) !== self &&
        !member.isAgent &&
        member.role !== "bot",
    );
  }, [membersQuery.data, myPubkey]);
  const managedAgentsQuery = useManagedAgentsQuery({
    enabled: mode === "add-member",
  });
  const userSearchQuery = useInfiniteUserSearchQuery(query, {
    enabled: mode === "add-member" && query.trim().length >= 2,
    limit: 20,
  });
  const userSearchResults = useFlattenedUserSearchResults(userSearchQuery.data);

  const openSettings = React.useCallback(
    (section: SettingsSection) => {
      onClose();
      void navigate({ to: "/settings", search: { section } });
    },
    [navigate, onClose],
  );

  const addUserToChannel = React.useCallback(
    async (user: UserSearchResult) => {
      if (!activeChannelId) return;
      setActionError(null);
      try {
        // Local managed agents need a running harness pair, not just channel
        // membership (see MembersSidebar) — route them through attach.
        const managedAgent = (managedAgentsQuery.data ?? []).find(
          (agent) =>
            normalizePubkey(agent.pubkey) === normalizePubkey(user.pubkey),
        );
        if (managedAgent?.backend.type === "local") {
          await attachManagedAgentToChannel(activeChannelId, {
            agent: managedAgent,
            ensureRunning: true,
          });
          await invalidateChannelState(queryClient, activeChannelId);
        } else {
          const result = await addMembersMutation.mutateAsync({
            pubkeys: [user.pubkey],
            role: user.isAgent ? "bot" : "member",
          });
          if (result.errors.length > 0) {
            setActionError(result.errors[0]?.error ?? "Failed to add member.");
            return;
          }
        }
        onClose();
      } catch (error) {
        setActionError(
          error instanceof Error ? error.message : "Failed to add member.",
        );
      }
    },
    [
      activeChannelId,
      addMembersMutation,
      managedAgentsQuery.data,
      onClose,
      queryClient,
    ],
  );

  const leaveChannel = React.useCallback(async () => {
    if (!activeChannelId) return;
    setActionError(null);
    try {
      if (isLastHumanMember) {
        await archiveMutation.mutateAsync();
      } else {
        await leaveMutation.mutateAsync();
      }
      onClose();
      onChannelLeft(activeChannelId);
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to leave channel.",
      );
    }
  }, [
    activeChannelId,
    archiveMutation,
    isLastHumanMember,
    leaveMutation,
    onChannelLeft,
    onClose,
  ]);

  // Renaming a sub-channel only edits its suffix (the `parent--` prefix is
  // the parent link); renaming a main cascades to its subs inside
  // useUpdateChannelMutation.
  const renameChannel = React.useCallback(
    async (rawName: string) => {
      if (!activeChannel) return;
      const slug = sanitizeChannelName(rawName);
      if (!slug) return;
      const base = parentOfActive
        ? subChannelName(parentOfActive.name, slug)
        : slug;
      const otherNames = new Set(
        channels
          .filter((channel) => channel.id !== activeChannel.id)
          .map((channel) => channel.name),
      );
      const newName = uniqueChannelName(base, otherNames);
      if (newName === activeChannel.name) {
        onClose();
        return;
      }
      setActionError(null);
      try {
        await updateChannelMutation.mutateAsync({ name: newName });
        // The mutation invalidates with refetchType "none"; the mounted
        // channel list must refetch now or cascaded sub renames stay stale
        // and the family's tabs fall apart.
        await invalidateChannelState(queryClient, activeChannel.id);
        onClose();
      } catch (error) {
        setActionError(
          error instanceof Error ? error.message : "Failed to rename channel.",
        );
      }
    },
    [
      activeChannel,
      channels,
      onClose,
      parentOfActive,
      queryClient,
      updateChannelMutation,
    ],
  );

  const joinAndOpenChannel = React.useCallback(
    async (channelId: string) => {
      setActionError(null);
      try {
        await joinChannel(channelId);
        await invalidateChannelState(queryClient, channelId);
        onOpenChannel(channelId);
        onClose();
      } catch (error) {
        setActionError(
          error instanceof Error ? error.message : "Failed to join channel.",
        );
      }
    },
    [onClose, onOpenChannel, queryClient],
  );

  const archiveChannel = React.useCallback(async () => {
    if (!activeChannelId) return;
    setActionError(null);
    try {
      await archiveMutation.mutateAsync();
      onClose();
      onChannelLeft(activeChannelId);
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to archive channel.",
      );
    }
  }, [activeChannelId, archiveMutation, onChannelLeft, onClose]);

  const entries = React.useMemo<PaletteEntry[]>(() => {
    const needle = query.trim().toLowerCase();

    if (mode === "color") {
      const colorEntries: PaletteEntry[] = AUTHOR_COLOR_PALETTE.map(
        (color) => ({
          id: `color-${color}`,
          label: color,
          swatch: color,
          run: () => {
            if (myPubkey) setNameColorOverride(myPubkey, color);
            onClose();
          },
        }),
      );
      const typed = normalizeHexColor(needle);
      if (typed) {
        colorEntries.unshift({
          id: "color-custom",
          label: `use ${typed}`,
          swatch: typed,
          run: () => {
            if (myPubkey) setNameColorOverride(myPubkey, typed);
            onClose();
          },
        });
      }
      colorEntries.push({
        id: "color-reset",
        label: "reset to default",
        swatch: myPubkey ? defaultAuthorColor(myPubkey) : undefined,
        run: () => {
          if (myPubkey) setNameColorOverride(myPubkey, null);
          onClose();
        },
      });
      return typed
        ? colorEntries
        : colorEntries.filter((entry) =>
            entry.label.toLowerCase().includes(needle),
          );
    }

    if (mode === "rename") {
      if (!activeChannel) return [];
      const slug = sanitizeChannelName(needle);
      if (!slug) {
        return [
          {
            id: "rename-hint",
            label: "type a new name…",
            detail: `renaming # ${activeChannel.name}`,
            run: () => {},
          },
        ];
      }
      const preview = parentOfActive
        ? subChannelName(parentOfActive.name, slug)
        : slug;
      return [
        {
          id: "rename-apply",
          label: `rename to # ${preview}`,
          detail: `was # ${activeChannel.name}`,
          run: () => void renameChannel(needle),
        },
      ];
    }

    if (mode === "members") {
      const members = membersQuery.data ?? [];
      const matched = needle
        ? members.filter((member) =>
            `${member.displayName ?? ""} ${member.pubkey}`
              .toLowerCase()
              .includes(needle),
          )
        : members;
      const memberEntries: PaletteEntry[] = matched
        .slice(0, MEMBERS_RENDER_CAP)
        .map((member) => ({
          id: `member-${member.pubkey}`,
          label: member.displayName || truncatePubkey(member.pubkey),
          detail:
            member.isAgent || member.role === "bot"
              ? "agent"
              : member.role !== "member"
                ? member.role
                : truncatePubkey(member.pubkey),
          swatch: resolveColor(member.pubkey),
          run: () => {},
        }));
      if (matched.length > MEMBERS_RENDER_CAP) {
        memberEntries.push({
          id: "members-overflow",
          label: `… ${matched.length - MEMBERS_RENDER_CAP} more — type to narrow`,
          run: () => {},
        });
      }
      return memberEntries;
    }

    if (mode === "add-member") {
      const memberPubkeys = new Set(
        (membersQuery.data ?? []).map((member) =>
          normalizePubkey(member.pubkey),
        ),
      );
      // Sub-channel invariant: only parent members may join `parent--sub`.
      const parentMemberPubkeys = parentOfActive
        ? new Set(
            parentOfActive.memberPubkeys.map((pubkey) =>
              normalizePubkey(pubkey),
            ),
          )
        : null;
      return userSearchResults
        .filter(
          (user) =>
            !memberPubkeys.has(normalizePubkey(user.pubkey)) &&
            normalizePubkey(user.pubkey) !== normalizePubkey(myPubkey ?? "") &&
            (parentMemberPubkeys?.has(normalizePubkey(user.pubkey)) ?? true),
        )
        .map((user) => ({
          id: `add-${user.pubkey}`,
          label: formatCandidateName(user),
          detail: user.isAgent
            ? "agent"
            : (user.nip05Handle ?? truncatePubkey(user.pubkey)),
          run: () => void addUserToChannel(user),
        }));
    }

    const channelActions: PaletteEntry[] = activeChannel
      ? [
          {
            id: "view-members",
            label: `view members of # ${activeChannel.name}`,
            detail: `${activeChannel.memberCount} ${
              activeChannel.memberCount === 1 ? "member" : "members"
            }`,
            run: () => {
              setMode("members");
              setQuery("");
              setSelectedIndex(0);
            },
          },
          {
            id: "add-member",
            label: `add someone to # ${activeChannel.name}`,
            detail: parentOfActive
              ? `members of # ${parentOfActive.name} only`
              : "people & agents",
            run: () => {
              setMode("add-member");
              setQuery("");
              setSelectedIndex(0);
            },
          },
          ...(onNewSubChannel
            ? [
                {
                  id: "new-sub-channel",
                  label: `new tab in # ${parentOfActive?.name ?? activeChannel.name}`,
                  detail: "spawn a focused agent session · ⌘⇧T",
                  run: () => {
                    onNewSubChannel();
                    onClose();
                  },
                } satisfies PaletteEntry,
              ]
            : []),
          {
            id: "rename-channel",
            label: `rename # ${activeChannel.name}`,
            detail: parentOfActive
              ? "rename this tab"
              : "its tabs are renamed with it",
            run: () => {
              setMode("rename");
              setQuery("");
              setSelectedIndex(0);
            },
          },
          // Subs never appear in the left list, so pinning one is meaningless.
          ...(parentOfActive
            ? []
            : [
                {
                  id: "pin-channel",
                  label: pinnedIds.has(activeChannel.id)
                    ? `unpin # ${activeChannel.name}`
                    : `pin # ${activeChannel.name}`,
                  detail: pinnedIds.has(activeChannel.id)
                    ? "remove from pinned section"
                    : "keep at the top of the channel list",
                  run: () => {
                    toggleChannelPinned(activeChannel.id);
                    onClose();
                  },
                } satisfies PaletteEntry,
              ]),
          {
            id: "leave-channel",
            label: `leave # ${activeChannel.name}`,
            detail: isLastHumanMember
              ? "archives — you're the last member"
              : "remove yourself",
            run: () => void leaveChannel(),
          },
          {
            id: "archive-channel",
            label: `archive # ${activeChannel.name}`,
            detail: "hide from the channel list",
            run: () => void archiveChannel(),
          },
        ]
      : [];

    const actions: PaletteEntry[] = [
      ...channelActions,
      {
        id: "new-session",
        label: "new session",
        detail: "fresh prompt",
        run: () => {
          onNewSession();
          onClose();
        },
      },
      {
        id: "standard-ui",
        label: "switch to standard ui",
        detail: "⌘⇧D",
        run: () => {
          onClose();
          setDisplayStyle("standard");
        },
      },
      {
        id: "name-color",
        label: "set my name color",
        detail: "hex or preset",
        run: () => {
          setMode("color");
          setQuery("");
          setSelectedIndex(0);
        },
      },
      ...SETTINGS_ENTRIES.map(
        (entry): PaletteEntry => ({
          id: `settings-${entry.section}`,
          label: entry.label,
          detail: "settings",
          run: () => openSettings(entry.section),
        }),
      ),
    ];

    const channelEntries: PaletteEntry[] = channels.map((channel) => ({
      id: `channel-${channel.id}`,
      label: `# ${channel.name}`,
      detail: channel.description ?? undefined,
      run: () => {
        onOpenChannel(channel.id);
        onClose();
      },
    }));

    if (!needle) return [...actions, ...channelEntries];

    // Open channels the user hasn't joined only surface while searching —
    // a relay can have hundreds and they'd drown the root list. Joining a
    // `parent--sub` requires parent membership, so foreign subs are hidden.
    const joinedNames = new Set(channels.map((channel) => channel.name));
    const joinableEntries: PaletteEntry[] = discoverableChannels
      .filter((channel) => {
        const parsed = parseSubChannelName(channel.name);
        return !parsed || joinedNames.has(parsed.parentName);
      })
      .map((channel) => ({
        id: `join-${channel.id}`,
        label: `# ${channel.name}`,
        detail: "not joined · enter to join",
        run: () => void joinAndOpenChannel(channel.id),
      }));

    const matches = (entry: PaletteEntry) =>
      `${entry.label} ${entry.detail ?? ""}`.toLowerCase().includes(needle);
    const matchesName = (entry: PaletteEntry) =>
      entry.label.toLowerCase().includes(needle);
    // An action verb typed literally ("archive", "leave", "pin"…) beats
    // channel-name substring hits; otherwise a query is usually a channel
    // lookup, so channels rank above incidental action matches. Joined
    // channels rank above joinable ones, which match on name only so
    // typing "join" doesn't dump every discoverable channel.
    const matchedActions = actions.filter(matches);
    const startsWithNeedle = (entry: PaletteEntry) =>
      entry.label.toLowerCase().startsWith(needle);
    return [
      ...matchedActions.filter(startsWithNeedle),
      ...channelEntries.filter(matches),
      ...joinableEntries.filter(matchesName),
      ...matchedActions.filter((entry) => !startsWithNeedle(entry)),
    ];
  }, [
    activeChannel,
    addUserToChannel,
    archiveChannel,
    channels,
    discoverableChannels,
    isLastHumanMember,
    joinAndOpenChannel,
    leaveChannel,
    membersQuery.data,
    mode,
    myPubkey,
    onClose,
    onNewSession,
    onNewSubChannel,
    onOpenChannel,
    openSettings,
    parentOfActive,
    pinnedIds,
    query,
    renameChannel,
    resolveColor,
    userSearchResults,
  ]);

  const clampedIndex = Math.min(selectedIndex, Math.max(0, entries.length - 1));

  const scrollSelectedIntoView = React.useCallback(
    (node: HTMLButtonElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (mode !== "root") {
        setMode("root");
        setQuery("");
        setSelectedIndex(0);
        setActionError(null);
        return;
      }
      onClose();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex(Math.min(clampedIndex + 1, entries.length - 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex(Math.max(clampedIndex - 1, 0));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      entries[clampedIndex]?.run();
    }
  };

  return (
    <div
      className="absolute inset-0 z-50 flex items-start justify-center pt-24 font-mono"
      data-testid="dev-mode-palette"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-background/60"
        onClick={onClose}
      />
      <div className="relative flex max-h-[60vh] w-[560px] flex-col border border-border bg-background shadow-lg">
        <input
          ref={inputRef}
          className="shrink-0 border-b border-border/60 bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground/60"
          data-testid="dev-mode-palette-input"
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
          }}
          onKeyDown={handleKeyDown}
          placeholder={
            mode === "color"
              ? "type a hex color or pick a preset…"
              : mode === "add-member"
                ? `search people & agents to add to # ${activeChannel?.name ?? ""}…`
                : mode === "members"
                  ? `search members of # ${activeChannel?.name ?? ""}…`
                  : mode === "rename"
                    ? `new name for # ${activeChannel?.name ?? ""}…`
                    : "search channels and commands…"
          }
          spellCheck={false}
          value={query}
        />
        {actionError ? (
          <div className="shrink-0 border-b border-destructive/40 bg-destructive/10 px-3 py-1.5 text-xs text-destructive">
            {actionError}
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {entries.length === 0 ? (
            <div className="px-3 py-2 text-sm text-muted-foreground/60">
              {mode === "add-member"
                ? query.trim().length < 2
                  ? "type at least 2 characters to search…"
                  : userSearchQuery.isFetching
                    ? "searching…"
                    : "no matches"
                : mode === "members" && membersQuery.isLoading
                  ? "loading members…"
                  : "no matches"}
            </div>
          ) : null}
          {entries.map((entry, index) => (
            <button
              key={entry.id}
              ref={index === clampedIndex ? scrollSelectedIntoView : undefined}
              className={cn(
                "flex w-full cursor-pointer items-baseline gap-2 px-3 py-1 text-left text-sm",
                index === clampedIndex
                  ? "bg-primary/15 text-foreground"
                  : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
              )}
              data-testid="dev-mode-palette-entry"
              onClick={entry.run}
              onMouseMove={() => setSelectedIndex(index)}
              type="button"
            >
              {entry.swatch ? (
                <span
                  aria-hidden
                  className="inline-block size-3 shrink-0 self-center border border-border/60"
                  style={{ backgroundColor: entry.swatch }}
                />
              ) : null}
              <span className="min-w-0 flex-1 truncate">{entry.label}</span>
              {entry.detail ? (
                <span className="max-w-48 shrink-0 truncate text-xs text-muted-foreground/60">
                  {entry.detail}
                </span>
              ) : null}
            </button>
          ))}
        </div>
        <div className="shrink-0 border-t border-border/60 px-3 py-1.5 text-xs text-muted-foreground/60">
          ↑↓: select · enter: run · esc: {mode === "root" ? "close" : "back"}
        </div>
      </div>
    </div>
  );
}
