import '../../shared/read_state/read_state_format.dart';
import 'feed_item.dart';
import 'inbox_item.dart';

/// The newest read marker that reads one grouped [event], following the
/// unread badge's `observedUnreadEventReadAt`: the channel mark, the
/// event's own `msg:` mark, and for a thread reply `thread:<root>` and
/// `thread-activity:<root>`. Returns null for an event with no channel.
///
/// `activity:<channel>` never counts here. Activity rows hold only mentions,
/// needs-action and agent events addressed to the reader, and DM messages,
/// and that catch-up mark reads none of those.
int? inboxEventReadAt(
  FeedItem event, {
  required int? Function(String contextId) markerOf,
}) {
  final channelId = event.channelId;
  if (channelId == null) return null;
  final rootId = isThreadReply(event.tags)
      ? threadReferenceOf(event.tags).rootId
      : null;
  return maxReadAt([
    markerOf(channelId),
    markerOf(msgContextKey(event.id)),
    if (rootId != null) ...[
      markerOf(threadContextKey(rootId)),
      markerOf(threadActivityContextKey(rootId)),
    ],
  ]);
}

/// Whether the marks read one grouped [event].
bool isInboxEventRead(
  FeedItem event, {
  required int? Function(String contextId) markerOf,
}) {
  final readAt = inboxEventReadAt(event, markerOf: markerOf);
  return readAt != null && event.createdAt <= readAt;
}

/// Whether the row is read ("done"): a local unread override always wins;
/// otherwise the row is done when the marks read every grouped event. Each
/// event is checked on its own, because reading a channel marks mentions
/// with per-message marks, so reading the newest says nothing about the
/// rest. Channel-less rows fall back to the local done set.
bool isInboxItemDone(
  InboxItem item, {
  required int? Function(String contextId) markerOf,
  required Set<String> localUnreadOverrides,
  required Set<String> localDoneSet,
}) {
  final ids = groupedInboxItemIds(item);
  if (ids.any(localUnreadOverrides.contains)) return false;

  if (item.item.channelId == null) return localDoneSet.contains(item.id);
  return _groupedEvents(
    item,
  ).every((event) => isInboxEventRead(event, markerOf: markerOf));
}

Iterable<FeedItem> _groupedEvents(InboxItem item) => {
  for (final event in [item.item, ...item.groupItems]) event.id: event,
}.values;

/// All event ids identified with the row — desktop's `getGroupedInboxItemIds`.
List<String> groupedInboxItemIds(InboxItem item) {
  return {item.id, item.item.id, ...item.groupItems.map((i) => i.id)}.toList();
}

/// The channel-level timestamp that marking a thread row read should also
/// advance — desktop's `getGroupedChannelReadTimestamp`. Only top-level
/// (non-thread-reply) grouped events count, so marking a thread read cannot
/// swallow unrelated channel messages.
({String channelId, int timestamp})? groupedChannelReadTimestamp(
  InboxItem item,
) {
  final channelId = item.item.channelId;
  if (channelId == null) return null;

  int? timestamp;
  for (final groupItem in item.groupItems) {
    if (groupItem.channelId != channelId || isThreadReply(groupItem.tags)) {
      continue;
    }
    if (timestamp == null || groupItem.createdAt > timestamp) {
      timestamp = groupItem.createdAt;
    }
  }
  return timestamp == null
      ? null
      : (channelId: channelId, timestamp: timestamp);
}
