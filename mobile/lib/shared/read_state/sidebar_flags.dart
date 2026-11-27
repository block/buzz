const maxSidebarFlags = 500;

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

/// Bound the record by age, keeping edits in this write and preferring to
/// evict a clear over an active flag of the same age. New clears must survive
/// older active flags so clients with stale caches can observe them.
void boundSidebarFlags<T extends SidebarFlagEntry>(
  Map<String, T> channels,
  Set<String> changed,
) {
  final excess = channels.length - maxSidebarFlags;
  if (excess <= 0) return;

  final evictionOrder = channels.entries.toList()
    ..sort((a, b) {
      final byEdit = (changed.contains(a.key) ? 1 : 0).compareTo(
        changed.contains(b.key) ? 1 : 0,
      );
      if (byEdit != 0) return byEdit;
      final byTime = sidebarFlagTimeMillis(
        a.value.updatedAt,
      ).compareTo(sidebarFlagTimeMillis(b.value.updatedAt));
      if (byTime != 0) return byTime;
      if (a.value.enabled != b.value.enabled) return a.value.enabled ? 1 : -1;
      return a.key.compareTo(b.key);
    });
  for (final entry in evictionOrder.take(excess)) {
    channels.remove(entry.key);
  }
}
