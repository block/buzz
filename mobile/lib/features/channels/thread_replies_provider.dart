import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

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
