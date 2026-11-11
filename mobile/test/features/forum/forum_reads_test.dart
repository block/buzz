import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/forum/forum_models.dart';
import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _channel = '11111111-1111-4111-8111-111111111111';
const _relay = 'https://relay.example';
const _secret =
    '0000000000000000000000000000000000000000000000000000000000000001';
final _author = derivePublicKeyHex(_secret);

NostrEvent _event(
  int kind,
  int time, {
  String? parent,
  String? root,
  String channel = _channel,
  String content = '',
  List<List<String>> tags = const [],
}) => NostrEvent.fromJson(
  jsonDecode(
        signNostrEvent(
          kind: kind,
          createdAt: time,
          secretKey: _secret,
          content: content,
          tags: [
            ['h', channel],
            if (parent != null) ['e', parent, '', 'reply'],
            if (root != null) ['e', root, '', 'root'],
            ...tags,
          ],
        ).toJson(),
      )
      as Map<String, dynamic>,
);

Map<String, Object> _cursor(NostrEvent event) => {
  'created_at': event.createdAt,
  'id': event.id,
};

List<NostrEvent> _window(
  NostrFilter filter,
  List<NostrEvent> rows, {
  NostrEvent? next,
  List<NostrEvent> aux = const [],
  bool thread = false,
  bool corruptSignature = false,
  String? boundChannel,
}) {
  final String key;
  if (thread) {
    // Independent serialization of the NIP-CW binding, rather than using the
    // production request object's key helper to generate its own test oracle.
    final binding = jsonEncode([
      'tw',
      1,
      'older',
      'relay.example',
      _author,
      _channel,
      filter.tags['#e']!.single,
      50,
      100,
      [9, 40002, 45003],
      filter.until == null
          ? null
          : [filter.until, filter.extensions['before_id']],
      true,
    ]);
    key = 'tw:1:${sha256.convert(utf8.encode(binding))}';
  } else {
    key =
        '$_channel:${filter.until == null ? 'head' : '${filter.until}:${filter.extensions['before_id']}'}';
  }
  final bound = NostrEvent.fromJson(
    jsonDecode(
          signNostrEvent(
            kind: thread ? 39007 : 39006,
            createdAt: 9000,
            secretKey: _secret,
            tags: [
              ['d', key],
              ['h', boundChannel ?? _channel],
              if (thread) ['e', filter.tags['#e']!.single],
            ],
            content: jsonEncode({
              if (thread) 'version': 1,
              if (thread) 'direction': 'older',
              'has_more': next != null,
              'next_cursor': next == null ? null : _cursor(next),
            }),
          ).toJson(),
        )
        as Map<String, dynamic>,
  );
  return [
    ...rows,
    ...aux,
    if (!corruptSignature)
      bound
    else
      NostrEvent(
        id: bound.id,
        pubkey: bound.pubkey,
        createdAt: bound.createdAt,
        kind: bound.kind,
        tags: bound.tags,
        content: bound.content,
        sig: '0' * 128,
      ),
  ];
}

class _Session extends RelaySessionNotifier {
  final FutureOr<List<NostrEvent>> Function(NostrFilter) answer;
  final filters = <NostrFilter>[];
  _Session(this.answer);
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    this.filters.addAll(filters);
    return answer(filters.single);
  }
}

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: _relay);
}

ProviderContainer _container(_Session session) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    relayConfigProvider.overrideWith(_Config.new),
    relaySessionProvider.overrideWith(() => session),
    myPubkeyProvider.overrideWithValue(_author),
    relayAuthorProvider(_relay).overrideWith((ref) async => _author),
  ],
);

