part of '../channel_messages_provider.dart';

extension _ThreadSummaryState on ChannelMessagesNotifier {
  bool _isVisibleRoot(String root) =>
      _retainedDeepLinkEventIds.contains(root) ||
      _windowStore.pages.any(
        (page) => page.rows.any((row) => row.event.id == root),
      ) ||
      _windowStore.liveOverlay.any((event) => event.id == root);

  void _queueOverflowSummary(String root) {
    if (!_isVisibleRoot(root)) return;
    final ids = cachedThreadReplyIds(root)..removeAll(_localReplyRoots.keys);
    final events =
        _windowStore.liveOverlay
            .where((event) => ids.contains(event.id))
            .toList()
          ..sort(compareThreadRepliesChronologically);
    final previous = _baseThreadSummaries[root];
    final count = (previous?.descendantCount ?? 0) > ids.length
        ? previous!.descendantCount
        : ids.length;
    _overflowFloors.remove(root);
    _overflowFloors[root] = ChannelWindowThreadSummary(
      replyCount: count,
      descendantCount: count,
      lastReplyAt:
          previous?.lastReplyAt ??
          (events.isEmpty ? null : events.last.createdAt),
      participantPubkeys:
          previous?.participantPubkeys ??
          events.reversed.map((event) => event.pubkey).toSet().take(5).toList(),
      isLowerBound: true,
      isCountPending: previous?.isCountPending ?? false,
    );
    while (_overflowFloors.length > 2048) {
      _overflowFloors.remove(_overflowFloors.keys.first);
    }
    _summaryRefreshes.enqueue(root);
  }

  Future<void> _refreshOverflowSummary(String root) async {
    if (!_isVisibleRoot(root)) return;
    final generation = _initVersion;
    final version = beginThreadQuery(root);
    final snapshot = cachedThreadReplyIds(root);
    bool current() =>
        _summaryMounted &&
        _hasListeners &&
        generation == _initVersion &&
        _threadQueryVersions[root] == version;
    try {
      final replies = await fetchCompleteThreadReplies(
        _summarySession,
        ThreadRepliesArgs(channelId: channelId, rootId: root),
        isCurrent: current,
      );
      if (current()) {
        cacheCompleteThreadQuery(
          root,
          snapshot,
          replies,
          queryVersion: version,
        );
        if (_summaryRefreshes.isDirty(root)) _queueOverflowSummary(root);
      }
    } catch (error) {
      if (current()) {
        debugPrint(
          '[ChannelMessagesNotifier] thread recount failed for $root: $error',
        );
      }
    }
  }

  void _refreshDeletedSummaries(Set<String> targets, List<NostrEvent> before) {
    _replyOwnership.record(before);
    final alreadyDeleted = {
      ..._processedDeletionTargets,
      for (final event in before)
        if (event.kind == EventKind.deletion ||
            event.kind == EventKind.nip29DeleteEvent)
          for (final tag in event.tags)
            if (tag.length > 1 && tag[0] == 'e') tag[1],
    };
    final fresh = targets.difference(alreadyDeleted);
    _rememberDeletionTargets(targets);
    if (fresh.isEmpty) return;
    final candidates = _applyDeletedSummaries(fresh, before);
    final unknown = fresh
        .where(
          (id) =>
              _replyOwnership.rootFor(id) == null &&
              !before.any((event) => event.id == id),
        )
        .take(100)
        .toSet();
    if (unknown.isNotEmpty) _resolveDeletionOwners(unknown, candidates);
  }

  void _rememberDeletionTargets(Iterable<String> targets) {
    for (final target in targets) {
      _processedDeletionTargets.remove(target);
      _processedDeletionTargets.add(target);
    }
    while (_processedDeletionTargets.length > 8192) {
      _processedDeletionTargets.remove(_processedDeletionTargets.first);
    }
  }

  Set<String> _applyDeletedSummaries(
    Set<String> targets,
    List<NostrEvent> before,
  ) {
    final owners = <String, String>{};
    for (final id in targets) {
      final root = _replyOwnership.rootFor(id);
      if (root != null) owners[id] = root;
    }
    final summaries = _baseThreadSummaries;
    final candidates =
        lowerBoundSummariesAfterDeletion(summaries, before, targets, owners)
            .entries
            .where(
              (entry) =>
                  entry.value.isCountPending && _isVisibleRoot(entry.key),
            )
            .map((entry) => entry.key)
            .toSet();
    final floors = lowerBoundSummariesAfterDeletion(
      summaries,
      before,
      targets.where(owners.containsKey).toSet(),
      owners,
    );
    for (final entry in floors.entries) {
      if (!_isVisibleRoot(entry.key)) continue;
      _overflowFloors[entry.key] = entry.value;
      // Unknown ownership changes presentation only; never scan every candidate.
      if (owners.containsValue(entry.key)) _queueOverflowSummary(entry.key);
    }
    while (_overflowFloors.length > 2048) {
      _overflowFloors.remove(_overflowFloors.keys.first);
    }
    return candidates;
  }

