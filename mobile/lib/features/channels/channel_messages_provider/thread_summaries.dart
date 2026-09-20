part of '../channel_messages_provider.dart';

extension _ThreadSummaryState on ChannelMessagesNotifier {
  int _reserveThreadQuery(String root) {
    final version = ++_threadQuerySerial;
    _setThreadQueryVersion(root, version, pending: true);
    return version;
  }

  void _retireThreadQuery(String root, int version) {
    if (_threadQueryVersions[root] != version) return;
    final evidence = _threadEvidenceVersions[root];
    if (evidence == null) {
      _threadQueryVersions.remove(root);
    } else {
      _threadQueryVersions[root] = evidence;
    }
  }

  void _setThreadQueryVersion(
    String root,
    int version, {
    bool pending = false,
  }) {
    final current = _threadQueryVersions[root];
    if (!pending) _threadEvidenceVersions[root] = version;
    _threadQueryVersions.remove(root);
    // A page may apply while a newer query is pending. Preserve that query's
    // reservation so its eventual result can still replace the page evidence.
    _threadQueryVersions[root] = current != null && current > version
        ? current
        : version;
    while (_threadQueryVersions.length > 2048) {
      final oldest = _threadQueryVersions.keys.first;
      _threadQueryVersions.remove(oldest);
      _threadEvidenceVersions.remove(oldest);
    }
  }

  bool _isVisibleRoot(String root) =>
      _retainedDeepLinkEventIds.contains(root) ||
      (!_usingChannelWindow &&
          (_lastKnownMessages?.any((event) => event.id == root) ?? false)) ||
      _windowStore.pages.any(
        (page) => page.rows.any((row) => row.event.id == root),
      ) ||
      _windowStore.liveOverlay.any((event) => event.id == root);

  void _reconcileLiveSummaryPayloads(NostrEvent event) {
    final root = event.getTagValue('e');
    if (root == null || !_isVisibleRoot(root)) return;
    final summary = _baseThreadSummaries[root];
    final confirmedIds = cachedThreadReplyIds(root)
      ..removeAll(_localReplyRoots.keys);
    if (summary == null || confirmedIds.length <= summary.descendantCount) {
      return;
    }
    // The summary may lag a new reply, or its deletion marker may have been
    // missed. A fresh complete query resolves either case without discarding
    // valid local payloads merely because a summary count is lower.
    _queueOverflowSummary(root, countPending: true);
  }

  void _reconcileFallbackSummaries(int historyVersion) {
    // A bounded WebSocket history is not proof that an absent reply was deleted.
    // Recount visible cached aggregates, marking their old totals uncertain in
    // the meantime. Requests or live summaries newer than this history win.
    final roots = {
      ..._queryThreadSummaries.keys,
      ..._overflowFloors.keys,
      for (final event in _windowStore.liveOverlay)
        if (event.threadReference.parentId != null &&
            event.threadReference.rootId != null &&
            !_localReplyRoots.containsKey(event.id))
          event.threadReference.rootId!,
    };
    for (final root in roots) {
      if ((_threadQueryVersions[root] ?? 0) > historyVersion) continue;
      _setThreadQueryVersion(root, historyVersion);
      if (!_isVisibleRoot(root)) {
        _queryThreadSummaries.remove(root);
        _overflowFloors.remove(root);
        _deletionSummaryUncertainty.remove(root);
        _summaryRefreshes.cancel(root);
        continue;
      }
      _queueOverflowSummary(root, countPending: true);
    }
  }

