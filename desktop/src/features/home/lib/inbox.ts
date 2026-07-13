import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import type { TimelineReaction } from "@/features/messages/types";
import type {
  Channel,
  FeedItem,
  FeedItemCategory,
  HomeFeedResponse,
  RelayEvent,
} from "@/shared/api/types";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";

export type InboxFilter =
  | "all"
  | "mention"
  | "thread"
  | "needs_action"
  | "activity"
  | "agent_activity"
  | "reminders"
  | "drafts";

export type InboxItem = {
  avatarUrl: string | null;
  /**
   * Stable conversation identity: `rootId ?? parentId ?? event.id` for the
   * thread group. Does NOT change when a new reply advances the representative
   * latest event. Use this for lifecycle continuity: scroll gating, draft
   * keys, local-reply storage, and selection identity.
   */
  conversationId: string;
  id: string;
  item: FeedItem;
  categories: FeedItemCategory[];
  categoryLabel: string;
  channelLabel: string | null;
  fullTimestampLabel: string;
  groupItems: FeedItem[];
  isActionRequired: boolean;
  latestActivityAt: number;
  mentionNames: string[];
  mentionPubkeysByName?: Record<string, string>;
  preview: string;
  senderLabel: string;
  subject: string;
  timestampLabel: string;
};

export type InboxTypeLabel = {
  text: string;
  channelLabel: string | null;
};

export type InboxReply = {
  authorLabel: string;
  authorPubkey: string;
  avatarUrl: string | null;
  content: string;
  createdAt: number;
  depth?: number;
  fullTimestampLabel: string;
  id: string;
  /** Raw event kind — input to the config-nudge trust gate. */
  kind?: number;
  parentId?: string | null;
  reactions?: TimelineReaction[];
  rootId?: string | null;
  /**
   * Raw event signer, NOT the tag-attributed display author
   * (`authorPubkey`). The config-nudge trust gate authenticates against
   * this field so `actor`/`p`-tag spoofing can't enable the card.
   */
  signerPubkey?: string;
  tags?: string[][];
  timeLabel?: string;
};

export type InboxContextMessage = InboxReply & {
  depth: number;
  isSelected: boolean;
  mentionNames: string[];
  mentionPubkeysByName?: Record<string, string>;
};

export type InboxGroup = {
  label: string;
  items: InboxItem[];
};

type InboxChannel = Pick<Channel, "channelType" | "id" | "name">;

const listTimeFormatter = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});

const fullTimeFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

const shortDateFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
});

const shortDateWithYearFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

const weekdayFormatter = new Intl.DateTimeFormat("en-US", {
  weekday: "long",
});

function startOfDay(value: Date) {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate());
}

function diffInDays(from: Date, to: Date) {
  return Math.round(
    (startOfDay(from).getTime() - startOfDay(to).getTime()) / 86_400_000,
  );
}

function feedHeadline(item: FeedItem) {
  switch (item.kind) {
    case 40007:
      return "Reminder";
    case 43001:
      return "Job requested";
    case 43002:
      return "Job accepted";
    case 43003:
      return "Progress update";
    case 43004:
      return "Job result";
    case 43005:
      return "Job cancelled";
    case 43006:
      return "Job failed";
    case 45001:
      return "Forum post";
    case 45003:
      return "Forum reply";
    case 46010:
      return "Approval requested";
    default:
      if (item.category === "mention") {
        return "Mention";
      }

      if (item.category === "agent_activity") {
        return "Agent update";
      }

      return "Channel update";
  }
}

function feedPreview(item: FeedItem) {
  const content = item.content.trim();
  if (content.length > 0) {
    return content;
  }

  if (item.kind === 46010) {
    return "A workflow is waiting for approval.";
  }

  if (item.kind === 40007) {
    return "A reminder is waiting for you.";
  }

  return "No additional details were attached to this event.";
}

function categoryLabelFor(category: FeedItemCategory) {
  return category === "needs_action"
    ? "Needs Action"
    : category === "mention"
      ? "Mention"
      : category === "agent_activity"
        ? "Agent update"
        : "Activity";
}

