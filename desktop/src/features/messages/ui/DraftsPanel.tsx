import { FileText, Lock, Pencil, Send, Trash2 } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  clearDraftEntry,
  getActiveDraftEntries,
  renameDraftEntry,
  useDraftsSnapshot,
  type DraftState,
} from "@/features/messages/lib/useDrafts";
import {
  useDraftRootStatus,
  type RootStatus,
} from "@/features/messages/lib/useDraftRootStatus";
import {
  migrateInboxReplyDraft,
  INBOX_REPLY_PREFIX,
} from "@/features/messages/lib/inboxReplyMigration";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getEventById } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const SENT_DRAFT_PREFIX = "sent:";
const THREAD_DRAFT_PREFIX = "thread:";
const UNKNOWN_CHANNEL_LABEL = "Unknown channel";

type DraftListEntry = {
  draft: DraftState;
  key: string;
};

type DraftSection = {
  entries: DraftListEntry[];
  label: string;
  status: DraftState["status"];
};

type DraftSource = {
  channel: Channel | null;
  label: string;
};

const UNKNOWN_DRAFT_SOURCE: DraftSource = {
  channel: null,
  label: UNKNOWN_CHANNEL_LABEL,
};

const draftTimeFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

function parseDraftTime(value: string): number {
  const time = new Date(value).getTime();
  return Number.isFinite(time) ? time : 0;
}

function formatDraftCreatedAt(draft: DraftState): string {
  const time = parseDraftTime(draft.createdAt);
  return time === 0
    ? "Unknown time"
    : draftTimeFormatter.format(new Date(time));
}

function getOriginalDraftKey(draftKey: string): string {
  if (!draftKey.startsWith(SENT_DRAFT_PREFIX)) {
    return draftKey;
  }

  const sentPayload = draftKey.slice(SENT_DRAFT_PREFIX.length);
  const timestampSeparatorIndex = sentPayload.lastIndexOf(":");
  return timestampSeparatorIndex > 0
    ? sentPayload.slice(0, timestampSeparatorIndex)
    : sentPayload;
}

function getThreadRootId(draftKey: string): string | null {
  const originalDraftKey = getOriginalDraftKey(draftKey);
  if (!originalDraftKey.startsWith(THREAD_DRAFT_PREFIX)) {
    return null;
  }

  const id = originalDraftKey.slice(THREAD_DRAFT_PREFIX.length).trim();
  return id.length > 0 ? id : null;
}

function isVisibleDraft(entry: DraftListEntry): boolean {
  const content = entry.draft.content.trim();
  const attachmentCount = entry.draft.pendingImeta.length;
  return content.length > 0 || attachmentCount > 0;
}

function getDraftPreview(draft: DraftState): string {
  const content = draft.content.trim();
  if (content.length > 0) {
    return content;
  }

  const attachmentCount = draft.pendingImeta.length;
  if (attachmentCount === 1) {
    return "1 attachment";
  }
  if (attachmentCount > 1) {
    return `${attachmentCount} attachments`;
  }
  return "Empty draft";
}

function readDraftSections(): DraftSection[] {
  const active = getActiveDraftEntries().filter(isVisibleDraft);
  const sections: DraftSection[] = [];

  if (active.length > 0) {
    sections.push({ label: "Drafts", status: "active", entries: active });
  }

  return sections;
}

function resolveDraftSources({
  channels,
  currentPubkey,
  drafts,
  profiles,
}: {
  channels: Channel[] | undefined;
  currentPubkey: string | undefined;
  drafts: DraftListEntry[];
  profiles: UserProfileLookup | undefined;
}): Map<string, DraftSource> {
  const channelsById = new Map(
    (channels ?? []).map((channel) => [channel.id, channel]),
  );
  const sources = new Map<string, DraftSource>();

  for (const entry of drafts) {
    const channel = channelsById.get(entry.draft.channelId);
    sources.set(entry.key, {
      channel: channel ?? null,
      label: channel
        ? resolveChannelDisplayLabel(channel, currentPubkey, profiles)
        : UNKNOWN_CHANNEL_LABEL,
    });
  }

  return sources;
}

