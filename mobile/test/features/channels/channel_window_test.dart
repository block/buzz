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

  group('live thread summaries', () {
    test('a live summary gives a loaded row its reply count', () {
      final store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('root', createdAt: 10)]),
      );
      // No summary yet: the row was fetched before anyone replied.
      expect(channelWindowThreadSummaries(store), isEmpty);

      final merged = mergeLiveChannelWindowEvent(
        store,
        _summary('root', replyCount: 1, participants: ['p1']),
        isTimelineRow: false,
      );

      expect(channelWindowThreadSummaries(merged)['root']?.replyCount, 1);
    });

    test('a live summary covers a root that is only in the overlay', () {
      // A message you just sent has no page row yet, so its count has nowhere
      // to live except beside the pages.
      final store = mergeLiveChannelWindowEvent(
        replaceNewestChannelWindow(
          const ChannelWindowStore.empty(),
          _page(rows: [_row('old', createdAt: 10)]),
        ),
        _row('mine', createdAt: 11),
        isTimelineRow: true,
      );

      final merged = mergeLiveChannelWindowEvent(
        store,
        _summary('mine', replyCount: 2, participants: ['p1', 'p2']),
        isTimelineRow: false,
      );

      expect(channelWindowThreadSummaries(merged)['mine']?.replyCount, 2);
    });

    test('a later live summary replaces an earlier one', () {
      var store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('root', createdAt: 10)]),
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _summary('root', replyCount: 1, participants: ['p1'], createdAt: 11),
        isTimelineRow: false,
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _summary(
          'root',
          replyCount: 2,
          participants: ['p1', 'p2'],
          createdAt: 12,
        ),
        isTimelineRow: false,
      );

      expect(channelWindowThreadSummaries(store)['root']?.replyCount, 2);
    });

    test(
      'an older live summary is ignored but a later same-second summary wins',
      () {
        var store = replaceNewestChannelWindow(
          const ChannelWindowStore.empty(),
          _page(rows: [_row('root', createdAt: 10)]),
        );
        store = mergeLiveChannelWindowEvent(
          store,
          _summary('root', replyCount: 3, participants: ['p1'], createdAt: 30),
          isTimelineRow: false,
        );
        final afterOlder = mergeLiveChannelWindowEvent(
          store,
          _summary('root', replyCount: 1, participants: ['p1'], createdAt: 29),
          isTimelineRow: false,
        );
        final afterSameSecond = mergeLiveChannelWindowEvent(
          afterOlder,
          _summary('root', replyCount: 2, participants: ['p1'], createdAt: 30),
          isTimelineRow: false,
        );

        expect(identical(afterOlder, store), isTrue);
        expect(
          channelWindowThreadSummaries(afterSameSecond)['root']?.replyCount,
          2,
        );
      },
    );

    test(
      'a summary received during the initial query wins the page snapshot',
      () {
        var store = mergeLiveChannelWindowEvent(
          const ChannelWindowStore.empty(),
          _summary('root', replyCount: 2, participants: ['p1'], createdAt: 30),
          isTimelineRow: false,
        );

        store = replaceNewestChannelWindow(
          store,
          ChannelWindowPage(
            startCursor: null,
            rows: [
              ChannelWindowRow(
                event: _row('root', createdAt: 10),
                thread: const ChannelWindowThreadSummary(
                  replyCount: 1,
                  descendantCount: 1,
                  lastReplyAt: 12,
                  participantPubkeys: ['p1'],
                ),
                // The page overlay can be signed after its query snapshot.
                threadSummaryCreatedAt: 31,
              ),
            ],
            aux: const [],
            nextCursor: null,
            hasMore: false,
          ),
          retainLiveSummaryRootIds: const {'root'},
        );

        expect(channelWindowThreadSummaries(store)['root']?.replyCount, 2);
      },
    );

    test('a replayed summary yields to a newer initial page snapshot', () {
      var store = mergeLiveChannelWindowEvent(
        const ChannelWindowStore.empty(),
        _summary('root', replyCount: 2, participants: ['p1'], createdAt: 30),
        isTimelineRow: false,
      );

      store = replaceNewestChannelWindow(
        store,
        ChannelWindowPage(
          startCursor: null,
          rows: [
            ChannelWindowRow(
              event: _row('root', createdAt: 10),
              thread: const ChannelWindowThreadSummary(
                replyCount: 1,
                descendantCount: 1,
                lastReplyAt: 12,
                participantPubkeys: ['p1'],
              ),
              threadSummaryCreatedAt: 31,
            ),
          ],
          aux: const [],
          nextCursor: null,
          hasMore: false,
        ),
      );

      expect(channelWindowThreadSummaries(store)['root']?.replyCount, 1);
    });

    test('a newer live summary survives an appended older page', () {
      var store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('head', createdAt: 10)], hasMore: true),
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _summary('older', replyCount: 2, participants: ['p1'], createdAt: 30),
        isTimelineRow: false,
      );

      store = appendOlderChannelWindow(
        store,
        ChannelWindowPage(
          startCursor: const ChannelPageCursor(createdAt: 10, eventId: 'head'),
          rows: [
            ChannelWindowRow(
              event: _row('older', createdAt: 9),
              thread: const ChannelWindowThreadSummary(
                replyCount: 1,
                descendantCount: 1,
                lastReplyAt: 12,
                participantPubkeys: ['p1'],
              ),
              threadSummaryCreatedAt: 29,
            ),
          ],
          aux: const [],
          nextCursor: null,
          hasMore: false,
        ),
      );

      expect(channelWindowThreadSummaries(store)['older']?.replyCount, 2);
    });

    test('a refetched row outranks the live entry it duplicates', () {
      var store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('root', createdAt: 10)]),
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _summary('root', replyCount: 1, participants: ['p1']),
        isTimelineRow: false,
      );

      // Refetching the head brings down the authoritative count with the row,
      // so the stale live entry must not shadow it.
      final refreshed = replaceNewestChannelWindow(
        store,
        ChannelWindowPage(
          startCursor: null,
          rows: [
            ChannelWindowRow(
              event: _row('root', createdAt: 10),
              thread: const ChannelWindowThreadSummary(
                replyCount: 5,
                descendantCount: 5,
                lastReplyAt: 12,
                participantPubkeys: ['p1'],
              ),
              threadSummaryCreatedAt: 20,
            ),
          ],
          aux: const [],
          nextCursor: null,
          hasMore: false,
        ),
      );

      expect(channelWindowThreadSummaries(refreshed)['root']?.replyCount, 5);
    });

    test('a summary survives unrelated live events', () {
      var store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('root', createdAt: 10)]),
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _summary('root', replyCount: 1, participants: ['p1']),
        isTimelineRow: false,
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _reaction('reaction'),
        isTimelineRow: false,
      );
      store = mergeLiveChannelWindowEvent(
        store,
        _row('later', createdAt: 11),
        isTimelineRow: true,
      );

      expect(channelWindowThreadSummaries(store)['root']?.replyCount, 1);
    });

    test('a summary with no root tag or bad content is ignored', () {
      final store = replaceNewestChannelWindow(
        const ChannelWindowStore.empty(),
        _page(rows: [_row('root', createdAt: 10)]),
      );

      final untagged = mergeLiveChannelWindowEvent(
        store,
        _event(id: 'no-e', kind: EventKind.channelThreadSummary, content: '{}'),
        isTimelineRow: false,
      );
      expect(identical(untagged, store), isTrue);

      final malformed = mergeLiveChannelWindowEvent(
        store,
        _event(
          id: 'bad',
          kind: EventKind.channelThreadSummary,
          tags: const [
            ['e', 'root'],
          ],
          content: 'not json',
        ),
        isTimelineRow: false,
      );
      expect(channelWindowThreadSummaries(malformed), isEmpty);
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
  int createdAt = 10,
}) => _event(
  id: 'summary-$rootId-$createdAt-$replyCount',
  createdAt: createdAt,
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
