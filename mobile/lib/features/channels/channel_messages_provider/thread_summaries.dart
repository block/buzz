part of '../channel_messages_provider.dart';

extension _ThreadSummaryState on ChannelMessagesNotifier {
  void _reconcilePageSummaries(ChannelWindowPage page, int pageVersion) {
    for (final row in page.rows) {
      final root = row.event.id;
      if ((_threadEvidenceVersions[root] ?? 0) > pageVersion) continue;
      _queryThreadSummaries.remove(root);
      _overflowFloors.remove(root);
      _summaryRefreshes.cancel(root);
      _setThreadQueryVersion(root, pageVersion);
      _clearDeletionUncertainty(root, pageVersion);
      if (cachedThreadReplyIds(root).length >
          (row.thread?.descendantCount ?? 0)) {
        _queueOverflowSummary(root);
      }
    }
  }

  Iterable<NostrEvent> get _cachedThreadEvents => [
    ..._windowStore.liveOverlay,
    if (!_usingChannelWindow) ...?_lastKnownMessages,
  ];

  List<String> _rotateDeletionProofTargets(Set<String> missingIds) {
    final targets = missingIds.take(20).toList();
    // Reuse the pending-send order instead of retaining a second per-root
    // cursor. It survives route disposal and disappears as sends settle.
    for (final id in targets) {
      final root = _localReplyRoots.remove(id);
      if (root != null) _localReplyRoots[id] = root;
    }
    return targets;
  }

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

  bool _hasVisibleThreadRows(String root) =>
      _isVisibleRoot(root) || _visibleNestedRows(root).isNotEmpty;

  ChannelWindowThreadSummary? _emptyPageSummary(String root) {
    // A page's implicit zero is no longer the only evidence once replies arrive.
    if (cachedThreadReplyIds(root).isNotEmpty) return null;
    for (final page in _windowStore.pages) {
      for (final row in page.rows) {
        if (row.event.id != root) continue;
        if (row.thread != null) return null;
        return const ChannelWindowThreadSummary(
          replyCount: 0,
          descendantCount: 0,
          lastReplyAt: null,
          participantPubkeys: [],
        );
      }
    }
    return null;
  }

  void _applyLiveThreadSummary(NostrEvent event, int? summaryVersion) {
    final root = event.getTagValue('e');
    if (root == null) return;
    final exact = _queryThreadSummaries[root] ?? _emptyPageSummary(root);
    if (summaryVersion == null && exact != null) {
      // WebSocket arrival order cannot establish freshness relative to HTTP
      // scans. Preserve the completed scan and reconcile conflicting counts.
      final incoming = parseChannelWindowThreadSummary(event);
      // Rejected payloads must not regain authority when a later page clears
      // the query overlay (zero-count page summaries are omitted by the relay).
      _windowStore = ChannelWindowStore(
        pages: _windowStore.pages,
        liveOverlay: _windowStore.liveOverlay,
        liveAux: _windowStore.liveAux,
        liveThreadSummaries: {
          for (final entry in _windowStore.liveThreadSummaries.entries)
            if (entry.key != root) entry.key: entry.value,
        },
      );
      if (incoming.replyCount != exact.replyCount ||
          incoming.descendantCount != exact.descendantCount) {
        _queueOverflowSummary(root, countPending: true);
      }
      return;
    }
    _queryThreadSummaries.remove(root);
    _overflowFloors.remove(root);
    _summaryRefreshes.cancel(root);
    final version = summaryVersion ?? ++_threadQuerySerial;
    _setThreadQueryVersion(root, version);
    _clearDeletionUncertainty(root, version);
  }

  Iterable<NostrEvent> _visibleNestedRows(String root) =>
      (_lastKnownMessages ?? const <NostrEvent>[]).where(
        (event) =>
            event.threadReference.rootId == root &&
            event.threadReference.parentId != null &&
            event.tags.any(
              (tag) => tag.length > 1 && tag[0] == 'broadcast' && tag[1] == '1',
            ) &&
            _isVisibleRoot(event.id),
      );

  void _reconcileNestedThreadSummaries(
    String root,
    List<NostrEvent> replies,
    int version,
  ) {
    final rows = _visibleNestedRows(root).toList();
    if (rows.isEmpty) return;
    final entries = buildMainTimelineEntries(
      formatTimeline([
        ...{
          for (final event in [...rows, ...replies]) event.id: event,
        }.values,
      ]),
    );
    for (final row in rows) {
      if ((_threadQueryVersions[row.id] ?? 0) > version) continue;
      final summary = entries
          .where((entry) => entry.message.id == row.id)
          .firstOrNull
          ?.summary;
      _setThreadQueryVersion(row.id, version);
      _clearDeletionUncertainty(row.id, version);
      _overflowFloors.remove(row.id);
      _queryThreadSummaries.remove(row.id);
      _queryThreadSummaries[row.id] = ChannelWindowThreadSummary(
        replyCount: replies
            .where((event) => event.threadReference.parentId == row.id)
            .length,
        descendantCount: summary?.replyCount ?? 0,
        lastReplyAt: summary?.lastReplyAt,
        participantPubkeys: summary?.participantPubkeys ?? const [],
      );
    }
  }