  void _queueOverflowSummary(String root, {bool countPending = false}) {
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
      isCountPending: countPending || (previous?.isCountPending ?? false),
    );
    while (_overflowFloors.length > 2048) {
      _overflowFloors.remove(_overflowFloors.keys.first);
    }
    _summaryRefreshes.enqueue(root);
  }

  Future<void> _refreshOverflowSummary(String root, {int attempt = 0}) async {
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
      if (!current()) return;
      if (attempt < 2) {
        // Retain this active queue slot during backoff, so retries share the
        // same concurrency budget and stop when newer work supersedes them.
        await Future<void>.delayed(Duration(milliseconds: 500 << attempt));
        if (current()) {
          await _refreshOverflowSummary(root, attempt: attempt + 1);
        }
        return;
      }
      debugPrint(
        '[ChannelMessagesNotifier] thread recount failed for $root: $error',
      );
    } finally {
      _retireThreadQuery(root, version);
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
    // Fence responses started before this deletion, including during debounce.
    for (final root in owners.values.toSet()) {
      _setThreadQueryVersion(root, ++_threadQuerySerial);
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

  void _resolveDeletionOwners(Set<String> targets, Set<String> candidates) {
    // Register every target before sending the first bounded-queue query. A
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
    // The shared queue bounds work across events as well as within each batch.
    for (final entry in versions.entries) {
      if (!_enqueueDeletionOwner(entry.key, candidates, entry.value)) {
        _deferredDeletionTargets[entry.key] = (
          candidates: candidates,
          version: entry.value,
        );
        while (_deferredDeletionTargets.length > 8192) {
          final oldest = _deferredDeletionTargets.keys.first;
          final dropped = _deferredDeletionTargets.remove(oldest)!;
          _finishDeletionLookup(
            dropped.candidates,
            dropped.version,
            resolved: false,
          );
        }
      }
    }
  }

  bool _enqueueDeletionOwner(
    String target,
    Set<String> candidates,
    int version,
  ) => _deletionOwnerQueue.enqueue(() async {
    if (!_summaryMounted) return true;
    return _resolveDeletionOwner(target, candidates, _initVersion, version);
  });

  void _drainDeferredDeletionOwners() {
    if (!_summaryMounted) return;
    while (_deletionOwnerQueue.hasCapacity &&
        _deferredDeletionTargets.isNotEmpty) {
      final target = _deferredDeletionTargets.keys.first;
      final work = _deferredDeletionTargets.remove(target)!;
      _enqueueDeletionOwner(target, work.candidates, work.version);
    }
  }

  void _retireDeferredDeletionTargets(Iterable<String> targets) {
    for (final target in targets) {
      final work = _deferredDeletionTargets.remove(target);
      if (work != null) {
        _finishDeletionLookup(work.candidates, work.version, resolved: true);
      }
    }
  }

  Future<bool> _resolveDeletionOwner(
    String target,
    Set<String> candidates,
    int generation,
    int version, {
    int attempt = 0,
  }) async {
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
      if (!_summaryMounted) return true;
      if (generation != _initVersion) return false;
      if (events.length > 1) {
        throw StateError('Expected at most one owner for a deletion target.');
      }
      var applied = false;
      String? resolvedRoot;
      for (final event in events) {
        if (event.channelId != channelId ||
            event.kind != EventKind.channelThreadSummary) {
          throw StateError('Expected a channel-scoped thread summary.');
        }
        final root = event.getTagValue('e');
        resolvedRoot = root;
        if (root != null && (_threadQueryVersions[root] ?? 0) <= version) {
          _handleLiveEvent(event, summaryVersion: version);
          applied = _threadQueryVersions[root] == version;
        }
      }
      // An empty response does not prove ownership (including on old relays).
      // A response fenced by a newer query is not applied evidence either:
      // that query can still fail, leaving the old count uncertain.
      _finishDeletionLookup(
        candidates,
        version,
        resolved: resolvedRoot != null,
        unresolvedRoot: applied ? null : resolvedRoot,
      );
    } catch (error) {
      if (!_summaryMounted) return true;
      if (generation != _initVersion) return false;
      if (error is! StateError && error is! FormatException && attempt < 2) {
        // Keep the queue slot and original version while backing off. Retries
        // cannot exceed the shared concurrency cap or supersede newer evidence.
        await Future<void>.delayed(Duration(milliseconds: 500 << attempt));
        if (!_summaryMounted) return true;
        if (generation != _initVersion) return false;
        return _resolveDeletionOwner(
          target,
          candidates,
          generation,
          version,
          attempt: attempt + 1,
        );
      }
      // Exhausted or invalid responses remain uncertain until fresh evidence.
      if (_summaryMounted && generation == _initVersion) {
        _finishDeletionLookup(candidates, version, resolved: false);
        debugPrint(
          '[ChannelMessagesNotifier] deletion ownership lookup failed: $error',
        );
      }
    }
    return true;
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
    String? unresolvedRoot,
  }) {
    for (final root in candidates) {
      final pending = _deletionSummaryUncertainty[root];
      if (pending == null) continue;
      pending.finish(version, resolved: resolved && root != unresolvedRoot);
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

// Bound retained work and concurrent HTTP requests for the whole channel.
class _DeletionOwnerQueue {
  final bool Function() canRun;
  final void Function() onCapacity;
  final _pending = <Future<bool> Function()>[];
  int _active = 0;
  bool _paused = false;

  _DeletionOwnerQueue({required this.canRun, required this.onCapacity});

  bool get hasCapacity => _pending.length + _active < 258;

  bool enqueue(Future<bool> Function() request) {
    if (!hasCapacity) return false;
    _pending.add(request);
    _drain();
    return true;
  }

  void pause() => _paused = true;

  void resume() {
    _paused = false;
    _drain();
    onCapacity();
  }

  void _drain() {
    while (!_paused && canRun() && _active < 2 && _pending.isNotEmpty) {
      _active++;
      _run(_pending.removeAt(0));
    }
  }

  Future<void> _run(Future<bool> Function() request) async {
    try {
      // Interrupted work keeps its place in the same total admission budget.
      // Reconnect resumes it with a fresh connection generation, but the
      // original summary version still fences it against newer root evidence.
      if (!await request()) _pending.insert(0, request);
    } finally {
      _active--;
      _drain();
      onCapacity();
    }
  }
}