export function isThreadActivityItem(item: FeedItem) {
  if (item.category !== "activity") {
    return false;
  }

  const thread = getThreadReference(item.tags);
  return thread.parentId !== null && !isBroadcastReply(item.tags);
}

function activityHeadline(item: FeedItem) {
  return feedHeadline(item);
}

function resolveItemChannel(
  item: FeedItem,
  channelById: ReadonlyMap<string, InboxChannel>,
) {
  const channel = item.channelId ? channelById.get(item.channelId) : undefined;
  const name = item.channelName?.trim() || channel?.name.trim() || null;

  return {
    name,
    type: item.channelType ?? channel?.channelType,
  };
}

function resolveGroupChannel(
  primaryItem: FeedItem,
  groupItems: FeedItem[],
  channelById: ReadonlyMap<string, InboxChannel>,
) {
  for (const candidate of [primaryItem, ...groupItems]) {
    const channel = resolveItemChannel(candidate, channelById);
    if (channel.name || channel.type) {
      return channel;
    }
  }

  return resolveItemChannel(primaryItem, channelById);
}

export function getInboxTypeLabel(item: InboxItem): InboxTypeLabel {
  const channelName = item.channelLabel;

  if (item.item.channelType === "dm") {
    return {
      text: item.senderLabel ? `DM from ${item.senderLabel}` : "DM",
      channelLabel: null,
    };
  }

  const primaryCategory = item.item.category;
  if (primaryCategory === "mention") {
    return {
      text: channelName ? "Mentioned in" : "Mentioned",
      channelLabel: channelName,
    };
  }

  if (primaryCategory === "needs_action") {
    return {
      text: channelName ? "Needs action in" : "Needs action",
      channelLabel: channelName,
    };
  }

  if (isThreadActivityItem(item.item)) {
    return {
      text: channelName ? "Thread in" : "Thread",
      channelLabel: channelName,
    };
  }

  return {
    text: channelName
      ? `${activityHeadline(item.item)} in`
      : activityHeadline(item.item),
    channelLabel: channelName,
  };
}

export function formatInboxTypeLabel(item: InboxItem) {
  const label = getInboxTypeLabel(item);
  return label.channelLabel
    ? `${label.text} #${label.channelLabel}`
    : label.text;
}

function categoryPriority(category: FeedItemCategory) {
  switch (category) {
    case "needs_action":
      return 0;
    case "mention":
      return 1;
    case "agent_activity":
      return 2;
    case "activity":
      return 3;
  }
}

function getInboxThreadKey(item: FeedItem) {
  const thread = getThreadReference(item.tags);
  return thread.rootId ?? thread.parentId ?? item.id;
}

/**
 * Returns the stable conversation ID for any FeedItem or relay event: the
 * NIP-10 root tag id, falling back to parent-reply tag id, then event id.
 * This is the same derivation used by `buildInboxItems` for `conversationId`.
 */
export function getInboxConversationId(
  tags: string[][],
  eventId: string,
): string {
  const thread = getThreadReference(tags);
  return thread.rootId ?? thread.parentId ?? eventId;
}

function formatInboxTimestamp(unixSeconds: number) {
  const date = new Date(unixSeconds * 1_000);
  const now = new Date();
  const dayDiff = diffInDays(now, date);

  if (dayDiff === 0) {
    return listTimeFormatter.format(date);
  }

  if (dayDiff === 1) {
    return "Yesterday";
  }

  if (now.getFullYear() === date.getFullYear()) {
    return shortDateFormatter.format(date);
  }

  return shortDateWithYearFormatter.format(date);
}

export function formatInboxFullTimestamp(unixSeconds: number) {
  return fullTimeFormatter.format(new Date(unixSeconds * 1_000));
}

export function relayEventFromFeedItem(item: FeedItem): RelayEvent {
  return {
    content: item.content,
    created_at: item.createdAt,
    id: item.id,
    kind: item.kind,
    pubkey: item.pubkey,
    sig: "",
    tags: item.tags,
  };
}

