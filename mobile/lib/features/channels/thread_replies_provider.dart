import 'dart:async';

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

final threadRepliesProvider = FutureProvider.autoDispose
    .family<List<NostrEvent>, ThreadRepliesArgs>((ref, args) async {
      final session = ref.watch(relaySessionProvider.notifier);
      // Establish the live subscription BEFORE snapshotting history. Otherwise
      // an event published after `/query` returns but before the REQ registers
      // is in neither result and is lost permanently — the same class of bug
      // this provider exists to fix. `ChannelMessagesNotifier._init()` orders
      // it the same way.
      await ref.read(threadLiveRepliesProvider(args).notifier).subscribed;
      final replies = <NostrEvent>[];
      _ThreadCursor? cursor;
      for (var page = 0; page < 500; page++) {
        final events = await session.queryRelay([
          _threadRepliesFilter(args, cursor),
        ]);
        replies.addAll(events);
        if (events.length < 200) return replies;
        final last = events.last;
        cursor = _ThreadCursor(createdAt: last.createdAt, eventId: last.id);
      }
      throw Exception('Thread ${args.rootId} exceeded the page safety limit.');
    });

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

/// Live relay replies for an open thread.
///
/// [threadRepliesProvider] is a one-shot paginated query: it resolves once when
/// the thread opens and caches. Replies that arrive afterwards — including the
/// viewer's own, and agent replies — never entered that list, so an open thread
/// looked frozen while the activity feed (which subscribes) updated instantly.
///
/// This subscribes to the same filter with `limit: 0` (live only, no history —
/// history is the paginated fetch's job) and accumulates arrivals. Merging
/// happens in [threadRepliesWithLocalProvider] via [_mergeReplies], which
/// de-duplicates by event id, so an event delivered both by the fetch and the
/// subscription appears once.
///
/// Mirrors `ChannelsNotifier._subscribeLive`.
class ThreadLiveRepliesNotifier extends Notifier<List<NostrEvent>> {
  final ThreadRepliesArgs args;

  ThreadLiveRepliesNotifier(this.args);

  void Function()? _unsubscribe;
  bool _disposed = false;
  int _subscribeGeneration = 0;

  /// Completes once the live REQ is registered, or has definitively failed.
  /// [threadRepliesProvider] awaits this before issuing its history query, so
  /// an event published between the history snapshot and the subscription
  /// being registered cannot fall through the gap and be lost permanently.
  /// `ChannelMessagesNotifier._init()` establishes the same ordering.
  final Completer<void> _subscribed = Completer<void>();
  Future<void> get subscribed => _subscribed.future;

  void _markSubscribed() {
    if (!_subscribed.isCompleted) _subscribed.complete();
  }

  @override
  List<NostrEvent> build() {
    _disposed = false;

    ref.onDispose(() {
      _disposed = true;
      _clearSubscription();
    });

    // Subscribe when connected. If already connected, initiate immediately.
    // Listen for reconnection or late connection if not already subscribed.
    ref.listen<SessionStatus>(
      relaySessionProvider.select((session) => session.status),
      (previous, next) {
        if (next == SessionStatus.connected && _unsubscribe == null) {
          unawaited(_subscribe());
        }
      },
    );

    if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
      unawaited(_subscribe());
    }

    return const [];
  }

  Future<void> _subscribe() async {
    if (_disposed || _unsubscribe != null) return;
    final generation = ++_subscribeGeneration;
    final session = ref.read(relaySessionProvider.notifier);
    try {
      final unsubscribe = await session.subscribe(
        _threadLiveRepliesFilter(args),
        _handleLiveEvent,
        onClosed: (message) => _handleClosed(generation, message),
      );
      // Dispose can land while this await is suspended. On a fast thread
      // open/close, onDispose runs while `_unsubscribe` is still null.
      // Also check if a new subscription attempt began or if session disconnected.
      if (_disposed ||
          generation != _subscribeGeneration ||
          ref.read(relaySessionProvider).status != SessionStatus.connected) {
        unsubscribe();
        _markSubscribed();
        return;
      }
      _unsubscribe = unsubscribe;
    } catch (error) {
      if (generation == _subscribeGeneration) {
        debugPrint(
          '[ThreadLiveReplies] live subscription failed for ${args.rootId}: $error',
        );
      }
    } finally {
      // Always release the history fetch, success or failure. A thread that
      // could not subscribe must still load its backlog rather than hang.
      _markSubscribed();
    }
  }

  /// The relay sent CLOSED for this subscription. RelaySessionNotifier drops it
  /// from `_liveSubscriptions`, so it will NOT be replayed on reconnect — but
  /// this notifier would still hold a non-null `_unsubscribe` and report
  /// healthy while silently receiving nothing again, which is the exact
  /// failure this provider exists to fix. Clear local state so a later
  /// reconnect can re-subscribe. The generation check stops a stale callback
  /// from tearing down a newer subscription.
  void _handleClosed(int generation, String message) {
    if (_disposed || generation != _subscribeGeneration) return;
    debugPrint(
      '[ThreadLiveReplies] subscription CLOSED for ${args.rootId}: $message',
    );
    _unsubscribe = null;
    if (ref.read(relaySessionProvider).status == SessionStatus.connected) {
      unawaited(_subscribe());
    }
  }

  void _clearSubscription() {
    _subscribeGeneration++;
    _unsubscribe?.call();
    _unsubscribe = null;
  }

  void _handleLiveEvent(NostrEvent event) {
    if (_disposed) return;
    state = _mergeReplies(state, [event]);
  }
}

