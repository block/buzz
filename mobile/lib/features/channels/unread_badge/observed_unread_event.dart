import '../../../shared/read_state/read_state_format.dart';
import '../../../shared/relay/nostr_models.dart';

class ObservedUnreadEvent {
  final String id;
  final int createdAt;
  final String? rootId;
  final bool highPriority;
  final bool countsTowardBadge;
  final bool countsTowardAppBadge;

  /// `activity:<channel>` reads this event (see `readByChannelCatchUp`).
  final bool channelCatchUp;

  /// Reply payload for Activity's existing thread rows. Retained in the same
  /// bounded registry as its unread evidence, so eviction and deletion agree.
  final NostrEvent? threadReply;

  const ObservedUnreadEvent({
    required this.id,
    required this.createdAt,
    required this.rootId,
    required this.highPriority,
    required this.countsTowardBadge,
    required this.countsTowardAppBadge,
    this.channelCatchUp = false,
    this.threadReply,
  });
}

ObservedUnreadEvent makeObservedUnreadEvent({
  required String id,
  required int createdAt,
  required String? rootId,
  required bool highPriority,
  required String? channelType,
  required bool isThreadedReply,
  NostrEvent? threadReply,
}) {
  final isDm = channelType == 'dm';
  return ObservedUnreadEvent(
    id: id,
    createdAt: createdAt,
    rootId: rootId,
    highPriority: highPriority,
    countsTowardBadge: isDm || isThreadedReply || highPriority,
    countsTowardAppBadge: isDm || (!isThreadedReply && highPriority),
    channelCatchUp: readByChannelCatchUp(
      isDm: isDm,
      isReply: isThreadedReply,
      highPriority: highPriority,
    ),
    threadReply: threadReply,
  );
}

bool recordObservedUnreadEvent(
  Map<String, Map<String, ObservedUnreadEvent>> eventsByChannel,
  String channelId,
  ObservedUnreadEvent event,
  int limit,
) {
  final eventsById = eventsByChannel.putIfAbsent(channelId, () => {});
  if (eventsById.containsKey(event.id)) return false;

  eventsById[event.id] = event;
  if (eventsById.length <= limit) return true;

  String? oldestId;
  int? oldestCreatedAt;
  for (final event in eventsById.values) {
    if (oldestCreatedAt == null || event.createdAt < oldestCreatedAt) {
      oldestCreatedAt = event.createdAt;
      oldestId = event.id;
    }
  }
  if (oldestId != null) {
    eventsById.remove(oldestId);
  }
  return true;
}

int countUnreadObservedEvents(
  Map<String, ObservedUnreadEvent>? eventsById,
  int? Function(ObservedUnreadEvent event) getReadAt,
) {
  if (eventsById == null) return 0;
  var count = 0;
  for (final event in eventsById.values) {
    final readAt = getReadAt(event);
    if (readAt == null || event.createdAt > readAt) count++;
  }
  return count;
}

int countUnreadBadgeObservedEvents(
  Map<String, ObservedUnreadEvent>? eventsById,
  int? Function(ObservedUnreadEvent event) getReadAt,
) {
  if (eventsById == null) return 0;
  var count = 0;
  for (final event in eventsById.values) {
    if (!event.countsTowardBadge) continue;
    final readAt = getReadAt(event);
    if (readAt == null || event.createdAt > readAt) count++;
  }
  return count;
}

int countUnreadAppBadgeObservedEvents(
  Map<String, ObservedUnreadEvent>? eventsById,
  int? Function(ObservedUnreadEvent event) getReadAt,
) {
  if (eventsById == null) return 0;
  var count = 0;
  for (final event in eventsById.values) {
    if (!event.countsTowardAppBadge) continue;
    final readAt = getReadAt(event);
    if (readAt == null || event.createdAt > readAt) count++;
  }
  return count;
}

int countUnreadHighPriorityObservedEvents(
  Map<String, ObservedUnreadEvent>? eventsById,
  int? Function(ObservedUnreadEvent event) getReadAt,
) {
  if (eventsById == null) return 0;
  var count = 0;
  for (final event in eventsById.values) {
    if (!event.highPriority) continue;
    final readAt = getReadAt(event);
    if (readAt == null || event.createdAt > readAt) count++;
  }
  return count;
}

/// The newest marker that reads `event` in `channelId`. `markerOf` looks up
/// a read-state context by key.
int? observedUnreadEventReadAt(
  ObservedUnreadEvent event,
  String channelId,
  int? Function(String contextId) markerOf,
) {
  final rootId = event.rootId;
  return maxReadAt([
    markerOf(channelId),
    markerOf(msgContextKey(event.id)),
    if (rootId != null) ...[
      markerOf(threadContextKey(rootId)),
      markerOf(threadActivityContextKey(rootId)),
    ],
    if (event.channelCatchUp) markerOf(activityContextKey(channelId)),
  ]);
}