function DraftRowActionButton({
  children,
  disabled = false,
  label,
  onClick,
}: {
  children: React.ReactNode;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className="h-7 w-7 rounded-full p-0 text-muted-foreground hover:text-foreground"
          disabled={disabled}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!disabled) {
              onClick();
            }
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

export function canOpenDraft(draft: DraftState, source: DraftSource): boolean {
  return (
    draft.status !== "sent" &&
    source.channel !== null &&
    draft.channelId.length > 0
  );
}

/** True when a draft is sendable: same conditions as openable, plus root not deleted. */
export function canSendDraft(
  draft: DraftState,
  source: DraftSource,
  rootStatus: RootStatus,
): boolean {
  if (!canOpenDraft(draft, source)) return false;
  if (rootStatus === "deleted") return false;
  // Mirror the destination composer's stable disabled states so we never
  // offer a Send that will silently no-op:
  //   - not a member: the composer rejects sends
  //   - archived: read-only
  //   - forum: forum posting is not wired
  // `isSending` is transient runtime state not visible from the panel — omit.
  const ch = source.channel;
  if (ch === null) return false;
  if (!ch.isMember) return false;
  if (ch.archivedAt !== null) return false;
  if (ch.channelType === "forum") return false;
  return true;
}

// ── Send confirmation dialog ──────────────────────────────────────────────────

type SendConfirmDialogProps = {
  channelLabel: string;
  isDm: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  open: boolean;
};

function SendConfirmDialog({
  channelLabel,
  isDm,
  onCancel,
  onConfirm,
  open,
}: SendConfirmDialogProps) {
  const destination = isDm ? channelLabel : `#${channelLabel}`;
  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onCancel();
        }
      }}
      open={open}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Send message</AlertDialogTitle>
          <AlertDialogDescription>
            Are you sure you want to send this message to {destination}?
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button onClick={onCancel} size="sm" type="button" variant="outline">
            Cancel
          </Button>
          <Button onClick={onConfirm} size="sm" type="button">
            Send
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

// ── DraftRow ─────────────────────────────────────────────────────────────────

function DraftRow({
  entry,
  onDelete,
  onOpen,
  onSend,
  rootStatus,
  source,
}: {
  entry: DraftListEntry;
  onDelete: (draftKey: string) => void;
  onOpen: (entry: DraftListEntry) => void;
  onSend: (entry: DraftListEntry) => void;
  rootStatus: RootStatus;
  source: DraftSource;
}) {
  const isSent = entry.draft.status === "sent";
  const isOrphaned = rootStatus === "deleted";
  const canOpen = canOpenDraft(entry.draft, source) && !isOrphaned;
  const canSend = canSendDraft(entry.draft, source, rootStatus);
  const isPrivate = source.channel?.visibility === "private";
  const isDm = source.channel?.channelType === "dm";
  const channelLabel = source.channel
    ? isDm
      ? source.label
      : `#${source.label}`
    : UNKNOWN_CHANNEL_LABEL;

  return (
    <div
      className={cn(
        "group/draft-row relative rounded-md border border-border/70 bg-background transition-colors hover:bg-muted/40 focus-within:bg-muted/40",
        isOrphaned && "opacity-50",
      )}
      data-testid={`home-draft-item-${entry.key}`}
    >
      <button
        aria-label={`Open draft in ${channelLabel}`}
        className="block w-full min-w-0 px-3 py-3 text-left disabled:cursor-default"
        disabled={!canOpen}
        onClick={() => onOpen(entry)}
        type="button"
      >
        <div className="min-w-0 pr-0 transition-[padding] group-hover/draft-row:pr-20 group-focus-within/draft-row:pr-20">
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
            {isPrivate ? <Lock className="h-3.5 w-3.5 shrink-0" /> : null}
            <span
              className={cn(
                "truncate font-medium",
                source.channel ? "text-foreground" : "text-muted-foreground",
                isOrphaned && "text-muted-foreground",
              )}
            >
              {channelLabel}
            </span>
            <span className="shrink-0 text-muted-foreground/70">
              {formatDraftCreatedAt(entry.draft)}
            </span>
            {isOrphaned ? (
              <span
                className="shrink-0 rounded px-1 py-0.5 text-2xs font-medium text-destructive/70 ring-1 ring-destructive/30"
                data-testid={`home-draft-orphaned-label-${entry.key}`}
              >
                thread deleted
              </span>
            ) : null}
          </div>
          <div className="mt-1 max-h-10 overflow-hidden text-sm font-medium leading-5 text-foreground">
            <Markdown
              className="inbox-preview-markdown text-inherit leading-5"
              content={getDraftPreview(entry.draft)}
              interactive={false}
            />
          </div>
        </div>
      </button>

      {/* Hover action buttons: edit / delete / send (order per spec) */}
      <div className="pointer-events-none absolute right-2 top-2 flex items-center gap-0.5 rounded-full bg-background/95 p-0.5 opacity-0 shadow-xs ring-1 ring-border/70 transition-opacity group-hover/draft-row:pointer-events-auto group-hover/draft-row:opacity-100 group-focus-within/draft-row:pointer-events-auto group-focus-within/draft-row:opacity-100">
        {isSent ? null : (
          <>
            <DraftRowActionButton
              disabled={!canOpen}
              label={
                canOpen
                  ? "Open draft"
                  : isOrphaned
                    ? "Thread deleted"
                    : "No channel link"
              }
              onClick={() => onOpen(entry)}
            >
              <Pencil className="h-4 w-4" />
            </DraftRowActionButton>
            <DraftRowActionButton
              disabled={!canSend}
              label={
                canSend
                  ? "Send message"
                  : isOrphaned
                    ? "Thread deleted"
                    : "No channel link"
              }
              onClick={() => onSend(entry)}
            >
              <Send className="h-4 w-4" />
            </DraftRowActionButton>
          </>
        )}
        <DraftRowActionButton
          label="Delete draft"
          onClick={() => onDelete(entry.key)}
        >
          <Trash2 className="h-4 w-4" />
        </DraftRowActionButton>
      </div>
    </div>
  );
}

