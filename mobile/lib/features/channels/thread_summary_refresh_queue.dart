import 'dart:async';
import 'dart:math' as math;

import '../../shared/relay/relay.dart';
import 'channel_window.dart';

/// Coalesces visible-thread recounts, with at most two concurrent scans.
class ThreadSummaryRefreshQueue {
  /// Builds a queue whose owner handles refresh errors and lifecycle validity.
  ThreadSummaryRefreshQueue({
    required this.refresh,
    required this.canRun,
    required this.onSettled,
  });

  /// Loads and publishes one root's complete summary.
  final Future<void> Function(String rootId) refresh;

  /// Whether the owner is connected and still mounted.
  final bool Function() canRun;

  /// Notifies the owner after an active request releases its slot.
  final void Function() onSettled;
  final Set<String> _queued = {};
  final Set<String> _active = {};
  Timer? _timer;

  /// Marks a root dirty. Repeated arrivals collapse into one pending scan.
  void enqueue(String rootId) {
    _queued.remove(rootId);
    _queued.add(rootId);
    // Dropped requests retain their lower-bound display until the thread opens.
    while (_queued.length > 256) {
      _queued.remove(_queued.first);
    }
    resume();
  }

  /// Removes queued work when a relay summary already supplied the answer.
  void cancel(String rootId) => _queued.remove(rootId);

  /// Whether another scan was requested after the active scan began.
  bool isDirty(String rootId) => _queued.contains(rootId);

  /// Cancels a scheduled wake-up while preserving dirty roots for reconnect.
  void pause() {
    _queued.addAll(_active);
    _timer?.cancel();
    _timer = null;
  }

  /// Resumes queued work after the owning connection recovers.
  void resume() {
    if (_timer != null || _queued.isEmpty || !canRun()) return;
    _timer = Timer(const Duration(milliseconds: 200), _drain);
  }

  void _drain() {
    _timer = null;
    if (!canRun()) return;
    for (final root in _queued.toList()) {
      if (_active.length >= 2) break;
      if (_active.contains(root)) continue;
      _queued.remove(root);
      _active.add(root);
      unawaited(_run(root));
    }
  }

  Future<void> _run(String root) async {
    try {
      await refresh(root);
    } finally {
      _active.remove(root);
      if (canRun()) {
        onSettled();
        resume();
      }
    }
  }
}

/// Conservatively adjusts known totals after explicit deletions, including
/// targets whose payloads have been evicted. Unknown ownership preserves navigation
/// and hides the stale number until a complete recount restores exactness.
Map<String, ChannelWindowThreadSummary> lowerBoundSummariesAfterDeletion(
  Map<String, ChannelWindowThreadSummary> summaries,
  List<NostrEvent> events,
  Set<String> targets,
) {
  final byId = {for (final event in events) event.id: event};
  final knownRoots = {
    for (final id in targets)
      if (EventKind.channelTimelineContentKinds.contains(byId[id]?.kind) &&
          byId[id]?.threadReference.parentId != null)
        byId[id]!.threadReference.rootId,
  };
  final knownTargetsPerRoot = <String?, int>{};
  for (final id in targets) {
    final event = byId[id];
    if (event != null &&
        EventKind.channelTimelineContentKinds.contains(event.kind) &&
        event.threadReference.parentId != null) {
      final root = event.threadReference.rootId;
      knownTargetsPerRoot[root] = (knownTargetsPerRoot[root] ?? 0) + 1;
    }
  }
  final hasUnknownTarget = targets.any((id) => !byId.containsKey(id));
  final deleted = {
    ...targets,
    for (final event in events)
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent)
        for (final tag in event.tags)
          if (tag.length > 1 && tag[0] == 'e') tag[1],
  };
  final cached = <String?, int>{};
  final remaining = <String?, int>{};
  for (final event in byId.values) {
    if (!EventKind.channelTimelineContentKinds.contains(event.kind) ||
        event.threadReference.parentId == null) {
      continue;
    }
    final root = event.threadReference.rootId;
    cached[root] = (cached[root] ?? 0) + 1;
    if (!deleted.contains(event.id)) {
      remaining[root] = (remaining[root] ?? 0) + 1;
    }
  }
  return {
    for (final entry in summaries.entries)
      if (knownRoots.contains(entry.key) ||
          (hasUnknownTarget &&
              entry.value.descendantCount > (cached[entry.key] ?? 0)))
        entry.key: ChannelWindowThreadSummary(
          replyCount: math.max(
            0,
            entry.value.replyCount - (knownTargetsPerRoot[entry.key] ?? 0),
          ),
          descendantCount: math.max(
            remaining[entry.key] ?? 0,
            entry.value.descendantCount - (knownTargetsPerRoot[entry.key] ?? 0),
          ),
          lastReplyAt: entry.value.lastReplyAt,
          participantPubkeys: entry.value.participantPubkeys,
          isLowerBound: true,
          isCountPending: hasUnknownTarget || entry.value.isCountPending,
        ),
  };
}
