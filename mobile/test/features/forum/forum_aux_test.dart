import 'dart:convert';

import 'package:buzz/features/forum/forum_ancestry.dart';
import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/features/forum/forum_root_reader.dart';
import 'package:buzz/features/forum/forum_window.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _channel = '11111111-1111-4111-8111-111111111111';
const _url = 'http://relay.example';
const _alice =
    '0000000000000000000000000000000000000000000000000000000000000001';
const _bob = '0000000000000000000000000000000000000000000000000000000000000002';

NostrEvent _event(
  int kind,
  int time, {
  String key = _alice,
  String content = '',
  List<List<String>> tags = const [],
}) => NostrEvent.fromJson(
  jsonDecode(
        signNostrEvent(
          kind: kind,
          createdAt: time,
          secretKey: key,
          content: content,
          tags: [
            ['h', _channel],
            ...tags,
          ],
        ).toJson(),
      )
      as Map<String, dynamic>,
);

List<NostrEvent> _roots({int count = 50}) => [
  for (var i = 0; i < count; i++) _event(9, 300 - i, content: 'post $i'),
];

class _Session extends RelaySessionNotifier {
  final List<NostrEvent> Function(NostrFilter) answer;
  _Session(this.answer);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => answer(filters.single);
}

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: _url);
}

ProviderContainer _container(_Session session) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    relayConfigProvider.overrideWith(_Config.new),
    relaySessionProvider.overrideWith(() => session),
  ],
);

_Session _withCompleteAux(List<NostrEvent> roots, List<NostrEvent> aux) =>
    _Session((filter) {
      final targets = filter.tags['#e'];
      if (targets == null) {
        return roots
            .where(
              (event) =>
                  filter.until == null || event.createdAt <= filter.until!,
            )
            .take(filter.limit)
            .toList();
      }
      // Supply an explicit terminal empty aux response to isolate authority
      // from inclusive-boundary uncertainty. Static duplicate-boundary reads
      // are exercised separately below and in forum_read_regression_test.
      if (filter.until != null) return [];
      return aux
          .where(
            (event) =>
                filter.kinds.contains(event.kind) &&
                event.tags.any(
                  (tag) =>
                      tag.length >= 2 &&
                      tag[0] == 'e' &&
                      targets.contains(tag[1]),
                ),
          )
          .toList();
    });