  Future<void> _resolveDeletionOwners(
    Set<String> targets,
    Set<String> candidates,
  ) async {
    final generation = _initVersion;
    // Register the whole bounded batch before sending its first query. A
    // successful target must not retire uncertainty from a queued sibling.
    final versions = {
      for (final target in targets) target: ++_threadQuerySerial,
    };
    for (final root in candidates) {
      final pending = _deletionSummaryUncertainty[root] ??=
          _DeletionSummaryUncertainty();
      for (final version in versions.values) {
        pending.begin(version);
      }
    }
    while (_deletionSummaryUncertainty.length > 2048) {
      _deletionSummaryUncertainty.remove(
        _deletionSummaryUncertainty.keys.first,
      );
    }
    _publishSummaryChange();
    // One target per query correlates the response even when multiple targets
    // share a root. Sequential reads avoid a burst of up to 100 HTTP requests.
    for (final entry in versions.entries) {
      if (!_summaryMounted || generation != _initVersion) return;
      await _resolveDeletionOwner(
        entry.key,
        candidates,
        generation,
        entry.value,
      );
    }
  }

  Future<void> _resolveDeletionOwner(
    String target,
    Set<String> candidates,
    int generation,
    int version,
  ) async {
    try {
      final events = await _summarySession.queryRelay([
        NostrFilter(
          ids: [target],
          kinds: const [EventKind.channelThreadSummary],
          extensions: const {'resolve_thread_roots': true},
          tags: {
            '#h': [channelId],
          },
          limit: 1,
        ),
      ]);
      if (!_summaryMounted || generation != _initVersion) return;
      if (events.length > 1) {
        throw StateError('Expected at most one owner for a deletion target.');
      }
      for (final event in events) {
        if (event.channelId != channelId ||
            event.kind != EventKind.channelThreadSummary) {
          throw StateError('Expected a channel-scoped thread summary.');
        }
        final root = event.getTagValue('e');
        if (root != null && (_threadQueryVersions[root] ?? 0) <= version) {
          _handleLiveEvent(event, summaryVersion: version);
        }
      }
      // An empty response does not prove ownership (including on old relays).
      // Only this target's correlated summary can retire its uncertainty.
      _finishDeletionLookup(candidates, version, resolved: events.length == 1);
    } catch (error) {
      // Keep pending presentation on query failure; a fresh summary or explicit
      // thread query can still reconcile it.
      if (_summaryMounted && generation == _initVersion) {
        _finishDeletionLookup(candidates, version, resolved: false);
        debugPrint(
          '[ChannelMessagesNotifier] deletion ownership lookup failed: $error',
        );
      }
    }
  }

  void _clearDeletionUncertainty(String root, int version) {
    final pending = _deletionSummaryUncertainty[root];
    if (pending == null) return;
    pending.clearThrough(version);
    if (pending.isEmpty) _deletionSummaryUncertainty.remove(root);
  }

  void _finishDeletionLookup(
    Set<String> candidates,
    int version, {
    required bool resolved,
  }) {
    for (final root in candidates) {
      final pending = _deletionSummaryUncertainty[root];
      if (pending == null) continue;
      pending.finish(version, resolved: resolved);
      if (pending.isEmpty) _deletionSummaryUncertainty.remove(root);
    }
    _publishSummaryChange();
  }
}

// Track uncertainty independently of counts so resolving one deletion cannot
// erase another pending deletion, or leave unrelated exact counts downgraded.
class _DeletionSummaryUncertainty {
  final _requests = <int>{};
  int? _unresolvedVersion;

  bool get isEmpty => _requests.isEmpty && _unresolvedVersion == null;

  void begin(int version) {
    _requests.add(version);
    if (_requests.length > 256) {
      final oldest = _requests.first;
      if (oldest > (_unresolvedVersion ?? -1)) _unresolvedVersion = oldest;
      _requests.remove(oldest);
    }
  }

  void finish(int version, {required bool resolved}) {
    if (!_requests.remove(version)) return;
    if (!resolved && version > (_unresolvedVersion ?? -1)) {
      _unresolvedVersion = version;
    }
  }

  void clearThrough(int version) {
    _requests.removeWhere((request) => request <= version);
    if ((_unresolvedVersion ?? -1) <= version) _unresolvedVersion = null;
  }
}
