import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel_window.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  group('parseChannelWindowResponse', () {
    test('requires exactly one matching bounds event', () {
      expect(
        () => parseChannelWindowResponse([_row('a')], _channelId, null),
        throwsException,
      );
      expect(
        () => parseChannelWindowResponse(
          [_row('a'), _bounds(), _bounds(id: 'b2')],
          _channelId,
          null,
        ),
        throwsException,
      );
      expect(
        () => parseChannelWindowResponse(
          [_row('a'), _bounds(dTag: 'wrong:head')],
          _channelId,
          null,
        ),
        throwsException,
      );
    });

    test('rejects hasMore and nextCursor disagreement', () {
      expect(
        () => parseChannelWindowResponse(
          [
            _row('a'),
            _bounds(content: {'has_more': true, 'next_cursor': null}),
          ],
          _channelId,
          null,
        ),
        throwsException,
      );
    });

    test('attaches summaries and keeps overlays out of rows', () {
      final page = parseChannelWindowResponse(
        [
          _row('root'),
          _summary('root', replyCount: 3, participants: ['p1', 'p2']),
          _reaction('reaction'),
          _bounds(),
        ],
        _channelId,
        null,
      );

      expect(page.rows.map((row) => row.event.id), ['root']);
      expect(page.rows.single.thread?.replyCount, 3);
      expect(page.rows.single.thread?.participantPubkeys, ['p1', 'p2']);
      expect(page.aux.map((event) => event.id), ['reaction']);
    });
  });

  group('ChannelWindowStore', () {
    test('rejects cursor interval and row order violations', () {
      expect(
        () => appendOlderChannelWindow(
          replaceNewestChannelWindow(
            const ChannelWindowStore.empty(),
            _page(rows: [_row('head', createdAt: 20)], hasMore: true),
          ),
          _page(
            startCursor: const ChannelPageCursor(
              createdAt: 20,
              eventId: 'head',
            ),
            rows: [_row('newer', createdAt: 21)],
          ),
        ),
        throwsException,
      );
      expect(
        () => replaceNewestChannelWindow(
          const ChannelWindowStore.empty(),
          _page(rows: [_row('b', createdAt: 9), _row('a', createdAt: 10)]),
        ),
        throwsException,
      );
    });

    test('rejects overlapping older pages and drops tail on head refresh', () {
      final head = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('a', createdAt: 10)], hasMore: true),
      );
      final withOlder = appendOlderChannelWindow(
        head,
        _page(
          startCursor: const ChannelPageCursor(createdAt: 10, eventId: 'a'),
          rows: [_row('z', createdAt: 9)],
        ),
      );
      expect(withOlder.pages, hasLength(2));
      expect(
        () => appendOlderChannelWindow(
          head,
          _page(
            startCursor: const ChannelPageCursor(createdAt: 10, eventId: 'a'),
            rows: [_row('a', createdAt: 9)],
          ),
        ),
        throwsException,
      );

      final refreshed = replaceNewestChannelWindow(
        withOlder,
        _page(rows: [_row('b', createdAt: 11)]),
      );
      expect(refreshed.pages, hasLength(1));
      expect(refreshed.pages.single.rows.single.event.id, 'b');
    });

    test('drops live rows at or older than oldest loaded boundary', () {
      final store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('a', createdAt: 10), _row('m', createdAt: 9)]),
      );
      final ignored = mergeLiveChannelWindowEvent(
        store,
        _row('z', createdAt: 8),
        isTimelineRow: true,
      );
      expect(identical(ignored, store), isTrue);

      final merged = mergeLiveChannelWindowEvent(
        store,
        _row('live', createdAt: 11),
        isTimelineRow: true,
      );
      expect(merged.liveOverlay.map((event) => event.id), ['live']);
    });
  });
}

const _channelId = 'ABCDEF';

ChannelWindowPage _page({
  ChannelPageCursor? startCursor,
  required List<NostrEvent> rows,
  bool hasMore = false,
}) {
  return ChannelWindowPage(
    startCursor: startCursor,
    rows: [for (final row in rows) ChannelWindowRow(event: row)],
    aux: const [],
    nextCursor: hasMore
        ? ChannelPageCursor(
            createdAt: rows.last.createdAt,
            eventId: rows.last.id,
          )
        : null,
    hasMore: hasMore,
  );
}

NostrEvent _row(String id, {int createdAt = 10}) => _event(
  id: id,
  createdAt: createdAt,
  kind: EventKind.streamMessageV2,
  tags: const [
    ['h', _channelId],
  ],
);

NostrEvent _reaction(String id) => _event(
  id: id,
  kind: EventKind.reaction,
  tags: const [
    ['e', 'root'],
  ],
);

NostrEvent _summary(
  String rootId, {
  required int replyCount,
  required List<String> participants,
}) => _event(
  id: 'summary-$rootId',
  kind: EventKind.channelThreadSummary,
  tags: [
    ['e', rootId],
  ],
  content: jsonEncode({
    'reply_count': replyCount,
    'descendant_count': replyCount,
    'last_reply_at': 12,
    'participants': participants,
  }),
);

NostrEvent _bounds({
  String id = 'bounds',
  String? dTag,
  Map<String, dynamic>? content,
}) => _event(
  id: id,
  kind: EventKind.channelWindowBounds,
  tags: [
    ['d', dTag ?? '${_channelId.toLowerCase()}:head'],
  ],
  content: jsonEncode(content ?? {'has_more': false, 'next_cursor': null}),
);

NostrEvent _event({
  required String id,
  int createdAt = 10,
  required int kind,
  List<List<String>> tags = const [],
  String content = '',
}) {
  return NostrEvent(
    id: id,
    pubkey: 'alice',
    createdAt: createdAt,
    kind: kind,
    tags: tags,
    content: content,
    sig: 'sig',
  );
}
