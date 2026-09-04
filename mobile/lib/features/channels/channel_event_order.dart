import '../../shared/relay/relay.dart';

/// Render order for top-level channel timelines.
///
/// The relay pages newest timestamp first and ascending id within a second.
/// Channel timelines reverse that composite order for display, matching
/// desktop: oldest timestamp first and descending id within a second.
int compareChannelTimelineEventsChronologically(
  NostrEvent left,
  NostrEvent right,
) {
  final createdAt = left.createdAt.compareTo(right.createdAt);
  return createdAt != 0 ? createdAt : right.id.compareTo(left.id);
}

/// Render order for thread replies.
///
/// Desktop threads use ascending id within a second, independently of the
/// channel-window display order.
int compareThreadRepliesChronologically(NostrEvent left, NostrEvent right) {
  final createdAt = left.createdAt.compareTo(right.createdAt);
  return createdAt != 0 ? createdAt : left.id.compareTo(right.id);
}
