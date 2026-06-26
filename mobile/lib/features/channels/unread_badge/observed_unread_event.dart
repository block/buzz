import '../read_state/read_state_format.dart';

class ObservedUnreadEvent {
  final String id;
  final int createdAt;
  final String? rootId;
  final bool highPriority;
  final bool countsTowardBadge;
  final bool countsTowardAppBadge;

  const ObservedUnreadEvent({
    required this.id,
    required this.createdAt,
    required this.rootId,
    required this.highPriority,
    required this.countsTowardBadge,
    required this.countsTowardAppBadge,
  });
}

ObservedUnreadEvent makeObservedUnreadEvent({
  required String id,
  required int createdAt,
  required String? rootId,
  required bool highPriority,
  required String? channelType,
  required bool isThreadedReply,
}) {
  final isDm = channelType == 'dm';
  return ObservedUnreadEvent(
    id: id,
    createdAt: createdAt,
    rootId: rootId,
    highPriority: highPriority,
    countsTowardBadge: isDm || isThreadedReply || highPriority,
    countsTowardAppBadge: isDm || (!isThreadedReply && highPriority),
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

int? observedUnreadEventReadAt(
  ObservedUnreadEvent event,
  int? channelReadAt,
  int? Function(String rootId) getThreadOwnMarker,
  int? Function(String messageId) getMessageOwnMarker,
) {
  final markers = <int?>[channelReadAt, getMessageOwnMarker(event.id)];
  final rootId = event.rootId;
  if (rootId != null) {
    markers.add(getThreadOwnMarker(rootId));
  }
  return maxReadAt(markers);
}
