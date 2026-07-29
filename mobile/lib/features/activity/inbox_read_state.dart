import '../channels/read_state/read_state_format.dart';
import 'inbox_item.dart';

/// Resolves the effective NIP-RS read marker for one inbox row, mirroring
/// desktop's `resolveInboxItemReadAt`:
/// - thread rows use `max(thread:<root>, msg:<id>)`
/// - channel rows use the channel context marker
/// - rows with no channel have no marker (caller falls back to local state)
int? resolveInboxItemReadAt(
  InboxItem item, {
  required int? Function(String contextId) markerOf,
}) {
  final channelId = item.item.channelId;
  final threadRootId = item.threadRootId;
  if (threadRootId != null) {
    return maxReadAt([
      markerOf(threadContextKey(threadRootId)),
      markerOf(msgContextKey(item.item.id)),
    ]);
  }
  return channelId == null ? null : markerOf(channelId);
}

/// Whether the row is read ("done"), mirroring desktop's
/// `useHomeInboxReadState` projection: a local unread override always wins;
/// otherwise the row is done when no grouped activity is newer than the
/// shared read marker; channel-less rows fall back to the local done set.
bool isInboxItemDone(
  InboxItem item, {
  required int? Function(String contextId) markerOf,
  required Set<String> localUnreadOverrides,
  required Set<String> localDoneSet,
}) {
  final ids = groupedInboxItemIds(item);
  if (ids.any(localUnreadOverrides.contains)) return false;

  final readAt = resolveInboxItemReadAt(item, markerOf: markerOf);
  if (readAt != null) return item.latestActivityAt <= readAt;

  if (item.threadRootId != null || item.item.channelId != null) return false;
  return localDoneSet.contains(item.id);
}

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
