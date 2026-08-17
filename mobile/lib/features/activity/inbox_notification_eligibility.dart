import 'dart:convert';

import '../../shared/deeplink/deep_link.dart';
import 'feed_item.dart';
import 'inbox_item.dart';

const inboxNotificationBodyMaxLength = 140;

/// Items that should surface as an OS inbox notification.
enum InboxNotificationKind { mention, needsAction, dm, agentActivity }

/// Whether [item] belongs in the Activity inbox notification surface.
InboxNotificationKind? inboxNotificationKindFor(
  FeedItem item, {
  required Set<String> dmChannelIds,
}) {
  if (item.category == 'needs_action') {
    return InboxNotificationKind.needsAction;
  }
  if (item.category == 'mention') {
    return InboxNotificationKind.mention;
  }
  if (item.category == 'agent_activity') {
    return InboxNotificationKind.agentActivity;
  }
  if (item.category == 'activity') {
    final channelId = item.channelId;
    if (channelId != null && dmChannelIds.contains(channelId)) {
      return InboxNotificationKind.dm;
    }
  }
  return null;
}

/// Collect inbox-eligible feed items sorted oldest-first for delivery order.
List<FeedItem> eligibleInboxNotificationItems(
  HomeFeedResponse feed, {
  required Set<String> dmChannelIds,
}) {
  final items = <FeedItem>[
    ...feed.mentions,
    ...feed.needsAction,
    ...feed.agentActivity,
    for (final item in feed.activity)
      if (inboxNotificationKindFor(item, dmChannelIds: dmChannelIds) ==
          InboxNotificationKind.dm)
        item,
  ];
  items.sort((left, right) => left.createdAt.compareTo(right.createdAt));
  return items;
}

/// All inbox-eligible ids currently present in [feed] (for first-snapshot seeding).
List<String> collectInboxNotificationItemIds(
  HomeFeedResponse feed, {
  required Set<String> dmChannelIds,
}) {
  return eligibleInboxNotificationItems(
    feed,
    dmChannelIds: dmChannelIds,
  ).map((item) => item.id).toList();
}

bool isOwnFeedItem(FeedItem item, String myPubkey) {
  return item.pubkey.toLowerCase() == myPubkey.toLowerCase();
}

bool isMutedChannelForInboxNotification(
  FeedItem item,
  Set<String> mutedChannelIds,
) {
  final channelId = item.channelId;
  if (channelId == null) return false;
  if (!mutedChannelIds.contains(channelId)) return false;
  // Mentions still notify on muted channels (matches desktop feed toasts).
  return inboxNotificationKindFor(item, dmChannelIds: const {}) !=
      InboxNotificationKind.mention;
}

bool isMutedThreadForInboxNotification(
  FeedItem item,
  Set<String> mutedRootIds,
) {
  if (mutedRootIds.isEmpty) return false;
  final thread = threadReferenceOf(item.tags);
  final rootId = isBroadcastReply(item.tags)
      ? null
      : (thread.rootId ?? thread.parentId);
  return rootId != null && mutedRootIds.contains(rootId);
}

/// Whether a new inbox item should post an OS notification.
bool shouldNotifyForInboxItem(
  FeedItem item, {
  required String myPubkey,
  required Set<String> dmChannelIds,
  required Set<String> mutedChannelIds,
  required Set<String> mutedRootIds,
}) {
  if (isOwnFeedItem(item, myPubkey)) return false;
  if (inboxNotificationKindFor(item, dmChannelIds: dmChannelIds) == null) {
    return false;
  }
  if (isMutedChannelForInboxNotification(item, mutedChannelIds)) {
    return false;
  }
  if (isMutedThreadForInboxNotification(item, mutedRootIds)) {
    return false;
  }
  return true;
}

String? notificationChannelLabel({
  required String? channelId,
  required String channelName,
  required bool isDm,
}) {
  if (isDm) return null;
  final trimmed = channelName.trim();
  if (trimmed.isEmpty) return null;
  return '#$trimmed';
}

String formatNotificationTitle({
  required String prefix,
  required String? channelLabel,
}) {
  if (channelLabel != null && channelLabel.isNotEmpty) {
    return '$prefix in $channelLabel';
  }
  return prefix;
}

