import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineReaction } from "@/features/messages/types";
import type {
  FeedItem,
  FeedItemCategory,
  HomeFeedResponse,
  RelayEvent,
} from "@/shared/api/types";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";

export type InboxFilter =
  | "all"
  | "mention"
  | "needs_action"
  | "activity"
  | "agent_activity";

export type InboxItem = {
  avatarUrl: string | null;
  id: string;
  item: FeedItem;
  categories: FeedItemCategory[];
  categoryLabel: string;
  channelLabel: string | null;
  fullTimestampLabel: string;
  isActionRequired: boolean;
  latestActivityAt: number;
  mentionNames: string[];
  preview: string;
  senderLabel: string;
  subject: string;
  timestampLabel: string;
};

export type InboxReply = {
  authorLabel: string;
  avatarUrl: string | null;
  content: string;
  depth?: number;
  fullTimestampLabel: string;
  id: string;
  parentId?: string | null;
  reactions?: TimelineReaction[];
  rootId?: string | null;
};

export type InboxContextMessage = InboxReply & {
  depth: number;
  isSelected: boolean;
  mentionNames: string[];
};

export type InboxGroup = {
  label: string;
  items: InboxItem[];
};

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

export function formatInboxTypeLabel(item: InboxItem) {
  const channelName = item.channelLabel;
  const channelSuffix = channelName ? ` in #${channelName}` : "";

  if (item.item.channelType === "dm") {
    return item.senderLabel ? `DM from ${item.senderLabel}` : "DM";
  }

  const category = item.categories[0] ?? item.item.category;
  if (category === "mention") {
    return channelName ? `Mentioned in #${channelName}` : "Mentioned";
  }

  if (category === "needs_action") {
    return channelName ? `Needs action in #${channelName}` : "Needs action";
  }

  return `${feedHeadline(item.item)}${channelSuffix}`;
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
  currentPubkey,
  feed,
  profiles,
}: {
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

  return [...threadGroups.values()]
    .sort((left, right) => right.latestActivityAt - left.latestActivityAt)
    .map((group) => {
      const latestItem = group.items.reduce((latest, current) =>
        current.createdAt > latest.createdAt ? current : latest,
      );
      const item = group.rootItem ?? latestItem;
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
      const mentionNames = resolveMentionNames(item.tags, profiles) ?? [];
      const channelLabel = item.channelName.trim() || null;
      const categoryLabel = categoryLabelFor(categories[0] ?? item.category);

      return {
        avatarUrl: profiles?.[item.pubkey.toLowerCase()]?.avatarUrl ?? null,
        id: item.id,
        item,
        categories,
        categoryLabel,
        channelLabel,
        fullTimestampLabel: formatInboxFullTimestamp(item.createdAt),
        isActionRequired: categories.includes("needs_action"),
        latestActivityAt: group.latestActivityAt,
        mentionNames,
        preview,
        senderLabel,
        subject,
        timestampLabel: formatInboxTimestamp(group.latestActivityAt),
      };
    });
}