// ── Shared derivation: active draft count ────────────────────────────────────
// The badge (InboxListPane/HomeView) and the panel both call
// `deriveActiveDraftCount` with the same arguments so the derivation logic
// cannot diverge. Note: the badge call-site feeds an empty rootStatusMap
// (panel closed → queries disabled), so orphaned thread drafts count
// optimistically until the panel opens and the relay confirms deletion.
// This bounded eventual-consistency is the sanctioned design (Will, 2026-07-07).

/**
 * Derives the active (non-orphaned) draft count from the draft store snapshot
 * and root-status map. A draft is excluded from the count when its thread root
 * is definitively deleted (`"deleted"` status).
 *
 * @param activeDrafts  Active draft entries from `getActiveDraftEntries()`.
 * @param rootStatusMap Root-status map from `useDraftRootStatus()`.
 * @returns             Count of active drafts whose root is NOT deleted.
 */
export function deriveActiveDraftCount(
  activeDrafts: Array<{ key: string; draft: DraftState }>,
  rootStatusMap: Map<string, RootStatus>,
): number {
  return activeDrafts.filter((entry) => {
    const threadRootId = getThreadRootId(entry.key);
    if (threadRootId === null) {
      // Channel-root draft — cannot be orphaned.
      return true;
    }
    const status = rootStatusMap.get(threadRootId) ?? "checking";
    // Exclude only on definitive `deleted`; `checking`/`error` are optimistic.
    return status !== "deleted";
  }).length;
}

/**
 * Reactive hook for the active (non-orphaned) draft count.
 *
 * Used by `HomeView` to thread the count to `InboxListPane` for the badge.
 * Uses the same `deriveActiveDraftCount` function as the panel so the
 * derivation logic cannot diverge.
 *
 * When the panel is closed, `rootStatusMap` is empty (queries disabled), so
 * thread-reply drafts are counted optimistically — orphaned roots only drop
 * out of the count once the panel opens and the relay lookups complete.
 * This bounded eventual-consistency is product-approved (Will, 2026-07-07).
 */
export function useActiveDraftCount(
  rootStatusMap: Map<string, RootStatus>,
): number {
  // Re-render on every draft write via useDraftsSnapshot.
  useDraftsSnapshot();
  const activeDrafts = getActiveDraftEntries().filter(isVisibleDraft);
  return deriveActiveDraftCount(activeDrafts, rootStatusMap);
}

// ── DraftsPanel ──────────────────────────────────────────────────────────────

