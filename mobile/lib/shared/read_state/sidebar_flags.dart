const _maxSidebarFlags = 500;

/// Older clients used ten-digit Unix seconds. Leave earlier sentinel values
/// and millisecond timestamps unchanged, including already converted entries.
int sidebarFlagTimeMillis(int timestamp) =>
    timestamp >= 1000000000 && timestamp < 10000000000
    ? timestamp * 1000
    : timestamp;

/// A channel star or mute and the timestamp of its last change.
abstract interface class SidebarFlagEntry {
  bool get enabled;
  int get updatedAt;
}

/// Evict old clears before active flags. Keep changes from the current write
/// ahead of existing entries so an unstar or unmute can still reach peers.
void boundSidebarFlags<T extends SidebarFlagEntry>(
  Map<String, T> channels,
  Set<String> changed,
) {
  final excess = channels.length - _maxSidebarFlags;
  if (excess <= 0) return;

  int priority(MapEntry<String, T> entry) =>
      changed.contains(entry.key) ? 2 : (entry.value.enabled ? 1 : 0);

  final evictionOrder = channels.entries.toList()
    ..sort((a, b) {
      final byPriority = priority(a).compareTo(priority(b));
      if (byPriority != 0) return byPriority;
      final byTime = sidebarFlagTimeMillis(
        a.value.updatedAt,
      ).compareTo(sidebarFlagTimeMillis(b.value.updatedAt));
      return byTime != 0 ? byTime : a.key.compareTo(b.key);
    });
  for (final entry in evictionOrder.take(excess)) {
    channels.remove(entry.key);
  }
}
