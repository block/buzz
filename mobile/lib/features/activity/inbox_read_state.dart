import '../../shared/read_state/read_state_format.dart';
import 'feed_item.dart';
import 'inbox_item.dart';

/// The newest read marker that reads one grouped [event], following the
/// unread badge's `observedUnreadEventReadAt`: the channel mark, the
/// event's own `msg:` mark, and for a thread reply `thread:<root>` and
/// `thread-activity:<root>`. Returns null for an event with no channel.
///
/// `activity:<channel>` never counts here. Activity rows hold only mentions,
/// needs-action and agent events addressed to the reader, eligible thread
/// replies, and DM messages. That catch-up mark reads none of those.
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

/// The newest marker covering any grouped event of [item], or 0.
int inboxItemReadAt(
  InboxItem item, {
  required int? Function(String contextId) markerOf,
}) =>
    maxReadAt([
      for (final event in _groupedEvents(item))
        inboxEventReadAt(event, markerOf: markerOf),
    ]) ??
    0;

/// Whether the row is read ("done"): a local unread override wins until any
/// device reads past the marker it was set at ([rawMarkerOf], the stored
/// markers without the moving retention floor, defaulting to [markerOf]);
/// otherwise the row is done
/// when the marks read every grouped event. Each event is checked on its
/// own, because reading a channel marks mentions with per-message marks, so
/// reading the newest says nothing about the rest. Channel-less rows fall
/// back to the local done set.
bool isInboxItemDone(
  InboxItem item, {
  required int? Function(String contextId) markerOf,
  int? Function(String contextId)? rawMarkerOf,
  required Map<String, int> localUnreadOverrides,
  required Set<String> localDoneSet,
}) {
  final ids = groupedInboxItemIds(item);
  final baselines = [for (final id in ids) ?localUnreadOverrides[id]];
  if (baselines.isNotEmpty &&
      inboxItemReadAt(item, markerOf: rawMarkerOf ?? markerOf) <=
          baselines.reduce((a, b) => a > b ? a : b)) {
    return false;
  }

  if (item.item.channelId == null) return localDoneSet.contains(item.id);
  return _groupedEvents(
    item,
  ).every((event) => isInboxEventRead(event, markerOf: markerOf));
}

Iterable<FeedItem> _groupedEvents(InboxItem item) => {
  for (final event in [item.item, ...item.groupItems]) event.id: event,
}.values;

/// Whether marking a channel row read reads [event] only by its own `msg:`
/// mark: an unmentioned reply also sent to the channel. The channel's unread
/// registry reads it by that mark alone, and the channel marker would also
/// read every older message in the channel. A mention keeps the channel
/// marker path it had before these replies joined Activity.
bool readsByOwnMark(FeedItem event) =>
    event.category != 'mention' && isChannelSentReply(event.tags);

/// All event ids identified with the row — desktop's `getGroupedInboxItemIds`.
List<String> groupedInboxItemIds(InboxItem item) {
  return {item.id, item.item.id, ...item.groupItems.map((i) => i.id)}.toList();
}

/// The channel-level timestamp that marking a thread row read should also
/// advance — desktop's `getGroupedChannelReadTimestamp`. Only top-level
/// (non-thread-reply) grouped events count, so marking a thread read cannot
/// swallow unrelated channel messages. Replies that [readsByOwnMark] get
/// their own marks instead.
({String channelId, int timestamp})? groupedChannelReadTimestamp(
  InboxItem item,
) {
  final channelId = item.item.channelId;
  if (channelId == null) return null;

  int? timestamp;
  for (final groupItem in item.groupItems) {
    if (groupItem.channelId != channelId ||
        isThreadReply(groupItem.tags) ||
        readsByOwnMark(groupItem)) {
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
