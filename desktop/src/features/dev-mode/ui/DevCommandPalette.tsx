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
} from "@/features/channels/hooks";
import {
  AUTHOR_COLOR_PALETTE,
  defaultAuthorColor,
  normalizeHexColor,
  setNameColorOverride,
} from "@/features/dev-mode/lib/authorColors";
import { setDisplayStyle } from "@/features/dev-mode/lib/displayStylePreference";
import {
  toggleChannelPinned,
  usePinnedChannels,
} from "@/features/dev-mode/lib/pinnedChannels";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
} from "@/features/profile/hooks";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
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

type PaletteMode = "root" | "color" | "add-member";

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
 * session plus management/configuration actions. Opened with Ctrl+O anywhere
 * in the shell, or `/` in an empty composer.
 */
export function DevCommandPalette({
  channels,
  activeChannel,
  myPubkey,
  onOpenChannel,
  onNewSession,
  onChannelLeft,
  onClose,
}: {
  /** All session channels, newest first. */
  channels: Channel[];
  /** Channel that add-member/leave actions apply to (focused or previewed). */
  activeChannel: Channel | null;
  myPubkey: string | null;
  onOpenChannel: (channelId: string) => void;
  onNewSession: () => void;
  onChannelLeft: () => void;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [query, setQuery] = React.useState("");
  const [mode, setMode] = React.useState<PaletteMode>("root");
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
  const membersQuery = useChannelMembersQuery(activeChannelId);

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
    setActionError(null);
    try {
      if (isLastHumanMember) {
        await archiveMutation.mutateAsync();
      } else {
        await leaveMutation.mutateAsync();
      }
      onClose();
      onChannelLeft();
    } catch (error) {
      setActionError(
        error instanceof Error ? error.message : "Failed to leave channel.",
      );
    }
  }, [
    archiveMutation,
    isLastHumanMember,
    leaveMutation,
    onChannelLeft,
    onClose,
  ]);

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

    if (mode === "add-member") {
      const memberPubkeys = new Set(
        (membersQuery.data ?? []).map((member) =>
          normalizePubkey(member.pubkey),
        ),
      );
      return userSearchResults
        .filter(
          (user) =>
            !memberPubkeys.has(normalizePubkey(user.pubkey)) &&
            normalizePubkey(user.pubkey) !== normalizePubkey(myPubkey ?? ""),
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
            id: "add-member",
            label: `add someone to # ${activeChannel.name}`,
            detail: "people & agents",
            run: () => {
              setMode("add-member");
              setQuery("");
              setSelectedIndex(0);
            },
          },
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
          },
          {
            id: "leave-channel",
            label: `leave # ${activeChannel.name}`,
            detail: isLastHumanMember
              ? "archives — you're the last member"
              : "remove yourself",
            run: () => void leaveChannel(),
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
    const matches = (entry: PaletteEntry) =>
      `${entry.label} ${entry.detail ?? ""}`.toLowerCase().includes(needle);
    // Channels first while typing: a query is usually a channel lookup.
    return [...channelEntries.filter(matches), ...actions.filter(matches)];
  }, [
    activeChannel,
    addUserToChannel,
    channels,
    isLastHumanMember,
    leaveChannel,
    membersQuery.data,
    mode,
    myPubkey,
    onClose,
    onNewSession,
    onOpenChannel,
    openSettings,
    pinnedIds,
    query,
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
