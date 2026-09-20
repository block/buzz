import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel_event_order.dart';
import 'pending_local_messages_provider.dart';
import 'channel_window.dart';
import 'thread_replies_provider.dart';
import 'thread_summary_refresh_queue.dart';
import 'thread_reply_ownership.dart';
import 'timeline_message.dart';

part 'channel_messages_provider/thread_summaries.dart';

// Channel history keeps partial reply evidence; full threads stay route-scoped.
const _maxCachedRepliesPerRoot = 256;
const _maxCachedReplies = 2048;

const _channelLiveEventKinds = [
  ...EventKind.channelEventKinds,
  EventKind.channelThreadSummary,
];

/// Provides the message list for a specific channel. Registers a live
/// subscription first, then syncs history via the server-assembled channel
/// window fast path, falling back to the legacy websocket history path when the
/// relay does not return a valid NIP-CW bounds overlay.
class ChannelMessagesNotifier extends Notifier<AsyncValue<List<NostrEvent>>> {
  final String channelId;
  void Function()? _unsubscribe;
  bool _reachedOldest = false;
  bool _initInFlight = false;
  bool _usingChannelWindow = false;
  bool _initialWindowQueryInFlight = false;
  int _initVersion = 0;
  ChannelWindowStore _windowStore = const ChannelWindowStore.empty();
  final Set<String> _liveSummaryRootsDuringInitialWindowQuery = {};
  final Map<String, NostrEvent> _deepLinkEvents = {};
  final Set<String> _retainedDeepLinkEventIds = {};
  final Map<String, String> _localReplyRoots = {};
  final Map<String, ChannelWindowThreadSummary> _queryThreadSummaries = {};
  final Map<String, ChannelWindowThreadSummary> _overflowFloors = {};
  final Map<String, int> _threadQueryVersions = {};
  final Map<String, int> _threadEvidenceVersions = {};
  int _threadQuerySerial = 0;
  final _deletionSummaryUncertainty = <String, _DeletionSummaryUncertainty>{};
  final _replyOwnership = ThreadReplyOwnership();
  final _processedDeletionTargets = <String>{};
  final _deferredDeletionTargets =
      <String, ({Set<String> candidates, int version})>{};
  late final _deletionOwnerQueue = _DeletionOwnerQueue(
    onCapacity: _drainDeferredDeletionOwners,
    canRun: () =>
        ref.mounted &&
        _hasListeners &&
        ref.read(relaySessionProvider).status == SessionStatus.connected,
  );
  bool _hasListeners = true;
  late final _summaryRefreshes = ThreadSummaryRefreshQueue(
    refresh: _refreshOverflowSummary,
    canRun: () =>
        ref.mounted &&
        _hasListeners &&
        ref.read(relaySessionProvider).status == SessionStatus.connected,
    onSettled: () {
      final events = _lastKnownMessages;
      if (events != null) state = AsyncData(events.toList());
    },
  );

  ChannelMessagesNotifier(this.channelId);

  bool get _summaryMounted => ref.mounted;
  RelaySessionNotifier get _summarySession =>
      ref.read(relaySessionProvider.notifier);

  /// Last successfully loaded messages, preserved across reconnections so the
  /// UI can show stale data instead of a blank loading spinner.
  List<NostrEvent>? _lastKnownMessages;

  /// Whether this channel has completed at least one message history load.
  ///
  /// This distinguishes a genuinely loaded empty channel from the synthetic
  /// empty value returned while the relay is not yet connected.
  bool get hasLoadedMessages => _lastKnownMessages != null;

  Map<String, ChannelWindowThreadSummary> get threadSummaries => {
    for (final entry in _baseThreadSummaries.entries)
      entry.key: _deletionSummaryUncertainty.containsKey(entry.key)
          ? ChannelWindowThreadSummary(
              replyCount: entry.value.replyCount,
              descendantCount: entry.value.descendantCount,
              lastReplyAt: entry.value.lastReplyAt,
              participantPubkeys: entry.value.participantPubkeys,
              isLowerBound: true,
              isCountPending: true,
            )
          : entry.value,
  };

  void _publishSummaryChange() {
    final events = _lastKnownMessages;
    if (events != null) state = AsyncData(events.toList());
  }

  Map<String, ChannelWindowThreadSummary> get _baseThreadSummaries => {
    ...channelWindowThreadSummaries(_windowStore),
    ..._queryThreadSummaries,
    ..._overflowFloors,
  };

