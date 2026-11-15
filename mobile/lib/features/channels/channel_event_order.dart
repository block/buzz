import '../../shared/relay/relay.dart';

final _millisecondOffset = RegExp(r'^(0|[1-9]\d{0,2})$');

int _eventMilliseconds(NostrEvent event) {
  for (final tag in event.tags) {
    if (tag.isEmpty || tag[0] != 'ms') continue;
    final value = tag.length > 1 ? tag[1] : '';
    final offset = _millisecondOffset.hasMatch(value) ? int.parse(value) : 0;
    return event.createdAt * 1000 + offset;
  }
  return event.createdAt * 1000;
}

/// Render order for top-level channel timelines.
///
/// Effective milliseconds ascend, with descending id at ties. Relay paging
/// separately uses whole seconds descending and id ascending.
int compareChannelTimelineEventsChronologically(
  NostrEvent left,
  NostrEvent right,
) {
  final createdAt = _eventMilliseconds(
    left,
  ).compareTo(_eventMilliseconds(right));
  return createdAt != 0 ? createdAt : right.id.compareTo(left.id);
}

/// Render order for thread replies.
///
/// Effective milliseconds ascend, with ascending id at ties.
int compareThreadRepliesChronologically(NostrEvent left, NostrEvent right) {
  final createdAt = _eventMilliseconds(
    left,
  ).compareTo(_eventMilliseconds(right));
  return createdAt != 0 ? createdAt : left.id.compareTo(right.id);
}
