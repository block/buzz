import { FileText, Lock, Pencil, Trash2 } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  clearDraftEntry,
  getActiveDraftEntries,
  getSentDraftEntries,
  type DraftState,
} from "@/features/messages/lib/useDrafts";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
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
  const sent = getSentDraftEntries().filter(isVisibleDraft);
  const sections: DraftSection[] = [];

  if (active.length > 0) {
    sections.push({ label: "Drafts", status: "active", entries: active });
  }
  if (sent.length > 0) {
    sections.push({ label: "Sent", status: "sent", entries: sent });
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

function DraftRow({
  entry,
  onDelete,
  onOpen,
  source,
}: {
  entry: DraftListEntry;
  onDelete: (draftKey: string) => void;
  onOpen: (entry: DraftListEntry) => void;
  source: DraftSource;
}) {
  const isSent = entry.draft.status === "sent";
  const canOpen = canOpenDraft(entry.draft, source);
  const isPrivate = source.channel?.visibility === "private";
  const isDm = source.channel?.channelType === "dm";
  const channelLabel = source.channel
    ? isDm
      ? source.label
      : `#${source.label}`
    : UNKNOWN_CHANNEL_LABEL;

  return (
    <div
      className="group/draft-row relative rounded-md border border-border/70 bg-background transition-colors hover:bg-muted/40 focus-within:bg-muted/40"
      data-testid={`home-draft-item-${entry.key}`}
    >
      <button
        aria-label={`Open draft in ${channelLabel}`}
        className="block w-full min-w-0 px-3 py-3 text-left disabled:cursor-default"
        disabled={!canOpen}
        onClick={() => onOpen(entry)}
        type="button"
      >
        <div className="min-w-0 pr-0 transition-[padding] group-hover/draft-row:pr-16 group-focus-within/draft-row:pr-16">
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
            {isPrivate ? <Lock className="h-3.5 w-3.5 shrink-0" /> : null}
            <span
              className={cn(
                "truncate font-medium",
                source.channel ? "text-foreground" : "text-muted-foreground",
              )}
            >
              {channelLabel}
            </span>
            <span className="shrink-0 text-muted-foreground/70">
              {formatDraftCreatedAt(entry.draft)}
            </span>
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

      <div className="pointer-events-none absolute right-2 top-2 flex items-center gap-0.5 rounded-full bg-background/95 p-0.5 opacity-0 shadow-xs ring-1 ring-border/70 transition-opacity group-hover/draft-row:pointer-events-auto group-hover/draft-row:opacity-100 group-focus-within/draft-row:pointer-events-auto group-focus-within/draft-row:opacity-100">
        {isSent ? null : (
          <DraftRowActionButton
            disabled={!canOpen}
            label={canOpen ? "Open draft" : "No channel link"}
            onClick={() => onOpen(entry)}
          >
            <Pencil className="h-4 w-4" />
          </DraftRowActionButton>
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

export function DraftsPanel() {
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const [sections, setSections] =
    React.useState<DraftSection[]>(readDraftSections);

  const refreshDrafts = React.useCallback(() => {
    setSections(readDraftSections());
  }, []);

  React.useEffect(() => {
    refreshDrafts();
  }, [refreshDrafts]);

  const drafts = React.useMemo(
    () => sections.flatMap((section) => section.entries),
    [sections],
  );

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

  const handleOpen = React.useCallback(
    (entry: DraftListEntry) => {
      if (!entry.draft.channelId) {
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

  const handleDelete = React.useCallback(
    (draftKey: string) => {
      clearDraftEntry(draftKey);
      refreshDrafts();
    },
    [refreshDrafts],
  );

  if (sections.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8 text-center">
        <FileText className="h-8 w-8 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">No drafts</p>
      </div>
    );
  }

  return (
    <div className="flex-1 space-y-4 overflow-y-auto p-4">
      {sections.map((section) => (
        <div className="space-y-2" key={section.status}>
          <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            {section.label}
          </h3>
          {section.entries.map((entry) => (
            <DraftRow
              entry={entry}
              key={entry.key}
              onDelete={handleDelete}
              onOpen={handleOpen}
              source={sources.get(entry.key) ?? UNKNOWN_DRAFT_SOURCE}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
