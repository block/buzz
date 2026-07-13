import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';
import 'channel_window.dart';
import 'thread_replies_provider.dart';

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
  int _initVersion = 0;
  ChannelWindowStore _windowStore = const ChannelWindowStore.empty();

  ChannelMessagesNotifier(this.channelId);

  /// Last successfully loaded messages, preserved across reconnections so the
  /// UI can show stale data instead of a blank loading spinner.
  List<NostrEvent>? _lastKnownMessages;

  Map<String, ChannelWindowThreadSummary> get threadSummaries =>
      channelWindowThreadSummaries(_windowStore);

  @override
  AsyncValue<List<NostrEvent>> build() {
    final sessionState = ref.watch(relaySessionProvider);
    ref.onDispose(() {
      _initVersion++;
      _clearSubscription();
    });

    if (sessionState.status != SessionStatus.connected) {
      _initVersion++;
      _initInFlight = false;
      return AsyncData(_lastKnownMessages ?? const []);
    }

    _reachedOldest = false;
    _windowStore = const ChannelWindowStore.empty();
    _usingChannelWindow = false;
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
            kinds: EventKind.channelEventKinds,
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

      final history = await _fetchNewestHistory(session);
      if (!_isCurrentInit(initVersion)) return;

      final existing = state.value ?? const <NostrEvent>[];
      final existingIds = existing.map((event) => event.id).toSet();
      final merged = [
        ...existing,
        ...history.where((event) => !existingIds.contains(event.id)),
      ]..sort((a, b) => a.createdAt.compareTo(b.createdAt));
      _lastKnownMessages = merged;
      state = AsyncData(merged);
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
      }
    }
  }

  Future<List<NostrEvent>> _fetchNewestHistory(
    RelaySessionNotifier session,
  ) async {
    try {
      final page = await _fetchWindowPage(session, null);
      _windowStore = replaceNewestChannelWindow(_windowStore, page);
      _usingChannelWindow = true;
      _reachedOldest = !channelWindowHasMore(_windowStore);
      return flattenChannelWindowEvents(_windowStore);
    } catch (error) {
      debugPrint(
        '[ChannelMessagesNotifier] channel window unavailable for $channelId, falling back to WS history: $error',
      );
      _usingChannelWindow = false;
      final history = await session.fetchHistory(
        NostrFilters.messages(channelId),
      );
      history.sort((a, b) => a.createdAt.compareTo(b.createdAt));
      return history;
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

  void _handleLiveEvent(NostrEvent event) {
    if (_usingChannelWindow) {
      _handleWindowLiveEvent(event);
    } else {
      final current = state.value ?? _lastKnownMessages ?? const <NostrEvent>[];
      final merged = _mergeEvent(current, event);
      _lastKnownMessages = merged;
      state = AsyncData(merged);
    }

    if (event.kind == EventKind.systemMessage &&
        _isMembershipEvent(event.content)) {
      ref.invalidate(channelMembersProvider(channelId));
    }
  }

  void _handleWindowLiveEvent(NostrEvent event) {
    if (!_mergeWindowEventIntoStore(event)) return;
    final flattened = flattenChannelWindowEvents(_windowStore);
    _lastKnownMessages = flattened;
    state = AsyncData(flattened);
  }

  bool _mergeWindowEventIntoStore(NostrEvent event) {
    final isTimelineRow = EventKind.channelTimelineContentKinds.contains(
      event.kind,
    );
    final thread = isTimelineRow ? event.threadReference : null;
    if (thread?.parentId != null) {
      final rootId = thread?.rootId;
      if (rootId != null) {
        ref.invalidate(
          threadRepliesProvider(
            ThreadRepliesArgs(channelId: channelId, rootId: rootId),
          ),
        );
      }
      final parentId = thread?.parentId;
      if (parentId != null && parentId != rootId) {
        ref.invalidate(
          threadRepliesProvider(
            ThreadRepliesArgs(channelId: channelId, rootId: parentId),
          ),
        );
      }
      if (!_isBroadcastReply(event)) return false;
    }
    if (!isTimelineRow &&
        !EventKind.channelAuxEventKinds.contains(event.kind)) {
      return false;
    }

    final next = mergeLiveChannelWindowEvent(
      _windowStore,
      event,
      isTimelineRow: isTimelineRow,
    );
    if (identical(next, _windowStore)) return false;
    _windowStore = next;
    return true;
  }

  static bool _isMembershipEvent(String content) {
    return content.contains('member_joined') ||
        content.contains('member_left') ||
        content.contains('member_removed');
  }

  static List<NostrEvent> _mergeEvent(
    List<NostrEvent> current,
    NostrEvent incoming,
  ) {
    if (current.any((e) => e.id == incoming.id)) return current;
    final updated = [...current, incoming];
    updated.sort((a, b) => a.createdAt.compareTo(b.createdAt));
    return updated;
  }

  bool _isCurrentInit(int initVersion) => initVersion == _initVersion;

  void _clearSubscription() {
    _unsubscribe?.call();
    _unsubscribe = null;
  }

  bool get reachedOldest => _reachedOldest;

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
        final page = await _fetchWindowPage(session, cursor);
        _windowStore = appendOlderChannelWindow(_windowStore, page);
        _reachedOldest = !channelWindowHasMore(_windowStore);
        final flattened = flattenChannelWindowEvents(_windowStore);
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
      merged.sort((a, b) => a.createdAt.compareTo(b.createdAt));
      _lastKnownMessages = merged;
      return merged;
    });
    return true;
  }
}

bool _isBroadcastReply(NostrEvent event) {
  return event.tags.any(
    (tag) => tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1',
  );
}

int _currentUnixSeconds() => DateTime.now().millisecondsSinceEpoch ~/ 1000;

final channelMessagesProvider =
    NotifierProvider.family<
      ChannelMessagesNotifier,
      AsyncValue<List<NostrEvent>>,
      String
    >(ChannelMessagesNotifier.new);
