import 'dart:async';

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

final threadRepliesProvider =
    FutureProvider.family<List<NostrEvent>, ThreadRepliesArgs>((
      ref,
      args,
    ) async {
      final session = ref.watch(relaySessionProvider.notifier);
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
final threadRepliesWithLocalProvider =
    Provider.family<AsyncValue<List<NostrEvent>>, ThreadRepliesArgs>((
      ref,
      args,
    ) {
      final relayReplies = ref.watch(threadRepliesProvider(args));
      final localReplies = ref.watch(threadLocalRepliesProvider(args));
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
      if (localReplies.isEmpty) return relayReplies;
      return relayReplies.when(
        data: (events) => AsyncData(_mergeReplies(events, localReplies)),
        loading: () => AsyncData(localReplies),
        error: (error, stackTrace) => AsyncData(localReplies),
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
