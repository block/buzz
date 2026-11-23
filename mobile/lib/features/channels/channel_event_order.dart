import '../../shared/relay/relay.dart';

final _effectiveMilliseconds = Expando<int>();

/// Effective event time in milliseconds: `createdAt * 1000` plus the first
/// `ms` tag's canonical 0-999 offset, or 0 when that tag is missing or
/// non-canonical. Memoized per event, since sorts call it on every compare.
int channelEventMilliseconds(NostrEvent event) =>
    _effectiveMilliseconds[event] ??=
        event.createdAt * 1000 + _millisecondOffset(event.tags);

int _millisecondOffset(List<List<String>> tags) {
  for (final tag in tags) {
    if (tag.isEmpty || tag[0] != 'ms') continue;
    return tag.length > 1 ? _parseCanonicalOffset(tag[1]) : 0;
  }
  return 0;
}

/// Parses 1-3 ASCII digits with no leading zero (except exactly `0`);
/// anything else is 0.
int _parseCanonicalOffset(String value) {
  if (value.isEmpty || value.length > 3) return 0;
  if (value.length > 1 && value.codeUnitAt(0) == 0x30) return 0;
  var offset = 0;
  for (var i = 0; i < value.length; i++) {
    final digit = value.codeUnitAt(i) - 0x30;
    if (digit < 0 || digit > 9) return 0;
    offset = offset * 10 + digit;
  }
  return offset;
}

/// Render order for top-level channel timelines.
///
/// Effective milliseconds ascend, with descending id at ties. Relay paging
/// separately uses whole seconds descending and id ascending.
int compareChannelTimelineEventsChronologically(
  NostrEvent left,
  NostrEvent right,
) {
  final createdAt = channelEventMilliseconds(
    left,
  ).compareTo(channelEventMilliseconds(right));
  return createdAt != 0 ? createdAt : right.id.compareTo(left.id);
}

/// Render order for thread replies.
///
/// Effective milliseconds ascend, with ascending id at ties.
int compareThreadRepliesChronologically(NostrEvent left, NostrEvent right) {
  final createdAt = channelEventMilliseconds(
    left,
  ).compareTo(channelEventMilliseconds(right));
  return createdAt != 0 ? createdAt : left.id.compareTo(right.id);
}
