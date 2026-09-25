import '../../shared/relay/relay.dart';

/// Bounded identity-only evidence for replies whose payloads may be evicted.
class ThreadReplyOwnership {
  final Map<String, String> _roots = {};

  /// Remembers recent ownership without retaining event payloads.
  void record(Iterable<NostrEvent> events) {
    for (final event in events) {
      final thread = event.threadReference;
      if (!EventKind.channelTimelineContentKinds.contains(event.kind) ||
          thread.parentId == null ||
          thread.rootId == null) {
        continue;
      }
      _roots.remove(event.id);
      _roots[event.id] = thread.rootId!;
      while (_roots.length > 8192) {
        _roots.remove(_roots.keys.first);
      }
    }
  }

  /// Returns known ownership or null when the identity evidence was evicted.
  String? rootFor(String eventId) => _roots[eventId];
}
