part of '../channel_detail_page.dart';

/// Where the timeline was when the user left it, to reopen it there after a
/// forward swipe. [alignment] is the message's bottom edge, as a fraction of
/// the viewport measured up from the bottom (the list is reversed).
typedef _TimelineAnchor = ({String messageId, double alignment});

int? _reversedIndexOf(
  List<List<MainTimelineEntry>> displayEntries,
  String? messageId,
) {
  if (messageId == null) return null;
  final chronologicalIndex = displayEntries.indexWhere(
    (group) => group.any((entry) => entry.message.id == messageId),
  );
  return chronologicalIndex < 0
      ? null
      : displayEntries.length - 1 - chronologicalIndex;
}

/// The lowest message whose bottom edge is on screen, and where that edge is.
/// None when following the latest message, which is where a fresh page opens.
_TimelineAnchor? _timelineAnchor(
  ItemPositionsListener positions,
  List<List<MainTimelineEntry>> displayEntries,
  ValueNotifier<bool> followsLatest,
) {
  if (followsLatest.value) return null;
  ItemPosition? lowest;
  for (final position in positions.itemPositions.value) {
    // The extra last index is the "Load earlier" row, not a message.
    if (position.index >= displayEntries.length) continue;
    if (position.itemLeadingEdge < 0) continue;
    if (lowest == null || position.itemLeadingEdge < lowest.itemLeadingEdge) {
      lowest = position;
    }
  }
  if (lowest == null) return null;
  final group = displayEntries[displayEntries.length - 1 - lowest.index];
  return (messageId: group.first.message.id, alignment: lowest.itemLeadingEdge);
}