void main() {
  test(
    'tied reply timestamps retain every ID across a thread page boundary',
    () async {
      final root = _event(9, 10);
      final replies = [
        _event(9, 100, parent: root.id, content: 'a'),
        _event(40002, 100, parent: root.id, content: 'b'),
        _event(45003, 100, parent: root.id, content: 'c'),
      ]..sort((a, b) => a.id.compareTo(b.id));
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        return filter.until == null
            ? _window(
                filter,
                replies.take(2).toList(),
                next: replies[1],
                thread: true,
              )
            : _window(filter, [replies[2]], thread: true);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      await container.read(provider.future);
      await container.read(provider.notifier).loadMore();
      expect(session.filters.last.until, 100);
      expect(session.filters.last.extensions['before_id'], replies[1].id);
      final response = container.read(provider).requireValue;
      expect(
        response.replies.map((reply) => reply.eventId),
        replies.map((event) => event.id),
      );
      expect(response.isIncomplete, isFalse);
    },
  );

  test(
    'a claimed root cannot override the resolved parent path to a different root',
    () async {
      final root = _event(9, 10);
      final otherRoot = _event(45001, 11);
      final parent = _event(40002, 20, parent: otherRoot.id);
      final claimed = _event(9, 30, parent: parent.id, root: root.id);
      final session = _Session((filter) {
        if (filter.ids != null) {
          return [
            root,
            otherRoot,
            parent,
          ].where((event) => filter.ids!.contains(event.id)).toList();
        }
        return _window(filter, [claimed], thread: true);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final response = await container.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      );
      expect(response.replies, isEmpty);
      expect(response.unresolvedReplies, 1);
      expect(response.isIncomplete, isTrue);
    },
  );

  test(
    'a retired continuation cannot overwrite a replacement forum head',
    () async {
      final head = _event(9, 100);
      final older = _event(40002, 90);
      final replacement = _event(45001, 200);
      final pending = Completer<List<NostrEvent>>();
      final started = Completer<NostrFilter>();
      var heads = 0;
      final session = _Session((filter) {
        if (filter.until != null) {
          started.complete(filter);
          return pending.future;
        }
        return ++heads == 1
            ? _window(filter, [head], next: head)
            : _window(filter, [replacement]);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumPostsProvider(_channel);
      await container.read(provider.future);
      final loading = container.read(provider.notifier).loadMore();
      final continuation = await started.future;
      container.invalidate(provider);
      await container.read(provider.future);
      pending.complete(_window(continuation, [older]));
      await loading;
      expect(
        container.read(provider).requireValue.posts.single.eventId,
        replacement.id,
      );
      expect(container.read(provider).requireValue.hasMore, isFalse);
    },
  );

  test(
    'mixed roots include both writer families and references, excluding replies',
    () async {
      final root = _event(9, 100);
      final rows = [
        root,
        _event(40002, 90),
        _event(45001, 80),
        _event(
          9,
          70,
          tags: [
            ['q', root.id],
          ],
        ),
        _event(
          40002,
          60,
          tags: [
            ['e', root.id, '', 'mention'],
          ],
        ),
        _event(
          9,
          50,
          tags: [
            ['e', root.id],
          ],
        ),
        _event(
          9,
          40,
          tags: [
            ['e', root.id, '', 'root'],
          ],
        ),
        _event(
          9,
          30,
          parent: root.id,
          tags: const [
            ['broadcast', '1'],
          ],
        ),
      ];
      final session = _Session((filter) => _window(filter, rows));
      final container = _container(session);
      addTearDown(container.dispose);
      final response = await container.read(
        forumPostsProvider(_channel).future,
      );
      expect(
        response.posts.map((post) => post.eventId),
        rows.take(7).map((row) => row.id),
      );
      expect(
        response.posts.map((post) => post.kind),
        containsAll([9, 40002, 45001]),
      );
      expect(session.filters.single.extensions['top_level'], isTrue);
      expect(session.filters.single.kinds, [9, 40002, 45001]);
    },
  );

  test(
    'busy reply history cannot crowd roots out of a channel window',
    () async {
      final root = _event(9, 10);
      final other = _event(45001, 9);
      final busy = [
        for (var i = 0; i < 210; i++) _event(9, 1000 + i, parent: root.id),
      ];
      final session = _Session((filter) {
        final rows = filter.extensions['top_level'] == true
            ? [root, other]
            : busy.reversed.take(50).toList();
        return _window(filter, rows);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final response = await container.read(
        forumPostsProvider(_channel).future,
      );
      expect(response.posts.map((post) => post.eventId), [root.id, other.id]);
      expect(session.filters, hasLength(1));
    },
  );

  test(
    'root continuation echoes tied timestamp cursor, independent of visible rows',
    () async {
      final tied = [
        _event(9, 100, content: 'a'),
        _event(40002, 100, content: 'b'),
        _event(45001, 100, content: 'c'),
      ]..sort((a, b) => a.id.compareTo(b.id));
      final session = _Session(
        (filter) => filter.until == null
            ? _window(filter, tied.take(2).toList(), next: tied[1])
            : _window(filter, [tied[2]]),
      );
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumPostsProvider(_channel);
      final head = await container.read(provider.future);
      expect(head.hasMore, isTrue);
      await container.read(provider.notifier).loadMore();
      expect(session.filters.last.until, 100);
      expect(session.filters.last.extensions['before_id'], tied[1].id);
      expect(
        container.read(provider).requireValue.posts.map((post) => post.eventId),
        tied.map((event) => event.id),
      );
      expect(container.read(provider).requireValue.hasMore, isFalse);
    },
  );

  test(
    'root auxiliary edits and deletions use chat formatting, including deleted edits',
    () async {
      final a = _event(9, 100, content: 'original');
      final b = _event(45001, 90);
      final edit = _event(
        40003,
        200,
        content: 'edited',
        tags: [
          ['e', a.id],
        ],
      );
      final deletion = _event(
        5,
        210,
        tags: [
          ['e', b.id],
        ],
      );
      final session = _Session(
        (filter) => _window(filter, [a, b], aux: [edit, deletion]),
      );
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumPostsProvider(_channel);
      expect(
        (await container.read(provider.future)).posts.single.content,
        'edited',
      );
      final deleteEdit = _event(
        5,
        220,
        tags: [
          ['e', edit.id],
        ],
      );
      final nextSession = _Session(
        (filter) => _window(filter, [a], aux: [edit, deleteEdit]),
      );
      final other = _container(nextSession);
      addTearDown(other.dispose);
      expect(
        (await other.read(provider.future)).posts.single.content,
        'original',
      );
    },
  );

  test(
    'head-only refresh retains older pages and merges IDs without extra history queries',
    () async {
      final a = _event(9, 100);
      final b = _event(45001, 90);
      final c = _event(40002, 80);
      final newer = _event(9, 110);
      var heads = 0;
      final session = _Session(
        (filter) => filter.until != null
            ? _window(filter, [c])
            : ++heads == 1
            ? _window(filter, [a, b], next: b)
            : _window(filter, [newer, a], next: a),
      );
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumPostsProvider(_channel);
      await container.read(provider.future);
      final reader = container.read(provider.notifier);
      await reader.loadMore();
      final before = session.filters.length;
      await reader.refresh();
      final response = container.read(provider).requireValue;
      expect(session.filters.length - before, 1);
      expect(session.filters.last.until, isNull);
      expect(response.posts.map((post) => post.eventId), [
        newer.id,
        a.id,
        b.id,
        c.id,
      ]);
      expect(response.hasMore, isFalse);
      await reader.refresh();
      expect(container.read(provider).requireValue.posts, hasLength(4));
    },
  );

  test(
    'fallback reissues clean filters and scans past busy thread replies',
    () async {
      final root = _event(9, 1);
      final replies = [
        for (var i = 0; i < 205; i++) _event(9, i + 10, parent: root.id),
      ]..sort((a, b) => b.createdAt.compareTo(a.createdAt));
      final session = _Session((filter) {
        if (filter.extensions['top_level'] == true) return [];
        expect(filter.extensions, isEmpty);
        if (filter.tags.containsKey('#e')) return [];
        return [...replies, root]
            .where(
              (event) =>
                  filter.until == null || event.createdAt <= filter.until!,
            )
            .take(filter.limit)
            .toList();
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final response = await container.read(
        forumPostsProvider(_channel).future,
      );
      expect(response.posts.single.eventId, root.id);
      expect(response.hasMore, isFalse);
      expect(
        session.filters.where(
          (filter) =>
              filter.extensions.isEmpty && !filter.tags.containsKey('#e'),
        ),
        hasLength(2),
      );
    },
  );

  for (final unsupported in [400, 422]) {
    test(
      'unsupported window $unsupported falls back, while timeout does not',
      () async {
        final root = _event(45001, 10);
        final session = _Session((filter) {
          if (filter.extensions.isNotEmpty) {
            throw RelayException(unsupported, 'unsupported filter');
          }
          return filter.tags.containsKey('#e') ? [] : [root];
        });
        final container = _container(session);
        addTearDown(container.dispose);
        expect(
          (await container.read(
            forumPostsProvider(_channel).future,
          )).posts.single.eventId,
          root.id,
        );
      },
    );
  }

  test('window deadline propagates without compatibility fallback', () async {
    final session = _Session(
      (_) => throw RelayException(503, '{"error":"query timed out"}'),
    );
    final container = _container(session);
    addTearDown(container.dispose);
    await expectLater(
      container.read(forumPostsProvider(_channel).future),
      throwsA(isA<RelayException>()),
    );
    expect(session.filters, hasLength(1));
  });

  test(
    'mixed replies resolve through out-of-page parent; root claims and references remain incomplete',
    () async {
      final root = _event(45001, 10);
      final parent = _event(9, 20, parent: root.id);
      final direct = _event(40002, 30, parent: root.id);
      final nested = _event(45003, 40, parent: parent.id);
      final unresolved = _event(9, 50, parent: 'f' * 64, root: root.id);
      final quote = _event(
        9,
        60,
        tags: [
          ['q', root.id],
        ],
      );
      final mention = _event(
        40002,
        70,
        tags: [
          ['e', root.id, '', 'mention'],
        ],
      );
      final rows = [mention, quote, unresolved, nested, direct];
      final session = _Session((filter) {
        if (filter.ids != null) {
          return [
            root,
            parent,
          ].where((event) => filter.ids!.contains(event.id)).toList();
        }
        return _window(filter, rows, thread: true);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final response = await container.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      );
      expect(response.replies.map((reply) => reply.eventId), [
        direct.id,
        nested.id,
      ]);
      expect(response.replies.last.parentEventId, parent.id);
      expect(response.replies.last.rootEventId, root.id);
      expect(response.unresolvedReplies, 3);
      expect(response.isIncomplete, isTrue);
      expect(response.hasMore, isFalse);
      expect(
        session.filters.any(
          (filter) => filter.ids?.contains(parent.id) == true,
        ),
        isTrue,
      );
    },
  );

  test(
    'root fetched for the wrong channel is rejected before requesting replies',
    () async {
      final root = _event(
        9,
        10,
        channel: '22222222-2222-4222-8222-222222222222',
      );
      final session = _Session((_) => [root]);
      final container = _container(session);
      addTearDown(container.dispose);
      await expectLater(
        container.read(
          forumThreadProvider((channelId: _channel, eventId: root.id)).future,
        ),
        throwsA(isA<StateError>()),
      );
      expect(session.filters.single.tags['#h'], [_channel]);
    },
  );

  test(
    'thread edits/deletions apply to root and replies while preserving ancestry',
    () async {
      final root = _event(9, 10, content: 'post');
      final a = _event(40002, 20, parent: root.id, content: 'reply');
      final b = _event(45003, 30, parent: root.id);
      final rootEdit = _event(
        40003,
        40,
        content: 'new post',
        tags: [
          ['e', root.id],
        ],
      );
      final replyEdit = _event(
        40003,
        50,
        content: 'new reply',
        tags: [
          ['e', a.id],
        ],
      );
      final deletion = _event(
        5,
        60,
        tags: [
          ['e', b.id],
        ],
      );
      var deletedRoot = false;
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        return _window(
          filter,
          [b, a],
          thread: true,
          aux: [
            rootEdit,
            replyEdit,
            deletion,
            if (deletedRoot)
              _event(
                5,
                70,
                tags: [
                  ['e', root.id],
                ],
              ),
          ],
        );
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      final response = await container.read(provider.future);
      expect(response.post.content, 'new post');
      expect(response.replies.single.content, 'new reply');
      expect(response.replies.single.parentEventId, root.id);
      deletedRoot = true;
      await container.read(provider.notifier).refresh();
      expect(container.read(provider).error, isA<ForumPostDeleted>());
    },
  );

  test(
    'invalid thread signature never falls back or advances the retained cursor',
    () async {
      final root = _event(9, 10);
      final a = _event(9, 100, parent: root.id);
      final b = _event(45003, 90, parent: root.id);
      var corrupt = true;
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        return filter.until == null
            ? _window(filter, [a], next: a, thread: true)
            : _window(filter, [b], thread: true, corruptSignature: corrupt);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      await container.read(provider.future);
      await container.read(provider.notifier).loadMore();
      final failed = container.read(provider).requireValue;
      expect(failed.replies.single.eventId, a.id);
      expect(failed.nextCursor?.eventId, a.id);
      expect(failed.loadMoreError, isA<FormatException>());
      corrupt = false;
      await container.read(provider.notifier).loadMore();
      expect(
        container
            .read(provider)
            .requireValue
            .replies
            .map((reply) => reply.eventId),
        [b.id, a.id],
      );
      expect(
        session.filters
            .where((filter) => filter.ids == null)
            .every((filter) => filter.extensions['thread_window'] == true),
        isTrue,
      );
    },
  );

  test(
    'thread head refresh keeps older replies with constant query count',
    () async {
      final root = _event(9, 10);
      final older = _event(45003, 20, parent: root.id);
      final head = _event(40002, 30, parent: root.id);
      final latest = _event(9, 40, parent: root.id);
      var heads = 0;
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        return filter.until != null
            ? _window(filter, [older], thread: true)
            : ++heads == 1
            ? _window(filter, [head], next: head, thread: true)
            : _window(filter, [latest, head], next: head, thread: true);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      await container.read(provider.future);
      await container.read(provider.notifier).loadMore();
      final before = session.filters.length;
      await container.read(provider.notifier).refresh();
      expect(session.filters.length - before, 1);
      expect(
        container
            .read(provider)
            .requireValue
            .replies
            .map((reply) => reply.eventId),
        [older.id, head.id, latest.id],
      );
      expect(container.read(provider).requireValue.hasMore, isFalse);
    },
  );

  test(
    '200-page cap retains more/incomplete state rather than declaring exhaustion',
    () async {
      final root = _event(9, 1);
      var page = 0;
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        final reply = _event(45003, 1000 - page++, parent: root.id);
        return _window(filter, [reply], next: reply, thread: true);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      await container.read(provider.future);
      for (var i = 1; i < 200; i++) {
        await container.read(provider.notifier).loadMore();
      }
      final before = session.filters.length;
      await container.read(provider.notifier).loadMore();
      final response = container.read(provider).requireValue;
      expect(session.filters.length, before);
      expect(response.replies, hasLength(200));
      expect(response.hasMore, isTrue);
      expect(response.isIncomplete, isTrue);
      expect(response.loadMoreError, isA<StateError>());
    },
  );
}
