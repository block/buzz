part of 'read_state_manager.dart';

/// A publication refused because the account's read state is full.
const readStateFullMessage =
    'Read state is full, so recent reads aren’t syncing to your other '
    'devices.';

/// Timestamp reservation and the shared full-snapshot capacity check.
extension ReadStateCapacity on ReadStateManager {
  /// Reserves the next timestamp. `created_at` is uint32 on the wire, so a
  /// reservation past it fails rather than wrapping or reusing one.
  int _reserve() {
    final next = max(max(_now(), _maxFetchedCreatedAt + 1), _lastReserved + 1);
    if (next > 4294967295) {
      throw StateError('No later read-state timestamp can be reserved');
    }
    return _lastReserved = next;
  }

  void _checkSnapshotCapacity(
    List<NostrEvent> snapshot,
    Iterable<NostrEvent> writes,
  ) {
    String coordinate(NostrEvent event) =>
        '${event.kind}:${event.pubkey}:${event.tags.where((tag) => tag.length >= 2 && tag.first == 'd').map((tag) => tag[1]).firstOrNull ?? ''}';
    final projected = {for (final event in snapshot) coordinate(event): event};
    for (final event in writes) {
      projected[coordinate(event)] = event;
      if (projected.length > readStateSnapshotMaxEvents ||
          utf8
                  .encode(
                    jsonEncode(
                      projected.values.map((event) => event.toJson()).toList(),
                    ),
                  )
                  .length >
              readStateSnapshotMaxBytes) {
        throw StateError(readStateFullMessage);
      }
    }
  }
}