/// AUTO-DISPOSE IS LOAD-BEARING. As a keep-alive family this leaks one live
/// relay subscription per thread opened for the lifetime of the app session —
/// `ref.onDispose` would only run when the container itself is torn down, so
/// the `_disposed` guard below would never fire in the case that matters. The
/// relay caps subscriptions (see buzz-relay handlers/req.rs), so the leak is
/// bounded only by eventual rejection.
///
/// The consumer [threadRepliesWithLocalProvider] must be auto-dispose too: a
/// keep-alive consumer watching an auto-dispose provider keeps it alive anyway.
final threadLiveRepliesProvider = NotifierProvider.autoDispose
    .family<ThreadLiveRepliesNotifier, List<NostrEvent>, ThreadRepliesArgs>(
      ThreadLiveRepliesNotifier.new,
    );

/// Live-only variant of [_threadRepliesFilter]: `limit: 0` asks the relay for
/// new events and no backlog. History is the paginated fetch's responsibility.
NostrFilter _threadLiveRepliesFilter(ThreadRepliesArgs args) {
  return NostrFilter(
    kinds: EventKind.channelTimelineContentKinds,
    tags: {
      '#e': [args.rootId],
      '#h': [args.channelId],
    },
    limit: 0,
    extensions: {'depth_limit': 64},
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
/// Auto-dispose: a keep-alive consumer would hold the auto-dispose providers
/// it watches alive indefinitely, defeating the subscription cleanup entirely.
final threadRepliesWithLocalProvider = Provider.autoDispose
    .family<AsyncValue<List<NostrEvent>>, ThreadRepliesArgs>((ref, args) {
      final relayReplies = ref.watch(threadRepliesProvider(args));
      final liveReplies = ref.watch(threadLiveRepliesProvider(args));
      final localReplies = ref.watch(threadLocalRepliesProvider(args));
      // Ownership is released ONLY by the authoritative paginated fetch — a
      // live echo does not count, even though it is displayed immediately.
      //
      // The live subscription can deliver an event the relay will not return
      // on a later query (rejected downstream, dropped on a failover, CLOSED
      // mid-flight). Confirming on the echo hands the row back to the relay,
      // and if the refetch then fails there is nothing left holding the
      // message on screen — the viewer's own reply disappears. Holding
      // ownership until a query confirms costs one redundant entry that
      // `_mergeReplies` de-duplicates by id anyway.
      final authoritative = relayReplies.value;
      if (authoritative != null && localReplies.isNotEmpty) {
        final authoritativeIds = authoritative.map((event) => event.id).toSet();
        if (localReplies.any((event) => authoritativeIds.contains(event.id))) {
          Future.microtask(() {
            ref
                .read(threadLocalRepliesProvider(args).notifier)
                .confirm(authoritativeIds);
            ref
                .read(pendingLocalMessagesProvider(args.channelId).notifier)
                .confirm(authoritativeIds);
          });
        }
      }
      if (localReplies.isEmpty && liveReplies.isEmpty) return relayReplies;
      return relayReplies.when(
        data: (events) => AsyncData(
          _mergeReplies(_mergeReplies(events, liveReplies), localReplies),
        ),
        loading: () => AsyncData(_mergeReplies(liveReplies, localReplies)),
        error: (error, stackTrace) =>
            AsyncData(_mergeReplies(liveReplies, localReplies)),
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