String truncateNotificationBody(String content, String fallback) {
  final trimmed = content.trim();
  if (trimmed.isEmpty) return fallback;
  if (trimmed.length <= inboxNotificationBodyMaxLength) return trimmed;
  return '${trimmed.substring(0, inboxNotificationBodyMaxLength - 3).trimRight()}...';
}

String inboxNotificationTitle(
  FeedItem item, {
  required bool isDm,
  required String? channelLabel,
  String? senderName,
}) {
  if (isDm) {
    return senderName?.trim().isNotEmpty == true
        ? senderName!.trim()
        : 'Direct message';
  }

  final kind = inboxNotificationKindFor(item, dmChannelIds: const {});
  switch (kind) {
    case InboxNotificationKind.mention:
      return formatNotificationTitle(
        prefix: senderName?.trim().isNotEmpty == true
            ? '$senderName mentioned you'
            : '@Mention',
        channelLabel: channelLabel,
      );
    case InboxNotificationKind.needsAction:
      if (item.kind == 46010) {
        return formatNotificationTitle(
          prefix: senderName?.trim().isNotEmpty == true
              ? '$senderName requested approval'
              : 'Approval Requested',
          channelLabel: channelLabel,
        );
      }
      return formatNotificationTitle(
        prefix: senderName?.trim().isNotEmpty == true
            ? senderName!.trim()
            : 'Needs Action',
        channelLabel: channelLabel,
      );
    case InboxNotificationKind.agentActivity:
      if (item.kind == 43004) {
        return formatNotificationTitle(
          prefix: senderName?.trim().isNotEmpty == true
              ? '$senderName finished a job'
              : 'Agent finished a job',
          channelLabel: channelLabel,
        );
      }
      return formatNotificationTitle(
        prefix: senderName?.trim().isNotEmpty == true
            ? senderName!.trim()
            : item.headline,
        channelLabel: channelLabel,
      );
    case InboxNotificationKind.dm:
      return senderName?.trim().isNotEmpty == true
          ? senderName!.trim()
          : 'Direct message';
    case null:
      return 'Buzz';
  }
}

String inboxNotificationBody(FeedItem item) {
  final fallback = switch (item.kind) {
    46010 => 'A workflow is waiting for your approval.',
    43004 => 'An agent finished a job.',
    _ => 'Something in Buzz needs your attention.',
  };
  return truncateNotificationBody(item.content, fallback);
}

/// Deep-link payload stored on the OS notification for tap handling.
String inboxNotificationPayload(FeedItem item) {
  final channelId = item.channelId;
  if (channelId == null || channelId.isEmpty) {
    throw ArgumentError('inboxNotificationPayload: channelId is required');
  }
  final thread = threadReferenceOf(item.tags);
  final threadRootId = isBroadcastReply(item.tags) ? null : thread.parentId;
  return buildMessageLink(
    channelId: channelId,
    messageId: item.id,
    threadRootId: threadRootId,
  );
}

bool shouldSkipInboxNotificationBecauseVisible({
  required FeedItem item,
  required VisibleConversationSnapshot? visible,
  required bool appIsResumed,
}) {
  if (!appIsResumed || visible == null) return false;
  if (visible.channelId != item.channelId) return false;

  final thread = threadReferenceOf(item.tags);
  final itemThreadRoot = isBroadcastReply(item.tags)
      ? null
      : (thread.rootId ?? thread.parentId);

  if (itemThreadRoot != null) {
    return visible.threadRootId == itemThreadRoot;
  }
  return visible.threadRootId == null;
}

/// Minimal projection of [VisibleConversation] for pure eligibility tests.
class VisibleConversationSnapshot {
  final String channelId;
  final String? messageId;
  final String? threadRootId;

  const VisibleConversationSnapshot({
    required this.channelId,
    this.messageId,
    this.threadRootId,
  });
}

Set<String> readMutedThreadRootIds(String? rawJson) {
  if (rawJson == null || rawJson.isEmpty) return {};
  try {
    final decoded = jsonDecode(rawJson);
    if (decoded is! List) return {};
    return {
      for (final value in decoded)
        if (value is String) value,
    };
  } catch (_) {
    return {};
  }
}

const mutedThreadRootIdsStoragePrefix = 'buzz-thread-muted.v1';

String mutedThreadRootIdsPrefsKey(String pubkey) =>
    '$mutedThreadRootIdsStoragePrefix:${pubkey.toLowerCase()}';