  void _reconcileLiveSummaryPayloads(NostrEvent event) {
    final root = event.getTagValue('e');
    if (root != null) _reconcileThreadSummaryPayloads(root);
  }

  void _reconcileRetainedSummaryPayloads(Iterable<NostrEvent> events) {
    // Evidence can precede the page that first exposes a broadcast branch.
    final roots = events
        .map((event) => event.threadReference.rootId)
        .whereType<String>()
        .where(_windowStore.liveThreadSummaries.containsKey)
        .toSet();
    for (final root in roots) {
      _reconcileThreadSummaryPayloads(root);
    }
  }

  void _reconcileThreadSummaryPayloads(String root) {
    if (!_hasVisibleThreadRows(root)) return;
    final summary = _baseThreadSummaries[root];
    final confirmedIds = cachedThreadReplyIds(root)
      ..removeAll(_localReplyRoots.keys);
    // Relay deletion fan-out updates the outer root only. Its visible broadcast
    // children need the same complete scan to refresh their direct-reply facts.
    final nestedNeedsRecount =
        !_queryThreadSummaries.containsKey(root) &&
        _visibleNestedRows(root).any((row) {
          final nested = _baseThreadSummaries[row.id];
          return nested != null &&
              (nested.replyCount > 0 || nested.descendantCount > 0);
        });
    if (!nestedNeedsRecount &&
        (summary == null || confirmedIds.length <= summary.descendantCount)) {
      return;
    }
    // The summary may lag a new reply, or its deletion marker may have been
    // missed. A fresh complete query resolves either case without discarding
    // valid local payloads merely because a summary count is lower.
    _queueOverflowSummary(root, countPending: true);
  }

  void _reconcileRetainedAggregates(int historyVersion) {
    // Bounded history cannot refresh retained roots outside the newest page.
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
      if (!_hasVisibleThreadRows(root)) {
        _queryThreadSummaries.remove(root);
        _overflowFloors.remove(root);
        _deletionSummaryUncertainty.remove(root);
        _summaryRefreshes.cancel(root);
        continue;
      }
      _queueOverflowSummary(root, countPending: true);
    }
  }

  void _reconcileEvictedReplies(
    Iterable<NostrEvent> replies,
    Set<String> retained,
  ) {
    for (final root
        in replies
            .where((event) => !retained.contains(event.id))
            .map((event) => event.threadReference.rootId)
            .whereType<String>()
            .toSet()) {
      if (!_queryThreadSummaries.containsKey(root)) _queueOverflowSummary(root);
    }
  }

  void _queueOverflowSummary(String root, {bool countPending = false}) {
    if (!_hasVisibleThreadRows(root)) return;
    final ids = cachedThreadReplyIds(root)..removeAll(_localReplyRoots.keys);
    final events =
        _cachedThreadEvents.where((event) => ids.contains(event.id)).toList()
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
    // The outer scan also owns reconciliation of visible broadcast branches.
    // Their old totals are uncertain for the entire wait, including exhaustion.
    for (final row in _visibleNestedRows(root)) {
      final nested = _baseThreadSummaries[row.id];
      if (nested == null) continue;
      _overflowFloors[row.id] = ChannelWindowThreadSummary(
        replyCount: nested.replyCount,
        descendantCount: nested.descendantCount,
        lastReplyAt: nested.lastReplyAt,
        participantPubkeys: nested.participantPubkeys,
        isLowerBound: true,
        isCountPending: true,
      );
    }
    while (_overflowFloors.length > 2048) {
      _overflowFloors.remove(_overflowFloors.keys.first);
    }
    _summaryRefreshes.enqueue(root);
  }

  Future<void> _refreshOverflowSummary(String root, {int attempt = 0}) async {
    if (!_hasVisibleThreadRows(root)) return;
    final generation = _initVersion;
    final version = beginThreadQuery(root);
    final snapshot = cachedThreadReplyIds(root);
    final provisional = unconfirmedThreadReplyIds(root);
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
          provisionalReplyIds: provisional,
          queryVersion: version,
        );
        if (_summaryRefreshes.isDirty(root)) _queueOverflowSummary(root);
      }
    } catch (error) {
      if (!current()) return;
      // Re-sending a timed-out recount re-runs the same slow scan.
      if (attempt < 2 && !isRelayDeadlineError(error)) {
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
                  entry.value.isCountPending &&
                  _hasVisibleThreadRows(entry.key),
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
      if (!_hasVisibleThreadRows(entry.key)) continue;
      _overflowFloors[entry.key] = entry.value;
      // Unknown ownership changes presentation only; never scan every candidate.
      if (owners.containsValue(entry.key)) _queueOverflowSummary(entry.key);
    }
    for (final root in owners.values.toSet()) {
      // An off-window outer root may have no summary, while its broadcast row
      // still needs reconciliation after a known child's deletion.
      if (!floors.containsKey(root)) _queueOverflowSummary(root);
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
