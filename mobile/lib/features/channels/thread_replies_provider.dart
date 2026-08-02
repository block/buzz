import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'pending_local_messages_provider.dart';

class ThreadRepliesArgs {
  final String channelId;
  final String rootId;

  const ThreadRepliesArgs({required this.channelId, required this.rootId});

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is ThreadRepliesArgs &&
          channelId == other.channelId &&
          rootId == other.rootId;

  @override
  int get hashCode => Object.hash(channelId, rootId);
}

class _ThreadCursor {
  final int createdAt;
  final String eventId;

  const _ThreadCursor({required this.createdAt, required this.eventId});
}

/// Provides one thread's replies from a subscribe-first live stream merged
/// with paged history.
///
/// Registering the live subscription before querying history closes the race
/// where a reply can arrive between those two operations. The subscription is
/// owned by [RelaySessionNotifier], so it stays registered while that session
/// reconnects and benefits from its last-seen replay.
class ThreadRepliesNotifier extends AsyncNotifier<List<NostrEvent>> {
  static const _replaySkewSeconds = 5;

  final ThreadRepliesArgs args;
  final Duration _retryBaseDelay;
  final Map<String, NostrEvent> _repliesById = {};
  void Function()? _unsubscribe;
  Future<bool>? _subscribeFuture;
  Future<void>? _historyFuture;
  Timer? _retryTimer;
  int _retryAttempt = 0;
  bool _disposed = false;

  ThreadRepliesNotifier(
    this.args, {
    @visibleForTesting Duration retryBaseDelay = const Duration(seconds: 2),
  }) : _retryBaseDelay = retryBaseDelay;

  @override
  Future<List<NostrEvent>> build() async {
    _disposed = false;
    ref.onDispose(() {
      _disposed = true;
      _retryTimer?.cancel();
      _retryTimer = null;
      _unsubscribe?.call();
      _unsubscribe = null;
    });

    final session = ref.read(relaySessionProvider.notifier);
    await _ensureSubscribed(session);

    try {
      await _ensureHistory(session);
    } catch (error, stackTrace) {
      if (_disposed) return _sortedReplies();
      if (_repliesById.isEmpty) {
        Error.throwWithStackTrace(error, stackTrace);
      }
      debugPrint(
        '[ThreadRepliesNotifier] history sync failed for '
        '${args.rootId}: $error',
      );
    }
    return _sortedReplies();
  }

  Set<String> get authoritativeEventIds => Set.unmodifiable(_repliesById.keys);

  Future<bool> _ensureSubscribed(RelaySessionNotifier session) async {
    if (_disposed) return false;
    if (_unsubscribe != null) return true;
    final inFlight = _subscribeFuture;
    if (inFlight != null) return inFlight;

    final attempt = _startSubscription(session);
    _subscribeFuture = attempt;
    try {
      return await attempt;
    } finally {
      if (identical(_subscribeFuture, attempt)) {
        _subscribeFuture = null;
      }
    }
  }

  Future<bool> _startSubscription(RelaySessionNotifier session) async {
    var closedBeforeReady = false;
    try {
      final unsubscribe = await session.subscribe(
        _threadLiveRepliesFilter(args),
        _handleReply,
        onClosed: (message) {
          closedBeforeReady = true;
          _handleSubscriptionClosed(session, message);
        },
      );
      if (_disposed || closedBeforeReady) {
        unsubscribe();
        return false;
      }
      _unsubscribe = unsubscribe;
      _retryTimer?.cancel();
      _retryTimer = null;
      _retryAttempt = 0;
      return true;
    } catch (error) {
      if (!_disposed) {
        debugPrint(
          '[ThreadRepliesNotifier] live subscription failed for '
          '${args.rootId}: $error',
        );
        _scheduleSubscriptionRetry(session);
      }
      return false;
    }
  }

  void _handleSubscriptionClosed(RelaySessionNotifier session, String message) {
    if (_disposed) return;
    debugPrint(
      '[ThreadRepliesNotifier] live subscription closed for '
      '${args.rootId}: $message',
    );
    _unsubscribe = null;
    _scheduleSubscriptionRetry(session);
  }

  void _scheduleSubscriptionRetry(RelaySessionNotifier session) {
    if (_disposed) return;
    _retryTimer?.cancel();
    final delayMs = min(
      _retryBaseDelay.inMilliseconds << min(_retryAttempt, 4),
      30000,
    );
    _retryAttempt++;
    _retryTimer = Timer(Duration(milliseconds: delayMs), () {
      _retryTimer = null;
      unawaited(_retrySubscription(session));
    });
  }

  Future<void> _retrySubscription(RelaySessionNotifier session) async {
    if (!await _ensureSubscribed(session) || _disposed) return;
    try {
      await _ensureHistory(session);
    } catch (error) {
      if (!_disposed) {
        debugPrint(
          '[ThreadRepliesNotifier] history resync failed for '
          '${args.rootId}: $error',
        );
      }
    }
  }

  Future<void> _ensureHistory(RelaySessionNotifier session) async {
    final inFlight = _historyFuture;
    if (inFlight != null) return inFlight;

    final fetch = _fetchHistory(session);
    _historyFuture = fetch;
    try {
      await fetch;
    } finally {
      if (identical(_historyFuture, fetch)) {
        _historyFuture = null;
      }
    }
  }