void main() {
  for (final kind in [40003, 5]) {
    test(
      'unverified kind $kind keeps a mixed page readable and incomplete',
      () async {
        final roots = _roots();
        final foreign = _event(
          kind,
          500,
          key: _bob,
          content: 'unverified replacement',
          tags: [
            ['e', roots[0].id],
          ],
        );
        final edit = _event(
          40003,
          501,
          content: 'author update',
          tags: [
            ['e', roots[1].id],
          ],
        );
        final deletion = _event(
          5,
          502,
          tags: [
            ['e', roots[2].id],
          ],
        );
        final c = _container(
          _withCompleteAux(roots, [foreign, edit, deletion]),
        );
        addTearDown(c.dispose);
        final p = forumPostsProvider(_channel);
        final result = await c.read(p.future);
        expect(result.posts, hasLength(49));
        expect(
          result.posts
              .singleWhere((post) => post.eventId == roots[0].id)
              .content,
          'post 0',
        );
        expect(
          result.posts
              .singleWhere((post) => post.eventId == roots[1].id)
              .content,
          'author update',
        );
        expect(
          result.posts.map((post) => post.eventId),
          isNot(contains(roots[2].id)),
        );
        expect(result.hasUnconfirmedHistory, isTrue);
        expect(result.refreshError, isNull);
        expect(result.loadMoreError, isNull);
        expect(c.read(p).hasError, isFalse);
      },
    );
  }

  test(
    'fallback cannot establish an owner deletion of their agent post',
    () async {
      final roots = _roots();
      final agent = _event(
        40002,
        roots.first.createdAt,
        key: _bob,
        content: 'agent post',
        tags: [
          ['auth', derivePublicKeyHex(_alice)],
        ],
      );
      roots[0] = agent;
      final ownerDeletion = _event(
        5,
        500,
        tags: [
          ['e', agent.id],
        ],
      );
      final c = _container(_withCompleteAux(roots, [ownerDeletion]));
      addTearDown(c.dispose);
      final result = await c.read(forumPostsProvider(_channel).future);
      expect(result.posts, hasLength(50));
      expect(
        result.posts.singleWhere((post) => post.eventId == agent.id).content,
        'agent post',
      );
      expect(result.hasUnconfirmedHistory, isTrue);
      expect(result.refreshError, isNull);
    },
  );

  for (final authorDeletion in [false, true]) {
    test(
      '${authorDeletion ? 'author' : 'foreign'} deletion of an edit respects auxiliary authority',
      () async {
        final roots = _roots();
        final edit = _event(
          40003,
          500,
          content: 'author update',
          tags: [
            ['e', roots[0].id],
          ],
        );
        final deletion = _event(
          5,
          501,
          key: authorDeletion ? _alice : _bob,
          tags: [
            ['e', edit.id],
          ],
        );
        final c = _container(_withCompleteAux(roots, [edit, deletion]));
        addTearDown(c.dispose);
        final result = await c.read(forumPostsProvider(_channel).future);
        expect(result.posts, hasLength(50));
        expect(
          result.posts.first.content,
          authorDeletion ? 'post 0' : 'author update',
        );
        expect(result.hasUnconfirmedHistory, !authorDeletion);
        expect(result.refreshError, isNull);
      },
    );
  }

  test(
    'author deletion ignores an unloaded reference without an incomplete notice',
    () async {
      final roots = _roots();
      final deletion = _event(
        5,
        500,
        tags: [
          ['e', roots[0].id],
          ['e', 'f' * 64],
        ],
      );
      final c = _container(_withCompleteAux(roots, [deletion]));
      addTearDown(c.dispose);
      final result = await c.read(forumPostsProvider(_channel).future);
      expect(result.posts, hasLength(49));
      expect(
        result.posts.map((post) => post.eventId),
        isNot(contains(roots[0].id)),
      );
      expect(result.hasUnconfirmedHistory, isFalse);
      expect(result.refreshError, isNull);
    },
  );

  test(
    'mixed loaded-author deletion is excluded rather than hiding another author',
    () async {
      final roots = _roots();
      roots[1] = _event(
        40002,
        roots[1].createdAt,
        key: _bob,
        content: 'bob post',
      );
      final deletion = _event(
        5,
        500,
        tags: [
          ['e', roots[0].id],
          ['e', roots[1].id],
        ],
      );
      final c = _container(_withCompleteAux(roots, [deletion]));
      addTearDown(c.dispose);
      final result = await c.read(forumPostsProvider(_channel).future);
      expect(result.posts, hasLength(50));
      expect(result.posts.first.content, 'post 0');
      expect(result.posts[1].content, 'bob post');
      expect(result.hasUnconfirmedHistory, isTrue);
      expect(result.refreshError, isNull);
    },
  );

  test(
    'unknown final edit target is unresolved while reference-only targets do not authorize it',
    () async {
      final roots = _roots();
      final edit = _event(
        40003,
        500,
        content: 'unresolved update',
        tags: [
          ['e', roots[0].id],
          ['e', 'f' * 64],
        ],
      );
      final c = _container(_withCompleteAux(roots, [edit]));
      addTearDown(c.dispose);
      final result = await c.read(forumPostsProvider(_channel).future);
      expect(result.posts.first.content, 'post 0');
      expect(result.hasUnconfirmedHistory, isTrue);
      expect(result.refreshError, isNull);
    },
  );

  test(
    'ignored deletion references never affect a subsequently loaded post',
    () async {
      final roots = _roots(count: 100);
      roots[50] = _event(
        40002,
        roots[50].createdAt,
        key: _bob,
        content: 'older bob post',
      );
      final deletion = _event(
        5,
        500,
        tags: [
          ['e', roots[0].id],
          ['e', roots[50].id],
        ],
      );
      final c = _container(_withCompleteAux(roots, [deletion]));
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      await c.read(p.future);
      await c.read(p.notifier).loadMore();
      final result = c.read(p).requireValue;
      expect(result.posts, hasLength(99));
      expect(
        result.posts
            .singleWhere((post) => post.eventId == roots[50].id)
            .content,
        'older bob post',
      );
      expect(result.hasUnconfirmedHistory, isTrue);
      await verifyForumEvents([deletion]);
      expect(deletion.tags.where((tag) => tag[0] == 'e'), hasLength(2));
    },
  );

  test(
    'author deletion keeps authorized targets from separately loaded pages',
    () async {
      final roots = _roots(count: 100);
      final deletion = _event(
        5,
        500,
        tags: [
          ['e', roots[0].id],
          ['e', roots[50].id],
        ],
      );
      final c = _container(_withCompleteAux(roots, [deletion]));
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      expect((await c.read(p.future)).posts, hasLength(49));
      await c.read(p.notifier).loadMore();
      final result = c.read(p).requireValue;
      expect(result.posts, hasLength(98));
      expect(
        result.posts.map((post) => post.eventId),
        isNot(anyOf(contains(roots[0].id), contains(roots[50].id))),
      );
      expect(result.hasUnconfirmedHistory, isFalse);
    },
  );

  test(
    'aux incompleteness follows affected loaded posts and clears on a complete reread',
    () async {
      final roots = _roots(count: 100);
      final reaction = _event(
        7,
        500,
        content: '+',
        tags: [
          ['e', roots[0].id],
        ],
      );
      var complete = false;
      final session = _Session((filter) {
        final targets = filter.tags['#e'];
        if (targets != null) {
          return !complete && targets.contains(roots[0].id) ? [reaction] : [];
        }
        return roots
            .where(
              (event) =>
                  filter.until == null || event.createdAt <= filter.until!,
            )
            .take(filter.limit)
            .toList();
      });
      final reader = ForumRootReader(session, _channel, _url, () {});
      final first = await reader.fetch(null) as ForumWindowPage;
      final second = await reader.fetch(first.nextCursor) as ForumWindowPage;
      expect(first.incompletePostIds, hasLength(50));
      expect(second.incompletePostIds, isEmpty);

      final c = _container(session);
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      expect((await c.read(p.future)).hasUnconfirmedHistory, isTrue);
      await c.read(p.notifier).loadMore();
      expect(c.read(p).requireValue.posts, hasLength(100));
      expect(
        c.read(p).requireValue.hasUnconfirmedHistory,
        isTrue,
        reason: 'The earlier affected posts are still loaded.',
      );
      complete = true;
      await c.read(p.notifier).refresh();
      expect(c.read(p).requireValue.posts, hasLength(100));
      expect(c.read(p).requireValue.hasUnconfirmedHistory, isFalse);
      expect(c.read(p).requireValue.refreshError, isNull);
    },
  );
}