  @override
  AsyncValue<List<NostrEvent>> build() {
    final sessionState = ref.watch(relaySessionProvider);
    ref.onCancel(() {
      _hasListeners = false;
      _deletionOwnerQueue.pause();
      _summaryRefreshes.pause();
    });
    ref.onResume(() {
      _hasListeners = true;
      Future.microtask(_summaryRefreshes.resume);
      Future.microtask(_deletionOwnerQueue.resume);
    });
    ref.onDispose(() {
      _initVersion++;
      _deletionOwnerQueue.pause();
      _summaryRefreshes.pause();
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) {
      _initVersion++;
      _initInFlight = false;
      _initialWindowQueryInFlight = false;
      _liveSummaryRootsDuringInitialWindowQuery.clear();
      return AsyncData(_lastKnownMessages ?? const []);
    }

    _reachedOldest = false;
    _windowStore = const ChannelWindowStore.empty();
    // A reconnect window can lag a confirmed thread query and contains only
    // top-level rows. Keep known replies as summary evidence, including their
    // deletion markers so a reset cannot resurrect a deleted reply.
    for (final event in _lastKnownMessages ?? const <NostrEvent>[]) {
      if (event.threadReference.parentId != null ||
          event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent) {
        _mergeWindowEventIntoStore(event);
      }
    }
    _usingChannelWindow = false;
    _initialWindowQueryInFlight = false;
    _liveSummaryRootsDuringInitialWindowQuery.clear();
    _init();
    if (_lastKnownMessages case final cached? when cached.isNotEmpty) {
      return AsyncData(cached);
    }
    return const AsyncLoading();
  }

  Future<void> _init() async {
    final initVersion = ++_initVersion;
    _initInFlight = true;
    _clearSubscription();
    try {
      final session = ref.read(relaySessionProvider.notifier);

      try {
        final unsubscribe = await session.subscribe(
          NostrFilter(
            kinds: _channelLiveEventKinds,
            tags: {
              '#h': [channelId],
            },
            since: _currentUnixSeconds(),
            limit: 200,
          ),
          _handleLiveEvent,
        );
        if (!_isCurrentInit(initVersion)) {
          unsubscribe();
          return;
        }
        _unsubscribe = unsubscribe;
      } catch (error) {
        if (!_isCurrentInit(initVersion)) return;
        debugPrint(
          '[ChannelMessagesNotifier] live subscription failed for $channelId: $error',
        );
      }

      final historyVersion = ++_threadQuerySerial;
      final history = await _fetchNewestHistory(session);
      if (!_isCurrentInit(initVersion)) return;
      _confirmLocalMessages(history.map((event) => event.id));

      final existing = state.value ?? const <NostrEvent>[];
      final existingIds = existing.map((event) => event.id).toSet();
      final merged = _withDeepLinkEvents([
        ...existing,
        ...history.where((event) => existingIds.add(event.id)),
      ]);
      _lastKnownMessages = merged;
      if (!_usingChannelWindow) _reconcileFallbackSummaries(historyVersion);
      state = AsyncData(merged);
      _summaryRefreshes.resume();
    } catch (e, st) {
      if (!_isCurrentInit(initVersion)) return;
      final fallbackMessages = state.value ?? _lastKnownMessages;
      if (fallbackMessages != null) {
        debugPrint(
          '[ChannelMessagesNotifier] history sync failed for $channelId: $e',
        );
        state = AsyncData(fallbackMessages);
        return;
      }
      state = AsyncError(e, st);
    } finally {
      if (_isCurrentInit(initVersion)) {
        _initInFlight = false;
        _deletionOwnerQueue.resume();
      }
    }
  }

  Future<List<NostrEvent>> _fetchNewestHistory(
    RelaySessionNotifier session,
  ) async {
    try {
      _initialWindowQueryInFlight = true;
      final pageVersion = ++_threadQuerySerial;
      final page = await _fetchWindowPage(session, null);
      _initialWindowQueryInFlight = false;
      _windowStore = replaceNewestChannelWindow(
        _windowStore,
        page,
        retainLiveSummaryRootIds: _liveSummaryRootsDuringInitialWindowQuery,
      );
      _liveSummaryRootsDuringInitialWindowQuery.clear();
      _pruneOffWindowReplies();
      _usingChannelWindow = true;
      _reconcilePageSummaries(page, pageVersion);
      _reachedOldest = !channelWindowHasMore(_windowStore);
      return flattenChannelWindowEvents(_windowStore);
    } catch (error) {
      _initialWindowQueryInFlight = false;
      _liveSummaryRootsDuringInitialWindowQuery.clear();
      debugPrint(
        '[ChannelMessagesNotifier] channel window unavailable for $channelId, falling back to WS history: $error',
      );
      _usingChannelWindow = false;
      final history = await session.fetchHistory(
        NostrFilters.messages(channelId),
      );
      history.sort(compareChannelTimelineEventsChronologically);
      return history;
    }
  }

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

  Future<ChannelWindowPage> _fetchWindowPage(
    RelaySessionNotifier session,
    ChannelPageCursor? cursor,
  ) async {
    final events = await session.queryRelay([_channelWindowFilter(cursor)]);
    return parseChannelWindowResponse(events, channelId, cursor);
  }

  NostrFilter _channelWindowFilter(ChannelPageCursor? cursor) => NostrFilter(
    kinds: EventKind.channelTimelineContentKinds,
    tags: {
      '#h': [channelId],
    },
    limit: 50,
    until: cursor?.createdAt,
    extensions: {
      'top_level': true,
      'include_summaries': true,
      'include_aux': true,
      if (cursor != null) 'before_id': cursor.eventId,
    },
  );

  void _handleLiveEvent(
    NostrEvent event, {
    bool authoritative = true,
    int? summaryVersion,
  }) {
    final before = _lastKnownMessages ?? const <NostrEvent>[];
    _replyOwnership.record([event]);
    // Invalidate the thread query independently of the selected channel-history
    // path. The websocket fallback does not merge through the window store.
    _invalidateThreadReplies(event);
    // A live summary can race the initial channel-window query. Buffer it in
    // the window store even before that query installs its first page, rather
    // than treating metadata as an ordinary websocket timeline event.
    if (event.kind == EventKind.channelThreadSummary && !_usingChannelWindow) {
      final rootId = _initialWindowQueryInFlight
          ? event.getTagValue('e')
          : null;
      if (_mergeWindowEventIntoStore(event, summaryVersion: summaryVersion)) {
        if (rootId != null) {
          _liveSummaryRootsDuringInitialWindowQuery.add(rootId);
        }
        if (_initInFlight) return;
        final current =
            state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
        _lastKnownMessages = current;
        state = AsyncData(current);
      }
      return;
    }

    if (_usingChannelWindow) {
      _handleWindowLiveEvent(event, summaryVersion: summaryVersion);
    } else {
      final current = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
      final merged = _boundEventReplies(_mergeEvent(current, event));
      _lastKnownMessages = merged;
      state = AsyncData(merged);
    }
    final rootId = event.threadReference.rootId;
    if (event.threadReference.parentId != null &&
        rootId != null &&
        cachedThreadReplyIds(rootId).length >= _maxCachedRepliesPerRoot) {
      _queueOverflowSummary(rootId);
    }
    if (authoritative) {
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent) {
        final targets = event.tags
            .where((tag) => tag.length > 1 && tag[0] == 'e')
            .map((tag) => tag[1])
            .toSet();
        _refreshDeletedSummaries(targets, before);
      }
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent) {
        _confirmIndexedLocalReplies(
          event.tags
              .where((tag) => tag.length > 1 && tag[0] == 'e')
              .map((tag) => tag[1]),
        );
      }
      _localReplyRoots.remove(event.id);
      // Store the reply first so clearing its overlay cannot disable its root.
      _confirmLocalMessages([event.id]);
      final thread = event.threadReference;
      if (thread.parentId != null && thread.rootId != null) {
        final localReplies = threadLocalRepliesProvider(
          ThreadRepliesArgs(channelId: channelId, rootId: thread.rootId!),
        );
        if (ref.exists(localReplies)) {
          ref.read(localReplies.notifier).confirm({event.id});
        }
      }
    }
  }

  void _handleWindowLiveEvent(NostrEvent event, {int? summaryVersion}) {
    if (!_mergeWindowEventIntoStore(event, summaryVersion: summaryVersion)) {
      return;
    }
    final windowEvents = flattenChannelWindowEvents(_windowStore);
    // Flattening already orders the window. Only merge and sort again when
    // there are retained deep-link events to include.
    final flattened = _deepLinkEvents.isEmpty
        ? windowEvents
        : _withDeepLinkEvents(windowEvents);
    _lastKnownMessages = flattened;
    state = AsyncData(flattened);
  }

  void _invalidateThreadReplies(NostrEvent event) {
    if (!EventKind.channelTimelineContentKinds.contains(event.kind)) return;
    final thread = event.threadReference;
    if (thread.parentId == null) return;

    final rootId = thread.rootId;
    if (rootId != null) {
      ref.invalidate(
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: channelId, rootId: rootId),
        ),
      );
    }
    final parentId = thread.parentId;
    if (parentId != null && parentId != rootId) {
      ref.invalidate(
        threadRepliesProvider(
          ThreadRepliesArgs(channelId: channelId, rootId: parentId),
        ),
      );
    }
  }

  bool _mergeWindowEventIntoStore(NostrEvent event, {int? summaryVersion}) {
    final isTimelineRow = EventKind.channelTimelineContentKinds.contains(
      event.kind,
    );
    final thread = isTimelineRow ? event.threadReference : null;
    if (thread?.parentId != null) {
      // Replies are kept in the store rather than dropped here, matching
      // desktop: the main timeline filters them out at render
      // (`buildMainTimelineEntries`), and their parent's "N replies" row needs
      // them as the local half of the summary merge when the relay's
      // best-effort recount is delayed, lost, or older than this reply.
    }
    // Thread summaries are neither a timeline row nor an aux event, but they are
    // how the root's "N replies" row learns a reply landed — a reply itself
    // never reaches the main timeline. Dropping them here meant the count only
    // appeared after leaving the channel and coming back, which refetched.
    if (!isTimelineRow &&
        event.kind != EventKind.channelThreadSummary &&
        !EventKind.channelAuxEventKinds.contains(event.kind)) {
      return false;
    }

    final next = mergeLiveChannelWindowEvent(
      _windowStore,
      event,
      isTimelineRow: isTimelineRow,
      // Deep-link roots stay visible independently of the newest window.
      // Keep their reply evidence under the same payload bounds as other roots.
      retainOutsideWindow:
          thread?.parentId != null &&
          _retainedDeepLinkEventIds.contains(thread?.rootId),
    );
    if (identical(next, _windowStore)) return false;
    _windowStore = next;
    if (event.kind == EventKind.channelThreadSummary) {
      _applyLiveThreadSummary(event, summaryVersion);
    }
    _trimReplyOverlay();
    if (event.kind == EventKind.channelThreadSummary) {
      _reconcileLiveSummaryPayloads(event);
    }
    return true;
  }

  List<NostrEvent> _boundedReplies(Iterable<NostrEvent> events) {
    final sorted = events.toList()
      ..sort((a, b) => compareThreadRepliesChronologically(b, a));
    final counts = <String?, int>{};
    final seen = <String>{};
    final retained = <NostrEvent>[];
    for (final event in sorted) {
      if (!seen.add(event.id)) continue;
      final root = event.threadReference.rootId;
      final count = counts[root] ?? 0;
      if (count >= _maxCachedRepliesPerRoot) continue;
      retained.add(event);
      counts[root] = count + 1;
      if (retained.length == _maxCachedReplies) break;
    }
    return retained;
  }

  List<NostrEvent> _boundEventReplies(List<NostrEvent> events) {
    final replies = events.where(
      (event) =>
          EventKind.channelTimelineContentKinds.contains(event.kind) &&
          event.threadReference.parentId != null,
    );
    final retained = _boundedReplies(replies).map((event) => event.id).toSet();
    return events
        .where(
          (event) =>
              !EventKind.channelTimelineContentKinds.contains(event.kind) ||
              event.threadReference.parentId == null ||
              retained.contains(event.id),
        )
        .toList();
  }

  void _trimReplyOverlay() {
    final replies = _windowStore.liveOverlay
        .where((event) => event.threadReference.parentId != null)
        .toList();
    if (replies.length <= _maxCachedRepliesPerRoot) return;
    final retained = _boundedReplies(replies).map((event) => event.id).toSet();
    if (retained.length == replies.length) return;
    for (final root
        in replies
            .where((event) => !retained.contains(event.id))
            .map((event) => event.threadReference.rootId)
            .whereType<String>()
            .toSet()) {
      if (!_queryThreadSummaries.containsKey(root)) _queueOverflowSummary(root);
    }
    _windowStore = ChannelWindowStore(
      pages: _windowStore.pages,
      liveOverlay: _windowStore.liveOverlay
          .where(
            (event) =>
                event.threadReference.parentId == null ||
                retained.contains(event.id),
          )
          .toList(),
      liveAux: _windowStore.liveAux,
      liveThreadSummaries: _windowStore.liveThreadSummaries,
    );
  }

  void _confirmIndexedLocalReplies(Iterable<String> ids) {
    final targets = ids.toSet();
    for (final id in targets) {
      final root = _localReplyRoots.remove(id);
      if (root == null) continue;
      final provider = threadLocalRepliesProvider(
        ThreadRepliesArgs(channelId: channelId, rootId: root),
      );
      if (ref.exists(provider)) ref.read(provider.notifier).confirm({id});
    }
    _confirmLocalMessages(targets);
  }

  void _confirmLocalMessages(Iterable<String> eventIds) {
    ref
        .read(pendingLocalMessagesProvider(channelId).notifier)
        .confirm(eventIds);
  }

  /// Reserves a version for a thread query without treating it as evidence.
  int beginThreadQuery(String rootId) => _reserveThreadQuery(rootId);

  /// Releases a failed or disposed query while preserving newer evidence.
  void failThreadQuery(String rootId, int version) =>
      _retireThreadQuery(rootId, version);

  /// Snapshots unacknowledged replies that still require explicit deletion proof.
  Set<String> unconfirmedThreadReplyIds(String rootId) => {
    for (final entry in _localReplyRoots.entries)
      if (entry.value == rootId) entry.key,
  };

  /// Reserves the next bounded proof batch, rotating pending sends fairly.
  List<String> nextThreadDeletionProofTargets(Set<String> missingIds) =>
      _rotateDeletionProofTargets(missingIds);

  /// Snapshots cached overlay replies covered by a query for this outer root.
  Set<String> cachedThreadReplyIds(String rootId) {
    final deletedIds = {
      for (final event in [
        ..._windowStore.liveAux,
        if (!_usingChannelWindow) ...?_lastKnownMessages,
        for (final page in _windowStore.pages) ...page.aux,
      ])
        if (event.kind == EventKind.deletion ||
            event.kind == EventKind.nip29DeleteEvent)
          for (final tag in event.tags)
            if (tag.length > 1 && tag[0] == 'e') tag[1],
    };
    return {
      for (final entry in _localReplyRoots.entries)
        if (entry.value == rootId && !deletedIds.contains(entry.key)) entry.key,
      for (final event in _cachedThreadEvents)
        if (event.threadReference.parentId != null &&
            event.threadReference.rootId == rootId &&
            !deletedIds.contains(event.id))
          event.id,
    };
  }

  /// Applies explicit deletion evidence fetched for cached replies.
  /// [scopedTargetIds] are targets of the authenticated channel-scoped query;
  /// a matching target permits kind-5 wire events without an h tag, including
  /// multi-target markers that also name events outside this query.
  /// [reconciledTargetIds] were already excluded by an applied complete scan.
  void cacheThreadDeletions(
    Iterable<NostrEvent> deletions, {
    Set<String> scopedTargetIds = const {},
    Set<String> reconciledTargetIds = const {},
  }) {
    var events = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
    for (final event in deletions) {
      final targets = event.tags
          .where((tag) => tag.length > 1 && tag[0] == 'e')
          .map((tag) => tag[1])
          .toSet();
      final scopedStandardDeletion =
          event.kind == EventKind.deletion &&
          event.channelId == null &&
          targets.isNotEmpty &&
          targets.any(scopedTargetIds.contains);
      if ((!scopedStandardDeletion && event.channelId != channelId) ||
          (event.kind != EventKind.deletion &&
              event.kind != EventKind.nip29DeleteEvent)) {
        throw StateError('Expected a deletion in channel $channelId.');
      }
      _retireDeferredDeletionTargets(targets.intersection(reconciledTargetIds));
      _rememberDeletionTargets(targets.intersection(reconciledTargetIds));
      final before = events;
      _mergeWindowEventIntoStore(event);
      events = _mergeEvent(events, event);
      _refreshDeletedSummaries({
        for (final tag in event.tags)
          if (tag.length > 1 && tag[0] == 'e') tag[1],
      }, before);
      _confirmIndexedLocalReplies(
        event.tags
            .where((tag) => tag.length > 1 && tag[0] == 'e')
            .map((tag) => tag[1]),
      );
    }
    _lastKnownMessages = events;
    state = AsyncData(events);
  }

  void _pruneOffWindowReplies() {
    _rememberDeletionTargets({
      for (final event in [
        ..._windowStore.liveAux,
        for (final page in _windowStore.pages) ...page.aux,
      ])
        if (event.kind == EventKind.deletion ||
            event.kind == EventKind.nip29DeleteEvent)
          for (final tag in event.tags)
            if (tag.length > 1 && tag[0] == 'e') tag[1],
    });
    final roots = {
      for (final page in _windowStore.pages)
        for (final row in page.rows) row.event.id,
      for (final event in _windowStore.liveOverlay)
        if (event.threadReference.parentId == null) event.id,
      ..._retainedDeepLinkEventIds,
    };
    bool keepReply(NostrEvent event) =>
        event.threadReference.parentId == null ||
        roots.contains(event.threadReference.rootId);
    final overlay = _windowStore.liveOverlay.where(keepReply).toList();
    final retainedIds = {...roots, ...overlay.map((event) => event.id)};
    _windowStore = ChannelWindowStore(
      pages: _windowStore.pages,
      liveOverlay: overlay,
      liveAux: _windowStore.liveAux
          .where(
            (event) => event.tags.any(
              (tag) =>
                  tag.length > 1 &&
                  tag[0] == 'e' &&
                  retainedIds.contains(tag[1]),
            ),
          )
          .toList(),
      liveThreadSummaries: _windowStore.liveThreadSummaries,
    );
    // Keep the currently displayed snapshot intact until the next window
    // publication drops off-window roots and their reply evidence together.
  }

  /// Publishes an insertion-complete thread scan, preserving aggregate facts
  /// before payload eviction. Only absent pre-query, accepted IDs are removed;
  /// in-flight sends and arrivals after the query began remain provisional.
  /// Returns whether this scan applied, rather than losing to newer evidence.
  bool cacheCompleteThreadQuery(
    String rootId,
    Set<String> queriedIds,
    List<NostrEvent> replies, {
    Set<String> provisionalReplyIds = const {},
    int? queryVersion,
  }) {
    if (queryVersion != null && _threadQueryVersions[rootId] != queryVersion) {
      return false;
    }
    final evidenceVersion = queryVersion ?? ++_threadQuerySerial;
    _setThreadQueryVersion(rootId, evidenceVersion);
    _overflowFloors.remove(rootId);
    _clearDeletionUncertainty(rootId, evidenceVersion);
    final resultIds = replies.map((event) => event.id).toSet();
    final missing = queriedIds.difference(resultIds)
      ..removeAll(_localReplyRoots.keys)
      ..removeAll(provisionalReplyIds);
    _rememberDeletionTargets(missing);
    _retireDeferredDeletionTargets(missing);
    final retainedIds = cachedThreadReplyIds(rootId);
    final later = _cachedThreadEvents
        .where(
          (event) =>
              event.threadReference.rootId == rootId &&
              retainedIds.contains(event.id) &&
              (!queriedIds.contains(event.id) ||
                  provisionalReplyIds.contains(event.id)) &&
              !resultIds.contains(event.id),
        )
        .toList();
    _windowStore = ChannelWindowStore(
      pages: _windowStore.pages,
      liveOverlay: _windowStore.liveOverlay
          .where((event) => !missing.contains(event.id))
          .toList(),
      liveAux: _windowStore.liveAux,
      liveThreadSummaries: _windowStore.liveThreadSummaries,
    );
    final current = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
    final remaining = current
        .where((event) => !missing.contains(event.id))
        .toList();
    _lastKnownMessages = remaining;
    state = AsyncData(remaining);
    final aggregate = {
      ...{for (final event in replies) event.id: event},
      ...{for (final event in later) event.id: event},
    }.values.toList()..sort(compareThreadRepliesChronologically);
    _queryThreadSummaries.remove(rootId);
    _queryThreadSummaries[rootId] = ChannelWindowThreadSummary(
      replyCount: aggregate
          .where((event) => event.threadReference.parentId == rootId)
          .length,
      descendantCount: aggregate.length,
      lastReplyAt: aggregate.isEmpty ? null : aggregate.last.createdAt,
      participantPubkeys: aggregate.reversed
          .map((event) => event.pubkey)
          .toSet()
          .take(5)
          .toList(),
    );
    _reconcileNestedThreadSummaries(rootId, aggregate, evidenceVersion);
    // Metadata is small but still bounded independently of payload retention.
    while (_queryThreadSummaries.length > 2048) {
      _queryThreadSummaries.remove(_queryThreadSummaries.keys.first);
    }
    cacheConfirmedThreadReplies(replies);
    return true;
  }

  /// Caches confirmed thread replies before their optimistic overlay is cleared.
  /// Thread queries must not invalidate themselves as live relay events do.
  void cacheConfirmedThreadReplies(Iterable<NostrEvent> replies) {
    _replyOwnership.record(replies);
    var events = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
    for (final reply in _boundedReplies(replies)) {
      if (reply.channelId != channelId ||
          reply.threadReference.parentId == null) {
        throw StateError('Expected a reply in channel $channelId.');
      }
      _mergeWindowEventIntoStore(reply);
      if (!_usingChannelWindow) {
        events = _mergeEvent(events, reply);
      }
    }
    if (_usingChannelWindow) {
      events = _withDeepLinkEvents(flattenChannelWindowEvents(_windowStore));
    }
    events = _boundEventReplies(events);
    _lastKnownMessages = events;
    state = AsyncData(events);
    _confirmIndexedLocalReplies(replies.map((event) => event.id));
  }

  /// Adds a just-signed outgoing message before the relay acknowledges it.
  /// The live relay echo is deduplicated by event id.
  void addLocalMessage(NostrEvent event) {
    ref.read(pendingLocalMessagesProvider(channelId).notifier).add(event);
    final thread = event.threadReference;
    if (thread.parentId != null) {
      final rootId = thread.rootId;
      if (rootId == null) {
        throw StateError('Reply ${event.id} has a parent but no thread root.');
      }
      _localReplyRoots[event.id] = rootId;
      ref
          .read(
            threadLocalRepliesProvider(
              ThreadRepliesArgs(channelId: channelId, rootId: rootId),
            ).notifier,
          )
          .add(event);
      return;
    }

    final isTimelineRow = EventKind.channelTimelineContentKinds.contains(
      event.kind,
    );
    if (!_usingChannelWindow && isTimelineRow) {
      _windowStore = mergeLiveChannelWindowEvent(
        _windowStore,
        event,
        isTimelineRow: true,
      );
    }
    _handleLiveEvent(event, authoritative: false);
  }

  /// Releases rollback ownership after the publish future succeeds.
  /// Accepted replies move into the bounded channel cache even if the relay's
  /// EVENT echo never arrives, so an unobserved thread overlay can dispose.
  void completeLocalMessage(String eventId) {
    final accepted = ref
        .read(pendingLocalMessagesProvider(channelId).notifier)
        .take(eventId);
    if (accepted?.threadReference.parentId != null) {
      cacheConfirmedThreadReplies([accepted!]);
      final root = accepted.threadReference.rootId;
      if (root != null &&
          cachedThreadReplyIds(root).length >= _maxCachedRepliesPerRoot) {
        _queueOverflowSummary(root);
      }
    }
  }

  /// Rolls back a local message when its publish is rejected or times out.
  void removeLocalMessage(String eventId) {
    final pending = ref
        .read(pendingLocalMessagesProvider(channelId).notifier)
        .take(eventId);
    if (pending == null) return;

    _localReplyRoots.remove(eventId);
    final thread = pending.threadReference;
    if (thread.parentId != null) {
      final rootId = thread.rootId;
      if (rootId == null) {
        throw StateError('Reply $eventId has a parent but no thread root.');
      }
      ref
          .read(
            threadLocalRepliesProvider(
              ThreadRepliesArgs(channelId: channelId, rootId: rootId),
            ).notifier,
          )
          .remove(eventId);
      return;
    }

    final nextOverlay = _windowStore.liveOverlay
        .where((event) => event.id != eventId)
        .toList();
    if (nextOverlay.length != _windowStore.liveOverlay.length) {
      _windowStore = ChannelWindowStore(
        pages: _windowStore.pages,
        liveOverlay: nextOverlay,
        liveAux: _windowStore.liveAux,
        liveThreadSummaries: _windowStore.liveThreadSummaries,
      );
    }

    final current = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
    final next = current.where((event) => event.id != eventId).toList();
    _lastKnownMessages = next;
    state = AsyncData(next);
  }

  static List<NostrEvent> _mergeEvent(
    List<NostrEvent> current,
    NostrEvent incoming,
  ) {
    if (current.any((e) => e.id == incoming.id)) return current;
    final updated = [...current, incoming];
    updated.sort(compareChannelTimelineEventsChronologically);
    return updated;
  }

  bool _isCurrentInit(int initVersion) => initVersion == _initVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }

  bool get reachedOldest => _reachedOldest;

  /// Loads specific deep-link targets that may fall outside the newest window.
  Future<void> loadEventsById(Iterable<String> eventIds) async {
    final ids = eventIds.where((id) => id.isNotEmpty).toSet();
    if (ids.isEmpty) return;
    _retainedDeepLinkEventIds.addAll(ids);

    final existing = state.value ?? const <NostrEvent>[];
    for (final event in existing) {
      if (ids.contains(event.id)) _deepLinkEvents[event.id] = event;
    }
    ids.removeAll(_deepLinkEvents.keys);
    if (ids.isEmpty) return;

    final events = await ref
        .read(relaySessionProvider.notifier)
        .fetchHistory(
          NostrFilter(
            kinds: EventKind.channelTimelineContentKinds,
            ids: ids.toList(),
            limit: ids.length,
          ),
        );
    for (final event in events) {
      if (event.channelId == channelId &&
          _retainedDeepLinkEventIds.contains(event.id)) {
        _deepLinkEvents[event.id] = event;
      }
    }

    // Let the initial history load publish the complete timeline once it
    // finishes. Publishing a target-only list here would make the UI consume
    // its one-shot jump against a provisional ordering.
    if (_initInFlight) return;
    final merged = _withDeepLinkEvents(
      state.value ?? _lastKnownMessages ?? const [],
    );
    _lastKnownMessages = merged;
    state = AsyncData(merged);
  }

  /// Stops pinning deep-link-only events into subsequent window rebuilds.
  void releaseDeepLinkEvents(Iterable<String> eventIds) {
    for (final id in eventIds) {
      _retainedDeepLinkEventIds.remove(id);
      _deepLinkEvents.remove(id);
    }
  }

  List<NostrEvent> _withDeepLinkEvents(List<NostrEvent> events) {
    final ids = events.map((event) => event.id).toSet();
    return [
      ...events,
      ..._deepLinkEvents.values.where((event) => ids.add(event.id)),
    ]..sort(compareChannelTimelineEventsChronologically);
  }

  Future<bool> fetchOlder() async {
    if (_reachedOldest || _initInFlight) return false;

    final session = ref.read(relaySessionProvider.notifier);
    if (_usingChannelWindow) {
      final cursor = channelWindowNextCursor(_windowStore);
      if (cursor == null) {
        _reachedOldest = true;
        return false;
      }
      try {
        final pageVersion = ++_threadQuerySerial;
        final page = await _fetchWindowPage(session, cursor);
        _windowStore = appendOlderChannelWindow(_windowStore, page);
        _reconcilePageSummaries(page, pageVersion);
        _reachedOldest = !channelWindowHasMore(_windowStore);
        final flattened = _withDeepLinkEvents(
          flattenChannelWindowEvents(_windowStore),
        );
        _lastKnownMessages = flattened;
        state = AsyncData(flattened);
        return page.rows.isNotEmpty || page.aux.isNotEmpty;
      } catch (error) {
        debugPrint(
          '[ChannelMessagesNotifier] failed to fetch older channel window page for $channelId: $error',
        );
        return false;
      }
    }

    final currentEvents = state.value;
    if (currentEvents == null || currentEvents.isEmpty) return false;
    final oldest = currentEvents.first.createdAt;
    final older = await session.fetchHistory(
      NostrFilters.messages(channelId, limit: 100, until: oldest),
    );
    if (older.isEmpty) {
      _reachedOldest = true;
      return false;
    }
    final currentIds = state.value?.map((e) => e.id).toSet() ?? {};
    final deduped = older.where((e) => !currentIds.contains(e.id)).toList();
    if (deduped.isEmpty) {
      _reachedOldest = true;
      return false;
    }
    state = state.whenData((events) {
      final merged = [...deduped, ...events];
      merged.sort(compareChannelTimelineEventsChronologically);
      _lastKnownMessages = merged;
      return merged;
    });
    return true;
  }
}

int _currentUnixSeconds() => DateTime.now().millisecondsSinceEpoch ~/ 1000;

final channelMessagesProvider =
    NotifierProvider.family<
      ChannelMessagesNotifier,
      AsyncValue<List<NostrEvent>>,
      String
    >(ChannelMessagesNotifier.new);
