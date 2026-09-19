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
    final previous = threadSummaries[root];
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
      for (final event in before)
        if (event.kind == EventKind.deletion ||
            event.kind == EventKind.nip29DeleteEvent)
          for (final tag in event.tags)
            if (tag.length > 1 && tag[0] == 'e') tag[1],
    };
    final fresh = targets.difference(alreadyDeleted);
    if (fresh.isEmpty) return;
    _applyDeletedSummaries(fresh, before);
    final unknown = fresh
        .where(
          (id) =>
              _replyOwnership.rootFor(id) == null &&
              !before.any((event) => event.id == id),
        )
        .take(100)
        .toSet();
    if (unknown.isNotEmpty) _resolveDeletionOwners(unknown);
  }

  void _applyDeletedSummaries(Set<String> targets, List<NostrEvent> before) {
    final owners = <String, String>{};
    for (final id in targets) {
      final root = _replyOwnership.rootFor(id);
      if (root != null) owners[id] = root;
    }
    final floors = lowerBoundSummariesAfterDeletion(
      threadSummaries,
      before,
      targets,
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
  }

  Future<void> _resolveDeletionOwners(Set<String> targets) async {
    final generation = _initVersion;
    final version = _threadQuerySerial;
    try {
      final events = await _summarySession.queryRelay([
        NostrFilter(
          ids: targets.toList(),
          kinds: const [EventKind.channelThreadSummary],
          extensions: const {'resolve_thread_roots': true},
          tags: {
            '#h': [channelId],
          },
          limit: targets.length,
        ),
      ]);
      if (!_summaryMounted || generation != _initVersion) return;
      for (final event in events) {
        if (event.channelId != channelId ||
            event.kind != EventKind.channelThreadSummary) {
          throw StateError('Expected a channel-scoped thread summary.');
        }
        final root = event.getTagValue('e');
        if (root != null && (_threadQueryVersions[root] ?? 0) <= version) {
          _handleLiveEvent(event);
        }
      }
    } catch (error) {
      // Keep pending presentation on query failure; a fresh summary or explicit
      // thread query can still reconcile it.
      if (_summaryMounted && generation == _initVersion) {
        debugPrint(
          '[ChannelMessagesNotifier] deletion ownership lookup failed: $error',
        );
      }
    }
  }
}