export function DraftsPanel() {
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();

  // Collapse the old `sections` state + `refreshDrafts` pattern onto a
  // reactive snapshot: every draft write re-renders via useSyncExternalStore.
  useDraftsSnapshot();
  const sections = readDraftSections();

  const drafts = React.useMemo(
    () => sections.flatMap((section) => section.entries),
    [sections],
  );

  // Collect unique thread-root IDs from active drafts only (sent drafts cannot
  // be sent/orphaned). Deduplicated on root id — multiple drafts can share one.
  const threadRootIds = React.useMemo(() => {
    const ids = new Set<string>();
    for (const entry of sections.flatMap((s) =>
      s.status === "active" ? s.entries : [],
    )) {
      const rootId = getThreadRootId(entry.key);
      if (rootId) {
        ids.add(rootId);
      }
    }
    return [...ids];
  }, [sections]);

  // Panel is always mounted when visible; `isOpen=true` enables root queries.
  const rootStatusMap = useDraftRootStatus(threadRootIds, true);

  const profilePubkeys = React.useMemo(
    () => [
      ...new Set(
        (channelsQuery.data ?? [])
          .filter((channel) =>
            drafts.some((entry) => entry.draft.channelId === channel.id),
          )
          .flatMap((channel) => channel.participantPubkeys),
      ),
    ],
    [channelsQuery.data, drafts],
  );
  const usersBatchQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });
  const profiles = usersBatchQuery.data?.profiles;

  const sources = React.useMemo(
    () =>
      resolveDraftSources({
        channels: channelsQuery.data,
        currentPubkey,
        drafts,
        profiles,
      }),
    [channelsQuery.data, currentPubkey, drafts, profiles],
  );

  // Send confirmation dialog state.
  const [sendTarget, setSendTarget] = React.useState<DraftListEntry | null>(
    null,
  );

  const handleOpen = React.useCallback(
    (entry: DraftListEntry) => {
      if (!entry.draft.channelId) {
        return;
      }

      // Legacy inbox-reply:<eventId> keys were written by the old InboxDetailPane
      // before stable conversation IDs were introduced. Migrate them on open:
      // resolve the embedded event to derive the thread root, verify the channel
      // tag matches the stored channelId, re-key to thread:<conversationId>, and
      // navigate to the correct thread composer with the migrated content.
      if (entry.key.startsWith(INBOX_REPLY_PREFIX)) {
        void (async () => {
          const migrated = await migrateInboxReplyDraft(
            entry.key,
            entry.draft,
            {
              getEventById,
              getChannelIdFromTags,
              getThreadReference,
              renameDraftEntry,
            },
          );
          // Navigate only on successful migration; any failure leaves the
          // legacy draft untouched and the user in place.
          if (!migrated) return;
          void goChannel(migrated.channelId, {
            messageId: migrated.conversationId,
            threadRootId: migrated.conversationId,
          });
        })();
        return;
      }

      const threadRootId = getThreadRootId(entry.key);
      void goChannel(
        entry.draft.channelId,
        threadRootId ? { messageId: threadRootId, threadRootId } : undefined,
      );
    },
    [goChannel],
  );

  const handleDelete = React.useCallback((draftKey: string) => {
    clearDraftEntry(draftKey);
    // No manual refresh needed — clearDraftEntry notifies subscribers and
    // useDraftsSnapshot() causes DraftsPanel to re-render automatically.
  }, []);

  const handleSendRequest = React.useCallback((entry: DraftListEntry) => {
    setSendTarget(entry);
  }, []);

  const handleSendCancel = React.useCallback(() => {
    setSendTarget(null);
  }, []);

  const handleSendConfirm = React.useCallback(() => {
    if (!sendTarget) return;
    const entry = sendTarget;
    setSendTarget(null);

    if (!entry.draft.channelId) return;

    // Legacy inbox-reply: keys: migrate first, then auto-send with new key.
    if (entry.key.startsWith(INBOX_REPLY_PREFIX)) {
      void (async () => {
        const migrated = await migrateInboxReplyDraft(entry.key, entry.draft, {
          getEventById,
          getChannelIdFromTags,
          getThreadReference,
          renameDraftEntry,
        });
        if (!migrated) return;
        void goChannel(migrated.channelId, {
          messageId: migrated.conversationId,
          threadRootId: migrated.conversationId,
          autoSend: migrated.newDraftKey,
        });
      })();
      return;
    }

    const threadRootId = getThreadRootId(entry.key);
    void goChannel(entry.draft.channelId, {
      ...(threadRootId ? { messageId: threadRootId, threadRootId } : {}),
      autoSend: entry.key,
    });
  }, [sendTarget, goChannel]);

  const sendDialogSource = sendTarget
    ? (sources.get(sendTarget.key) ?? UNKNOWN_DRAFT_SOURCE)
    : UNKNOWN_DRAFT_SOURCE;
  const sendDialogIsDm = sendDialogSource.channel?.channelType === "dm";
  const sendDialogChannelLabel = sendDialogSource.channel
    ? sendDialogSource.label
    : UNKNOWN_CHANNEL_LABEL;

  if (sections.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
        <FileText className="h-8 w-8 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">No drafts</p>
      </div>
    );
  }

  return (
    <>
      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {sections.map((section) => (
          <div className="space-y-2" key={section.status}>
            <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {section.label}
            </h3>
            {section.entries.map((entry) => {
              const threadRootId = getThreadRootId(entry.key);
              const rootStatus: RootStatus =
                threadRootId !== null
                  ? (rootStatusMap.get(threadRootId) ?? "checking")
                  : "available";
              return (
                <DraftRow
                  entry={entry}
                  key={entry.key}
                  onDelete={handleDelete}
                  onOpen={handleOpen}
                  onSend={handleSendRequest}
                  rootStatus={rootStatus}
                  source={sources.get(entry.key) ?? UNKNOWN_DRAFT_SOURCE}
                />
              );
            })}
          </div>
        ))}
      </div>

      {sendTarget ? (
        <SendConfirmDialog
          channelLabel={sendDialogChannelLabel}
          isDm={sendDialogIsDm}
          onCancel={handleSendCancel}
          onConfirm={handleSendConfirm}
          open={true}
        />
      ) : null}
    </>
  );
}