export function groupInboxItems(items: InboxItem[]): InboxGroup[] {
  const groups = new Map<string, InboxItem[]>();
  const now = new Date();

  for (const item of items) {
    const date = new Date(item.latestActivityAt * 1_000);
    const dayDiff = diffInDays(now, date);
    const label =
      dayDiff === 0
        ? "Today"
        : dayDiff === 1
          ? "Yesterday"
          : dayDiff < 7
            ? weekdayFormatter.format(date)
            : shortDateWithYearFormatter.format(date);

    const current = groups.get(label) ?? [];
    current.push(item);
    groups.set(label, current);
  }

  return [...groups.entries()].map(([label, groupedItems]) => ({
    label,
    items: groupedItems,
  }));
}

export function buildInboxItems({
  channels,
  currentPubkey,
  feed,
  profiles,
}: {
  channels?: InboxChannel[];
  currentPubkey?: string;
  feed?: HomeFeedResponse;
  profiles?: UserProfileLookup;
}): InboxItem[] {
  if (!feed) {
    return [];
  }

  const feedItems = [
    ...feed.feed.mentions.map((item) => ({
      ...item,
      category: "mention" as const,
    })),
    ...feed.feed.needsAction.map((item) => ({
      ...item,
      category: "needs_action" as const,
    })),
    ...feed.feed.activity.map((item) => ({
      ...item,
      category: "activity" as const,
    })),
    ...feed.feed.agentActivity.map((item) => ({
      ...item,
      category: "agent_activity" as const,
    })),
  ];
  const channelById = new Map(
    (channels ?? []).map((channel) => [channel.id, channel]),
  );

  const threadGroups = new Map<
    string,
    {
      items: FeedItem[];
      latestActivityAt: number;
      rootItem: FeedItem | null;
    }
  >();

  for (const item of feedItems) {
    const threadKey = getInboxThreadKey(item);
    const group = threadGroups.get(threadKey) ?? {
      items: [],
      latestActivityAt: 0,
      rootItem: null,
    };

    group.items.push(item);
    group.latestActivityAt = Math.max(group.latestActivityAt, item.createdAt);
    if (item.id === threadKey) {
      group.rootItem = item;
    }

    threadGroups.set(threadKey, group);
  }

  return [...threadGroups.entries()]
    .sort(
      ([, left], [, right]) => right.latestActivityAt - left.latestActivityAt,
    )
    .map(([conversationId, group]) => {
      const latestItem = group.items.reduce((latest, current) =>
        current.createdAt > latest.createdAt ? current : latest,
      );
      const item = latestItem;
      const categories = [
        ...new Set(group.items.map((groupItem) => groupItem.category)),
      ].sort((left, right) => categoryPriority(left) - categoryPriority(right));
      const senderLabel = resolveUserLabel({
        pubkey: item.pubkey,
        currentPubkey,
        profiles,
        preferResolvedSelfLabel: true,
      });
      const subject = feedHeadline(item);
      const preview = feedPreview(item);
      const { mentionNames, mentionPubkeysByName } = resolveMentionProps(
        item.tags,
        profiles,
      );
      const groupChannel = resolveGroupChannel(item, group.items, channelById);
      const channelLabel = groupChannel.name;
      const displayItem: FeedItem = {
        ...item,
        channelName: channelLabel ?? item.channelName,
        channelType: item.channelType ?? groupChannel.type,
      };
      const categoryLabel = categoryLabelFor(categories[0] ?? item.category);

      return {
        avatarUrl: profiles?.[item.pubkey.toLowerCase()]?.avatarUrl ?? null,
        conversationId,
        id: item.id,
        item: displayItem,
        categories,
        categoryLabel,
        channelLabel,
        fullTimestampLabel: formatInboxFullTimestamp(item.createdAt),
        groupItems: group.items,
        isActionRequired: categories.includes("needs_action"),
        latestActivityAt: group.latestActivityAt,
        mentionNames: mentionNames ?? [],
        mentionPubkeysByName,
        preview,
        senderLabel,
        subject,
        timestampLabel: formatInboxTimestamp(group.latestActivityAt),
      };
    });
}
