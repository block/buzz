import 'dart:collection';

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

  /// Whether this event adds the channel's attention dot rather than only
  /// bolding it.
  bool get attention => countsTowardBadge;

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

/// Per-channel change stamps for the observed unread registry. Every add,
/// eviction, deletion or clear of a channel's events must [touch] it, so
/// readers can reuse work derived from an unchanged channel.
class ObservedUnreadRevisions {
  int _next = 0;
  int _floor = 0;
  final Map<String, int> _byChannel = {};

  /// Changes whenever [channelId]'s observed events may have changed.
  int of(String channelId) => _byChannel[channelId] ?? _floor;

  void touch(String channelId) => _byChannel[channelId] = ++_next;

  void touchAll() {
    _byChannel.clear();
    _floor = ++_next;
  }
}

/// Observed unread events by channel, with the stamp that says when a
/// channel's events last changed. Only a view carrying stamps may be
/// memoized against; a plain map is always read in full.
class ObservedUnreadSnapshot
    extends UnmodifiableMapView<String, Map<String, ObservedUnreadEvent>> {
  ObservedUnreadSnapshot(super.map, this.revisionOf);

  /// See [ObservedUnreadRevisions.of].
  final int Function(String channelId) revisionOf;
}

ObservedUnreadEvent makeObservedUnreadEvent({
  required String id,
  required int createdAt,
  required String? rootId,
  required bool highPriority,
  required String? channelType,
  required bool isThreadedReply,
  bool isBroadcast = false,
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
      // A broadcast is read only by its own mark, like any reply, even when
      // it counts as ordinary unread.
      isReply: isThreadedReply || isBroadcast,
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
