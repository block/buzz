import 'timeline_message.dart';

/// Endings collapse only when a visible start card represents them and no
/// reactions on the ending need their own interactive surface.
bool isCollapsedHuddleEnding(
  TimelineMessage message,
  Iterable<TimelineMessage> loaded,
) {
  final event = message.systemEvent;
  if (event?.type != SystemEventType.huddleEnded ||
      event?.ephemeralChannelId == null ||
      message.reactions.isNotEmpty) {
    return false;
  }
  return loaded.any(
    (start) =>
        start.parentId == null &&
        start.systemEvent?.type == SystemEventType.huddleStarted &&
        start.systemEvent?.ephemeralChannelId == event!.ephemeralChannelId &&
        start.createdAt <= message.createdAt,
  );
}