  Future<void> _fetchHistory(RelaySessionNotifier session) async {
    _ThreadCursor? cursor;
    for (var page = 0; page < 500; page++) {
      final events = await session.queryRelay([
        _threadRepliesFilter(args, cursor),
      ]);
      if (_disposed) return;

      for (final event in events) {
        _repliesById[event.id] = event;
      }
      state = AsyncData(_sortedReplies());

      if (events.length < 200) return;
      final last = events.last;
      cursor = _ThreadCursor(createdAt: last.createdAt, eventId: last.id);
    }
    throw Exception('Thread ${args.rootId} exceeded the page safety limit.');
  }

  void _handleReply(NostrEvent event) {
    if (_disposed || event.channelId != args.channelId) return;
    if (!event.tags.any(
      (tag) => tag.length >= 2 && tag[0] == 'e' && tag[1] == args.rootId,
    )) {
      return;
    }
    if (_repliesById.containsKey(event.id)) return;

    _repliesById[event.id] = event;
    state = AsyncData(_sortedReplies());
  }

  List<NostrEvent> _sortedReplies() =>
      _mergeReplies(const <NostrEvent>[], _repliesById.values);
}

final threadRepliesProvider = AsyncNotifierProvider.autoDispose
    .family<ThreadRepliesNotifier, List<NostrEvent>, ThreadRepliesArgs>(
      ThreadRepliesNotifier.new,
    );

NostrFilter _threadRepliesFilter(
  ThreadRepliesArgs args,
  _ThreadCursor? cursor,
) {
  return NostrFilter(
    kinds: EventKind.channelTimelineContentKinds,
    tags: {
      '#e': [args.rootId],
      '#h': [args.channelId],
    },
    limit: 200,
    extensions: {
      'depth_limit': 64,
      if (cursor != null) 'thread_cursor': cursor.createdAt,
      if (cursor != null) 'thread_cursor_id': cursor.eventId,
    },
  );
}

NostrFilter _threadLiveRepliesFilter(ThreadRepliesArgs args) {
  final now = DateTime.now().toUtc().millisecondsSinceEpoch ~/ 1000;
  return NostrFilter(
    kinds: EventKind.channelTimelineContentKinds,
    tags: {
      '#e': [args.rootId],
      '#h': [args.channelId],
    },
    since: now - ThreadRepliesNotifier._replaySkewSeconds,
    limit: 200,
  );
}

class ThreadLocalRepliesNotifier extends Notifier<List<NostrEvent>> {
  final ThreadRepliesArgs args;

  ThreadLocalRepliesNotifier(this.args);

  @override
  List<NostrEvent> build() => const [];

  void add(NostrEvent event) {
    state = _mergeReplies(state, [event]);
  }

  void remove(String eventId) {
    state = state.where((event) => event.id != eventId).toList();
  }

  void confirm(Set<String> eventIds) {
    if (!state.any((event) => eventIds.contains(event.id))) return;
    state = state.where((event) => !eventIds.contains(event.id)).toList();
  }
}

final threadLocalRepliesProvider =
    NotifierProvider.family<
      ThreadLocalRepliesNotifier,
      List<NostrEvent>,
      ThreadRepliesArgs
    >(ThreadLocalRepliesNotifier.new);

/// Relay-backed replies merged with signed local replies that are still
/// waiting for acknowledgement.
final threadRepliesWithLocalProvider = Provider.autoDispose
    .family<AsyncValue<List<NostrEvent>>, ThreadRepliesArgs>((ref, args) {
      final relayReplies = ref.watch(threadRepliesProvider(args));
      final authoritativeIds = ref
          .watch(threadRepliesProvider(args).notifier)
          .authoritativeEventIds;
      final localReplies = ref.watch(threadLocalRepliesProvider(args));
      if (authoritativeIds.isNotEmpty && localReplies.isNotEmpty) {
        if (localReplies.any((event) => authoritativeIds.contains(event.id))) {
          Future.microtask(() {
            if (!ref.mounted) return;
            ref
                .read(threadLocalRepliesProvider(args).notifier)
                .confirm(authoritativeIds);
            ref
                .read(pendingLocalMessagesProvider(args.channelId).notifier)
                .confirm(authoritativeIds);
          });
        }
      }
      return relayReplies.when(
        data: (events) => AsyncData(_mergeReplies(events, localReplies)),
        loading: () {
          return localReplies.isEmpty
              ? const AsyncLoading()
              : AsyncData(localReplies);
        },
        error: (error, stackTrace) {
          return localReplies.isEmpty
              ? AsyncError(error, stackTrace)
              : AsyncData(localReplies);
        },
      );
    });

/// Union two event lists by id, newest-wins, in timeline order.
///
/// The thread view needs this to fold the channel's live socket events into its
/// own one-shot query result: the query asks for content kinds only, so
/// reactions, edits, and deletions that land while a thread is open never reach
/// it on their own.
List<NostrEvent> mergeThreadEvents(
  Iterable<NostrEvent> first,
  Iterable<NostrEvent> second,
) => _mergeReplies(first, second);

List<NostrEvent> _mergeReplies(
  Iterable<NostrEvent> first,
  Iterable<NostrEvent> second,
) {
  final byId = <String, NostrEvent>{};
  for (final event in [...first, ...second]) {
    byId[event.id] = event;
  }
  return byId.values.toList()..sort((a, b) {
    final createdAt = a.createdAt.compareTo(b.createdAt);
    return createdAt != 0 ? createdAt : a.id.compareTo(b.id);
  });
}
