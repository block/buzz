import 'dart:async';
import 'dart:convert';

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

ProviderContainer _container(_Session session, {String relayUrl = _relay}) {
  final c = ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      relayConfigProvider.overrideWith(_Config.new),
      relaySessionProvider.overrideWith(() => session),
      myPubkeyProvider.overrideWithValue(_author),
      relayAuthorProvider(relayUrl).overrideWith((ref) async => _author),
    ],
  );
  if (relayUrl != _relay) {
    c.read(relayConfigProvider.notifier).update(baseUrl: relayUrl);
  }
  return c;
}

const _second =
    '0000000000000000000000000000000000000000000000000000000000000002';

NostrEvent _signedAs(
  int kind,
  int time,
  String secret,
  List<List<String>> tags,
  String content,
) => NostrEvent.fromJson(
  jsonDecode(
        signNostrEvent(
          kind: kind,
          createdAt: time,
          secretKey: secret,
          tags: [
            ['h', _channel],
            ...tags,
          ],
          content: content,
        ).toJson(),
      )
      as Map<String, dynamic>,
);

void main() {
  for (final path in ['channel', 'thread', 'fallback']) {
    test(
      '$path excludes an edit with different authorized and applied targets',
      () async {
        final victim = _event(9, 10, content: 'original post');
        final own = _signedAs(9, 20, _second, [], 'other post');
        // On fallback, even an edit signed by the rendered target's author
        // cannot establish that the first authorized target is the same event.
        final edit = _signedAs(
          40003,
          30,
          path == 'fallback' ? _secret : _second,
          [
            ['e', own.id],
            ['e', victim.id],
          ],
          'replacement',
        );
        final session = _Session((f) {
          if (f.ids != null) return [victim];
          if (path == 'fallback') {
            if (f.extensions.isNotEmpty) return [];
            if (!f.tags.containsKey('#e')) return [own, victim];
            return f.until == null && f.tags['#e']!.contains(victim.id)
                ? [edit]
                : [];
          }
          return _window(
            f,
            path == 'thread' ? [] : [own, victim],
            thread: path == 'thread',
            aux: [edit],
          );
        });
        final c = _container(session);
        addTearDown(c.dispose);
        if (path == 'thread') {
          final r = await c.read(
            forumThreadProvider((
              channelId: _channel,
              eventId: victim.id,
            )).future,
          );
          expect(r.post.content, 'original post');
          expect(r.post.pubkey, victim.pubkey);
          expect(r.isIncomplete, isTrue);
          expect(r.hasUnconfirmedHistory, isTrue);
          expect(r.refreshError, isNull);
        } else {
          final r = await c.read(forumPostsProvider(_channel).future);
          final post = r.posts.singleWhere((p) => p.eventId == victim.id);
          expect(post.content, 'original post');
          expect(post.pubkey, victim.pubkey);
          expect(r.hasUnconfirmedHistory, isTrue);
          expect(r.refreshError, isNull);
        }
      },
    );
  }

  for (final kind in [5, 9005, 40003]) {
    test('HTTP thread excludes unverified auxiliary kind $kind', () async {
      final root = _event(9, 10, content: 'root');
      final reply = _event(
        45003,
        20,
        parent: root.id,
        content: 'original reply',
      );
      final effect = _signedAs(kind, 30, _second, [
        ['e', reply.id],
      ], 'replacement');
      final session = _Session(
        (f) => f.ids != null
            ? [root]
            : _window(f, [reply], thread: true, aux: [effect]),
      );
      final c = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relayConfigProvider.overrideWith(_Config.new),
          relaySessionProvider.overrideWith(() => session),
          myPubkeyProvider.overrideWithValue(_author),
          relayAuthorProvider(
            'http://relay.example',
          ).overrideWith((ref) async => _author),
        ],
      );
      addTearDown(c.dispose);
      c
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'http://relay.example');
      final r = await c.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      );
      expect(r.replies, hasLength(1));
      expect(r.replies.single.content, 'original reply');
      expect(r.replies.single.pubkey, reply.pubkey);
      expect(r.isIncomplete, isTrue);
      expect(r.hasUnconfirmedHistory, isTrue);
      expect(r.refreshError, isNull);
    });
  }

  test('head refresh preserves an unretried continuation error', () async {
    final head = _event(9, 10);
    var failOlder = true;
    var failHead = false;
    final older = _event(9, 5);
    final session = _Session((f) {
      if (f.until != null) {
        if (failOlder) throw RelayException(500, 'older page failed');
        return _window(f, [older]);
      }
      if (failHead) throw RelayException(500, 'head failed');
      return _window(f, [head], next: head);
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumPostsProvider(_channel);
    await c.read(p.future);
    await c.read(p.notifier).loadMore();
    expect(c.read(p).requireValue.loadMoreError, isNotNull);
    await c.read(p.notifier).refresh();
    expect(c.read(p).requireValue.loadMoreError, isNotNull);
    failHead = true;
    await c.read(p.notifier).refresh();
    expect(c.read(p).requireValue.loadMoreError, isNotNull);
    expect(c.read(p).requireValue.refreshError, isNotNull);
    failOlder = false;
    await c.read(p.notifier).loadMore();
    expect(c.read(p).requireValue.loadMoreError, isNull);
    expect(c.read(p).requireValue.refreshError, isNotNull);
  });

  for (final path in ['channel', 'thread', 'fallback', 'HTTP']) {
    test(
      '$path preserves a consistent edit with uppercase first target',
      () async {
        final window = path == 'channel' || path == 'thread';
        final root = _signedAs(
          9,
          10,
          window ? _second : _secret,
          [],
          'original',
        );
        // Served windows retain relay-authorized owner edits. Standard filters
        // and HTTP threads require the event's own author instead.
        final edit = _signedAs(40003, 30, _secret, [
          ['e', 'not-an-id'],
          ['e', root.id.toUpperCase()],
          ['e', root.id],
        ], 'updated');
        final thread = path == 'thread' || path == 'HTTP';
        final session = _Session((f) {
          if (f.ids != null) return [root];
          if (path == 'fallback') {
            if (f.extensions.isNotEmpty) return [];
            if (!f.tags.containsKey('#e')) return [root];
            return f.until == null && f.tags['#e']!.contains(root.id)
                ? [edit]
                : [];
          }
          return _window(f, thread ? [] : [root], thread: thread, aux: [edit]);
        });
        final c = _container(
          session,
          relayUrl: path == 'HTTP' ? 'http://relay.example' : _relay,
        );
        addTearDown(c.dispose);
        if (thread) {
          final r = await c.read(
            forumThreadProvider((channelId: _channel, eventId: root.id)).future,
          );
          expect(r.post.content, 'updated');
          expect(r.hasUnconfirmedHistory, isFalse);
        } else {
          final r = await c.read(forumPostsProvider(_channel).future);
          expect(r.posts.single.content, 'updated');
          if (window) {
            expect(r.hasUnconfirmedHistory, isFalse);
          }
        }
      },
    );
  }

  test(
    'HTTPS thread excludes mismatched reply edits as well as root edits',
    () async {
      final root = _event(9, 1);
      final reply = _event(
        45003,
        10,
        parent: root.id,
        content: 'original reply',
      );
      final own = _signedAs(9, 20, _second, [], 'other post');
      final edit = _signedAs(40003, 30, _second, [
        ['e', own.id],
        ['e', reply.id],
      ], 'replacement');
      final session = _Session(
        (f) => f.ids != null
            ? [root]
            : _window(f, [reply], thread: true, aux: [edit]),
      );
      final c = _container(session);
      addTearDown(c.dispose);
      final r = await c.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      );
      expect(r.replies.single.content, 'original reply');
      expect(r.hasUnconfirmedHistory, isTrue);
      expect(r.refreshError, isNull);
    },
  );

  for (final kind in [5, 9005, 40003]) {
    test('HTTP thread applies author-signed auxiliary kind $kind', () async {
      final root = _event(9, 1);
      final reply = _event(
        45003,
        10,
        parent: root.id,
        content: 'original reply',
      );
      final effect = _event(
        kind,
        20,
        tags: [
          ['e', reply.id],
        ],
        content: 'updated',
      );
      final session = _Session(
        (f) => f.ids != null
            ? [root]
            : _window(f, [reply], thread: true, aux: [effect]),
      );
      final c = _container(session, relayUrl: 'http://relay.example');
      addTearDown(c.dispose);
      final r = await c.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      );
      if (kind == 40003) {
        expect(r.replies.single.content, 'updated');
      } else {
        expect(r.replies, isEmpty);
      }
      expect(r.hasUnconfirmedHistory, isFalse);
      expect(r.isIncomplete, isFalse);
    });
  }

  test('HTTP thread excludes a foreign deletion of an author edit', () async {
    final root = _event(9, 1);
    final reply = _event(45003, 10, parent: root.id, content: 'original reply');
    final edit = _event(
      40003,
      20,
      tags: [
        ['e', reply.id],
      ],
      content: 'updated',
    );
    final deletion = _signedAs(5, 30, _second, [
      ['e', edit.id],
    ], '');
    final session = _Session(
      (f) => f.ids != null
          ? [root]
          : _window(f, [reply], thread: true, aux: [edit, deletion]),
    );
    final c = _container(session, relayUrl: 'http://relay.example');
    addTearDown(c.dispose);
    final r = await c.read(
      forumThreadProvider((channelId: _channel, eventId: root.id)).future,
    );
    expect(r.replies.single.content, 'updated');
    expect(r.hasUnconfirmedHistory, isTrue);
    expect(r.refreshError, isNull);
  });

  for (final relayUrl in [_relay, 'http://relay.example']) {
    for (final part in ['root', 'row', 'aux', 'bounds']) {
      test(
        '$relayUrl thread rejects invalid $part signatures without downgrade',
        () async {
          final root = _event(9, 1);
          final reply = _event(45003, 10, parent: root.id);
          final edit = _event(
            40003,
            20,
            tags: [
              ['e', reply.id],
            ],
          );
          NostrEvent corrupt(NostrEvent e) =>
              NostrEvent.fromJson({...e.toJson(), 'sig': '0' * 128});
          final session = _Session(
            (f) => f.ids != null
                ? [part == 'root' ? corrupt(root) : root]
                : _window(
                    f,
                    [part == 'row' ? corrupt(reply) : reply],
                    thread: true,
                    aux: [part == 'aux' ? corrupt(edit) : edit],
                    corruptSignature: part == 'bounds',
                  ),
          );
          final c = _container(session, relayUrl: relayUrl);
          addTearDown(c.dispose);
          await expectLater(
            c.read(
              forumThreadProvider((
                channelId: _channel,
                eventId: root.id,
              )).future,
            ),
            throwsA(isA<FormatException>()),
          );
          expect(
            session.filters
                .where((f) => f.ids == null)
                .every((f) => f.extensions['thread_window'] == true),
            isTrue,
          );
        },
      );
    }
  }

  for (final http in [false, true]) {
    for (final valid in [false, true]) {
      test(
        '${http ? 'HTTP posts' : 'legacy fallback'} ${valid ? 'accepts signed' : 'rejects unsigned'} rows using standard filters',
        () async {
          final original = _event(9, 10, content: 'verified post');
          final row = valid
              ? original
              : NostrEvent.fromJson({...original.toJson(), 'sig': '0' * 128});
          final session = _Session((f) {
            if (f.extensions.isNotEmpty || f.tags.containsKey('#e')) return [];
            return [row];
          });
          final c = _container(
            session,
            relayUrl: http ? 'http://relay.example' : _relay,
          );
          addTearDown(c.dispose);
          final future = c.read(forumPostsProvider(_channel).future);
          if (valid) {
            final r = await future;
            expect(r.posts.single.content, 'verified post');
            expect(r.hasUnconfirmedContinuation, isTrue);
            expect(r.hasUnconfirmedHistory, isTrue);
          } else {
            await expectLater(future, throwsA(isA<FormatException>()));
          }
          if (http) {
            expect(session.filters.every((f) => f.extensions.isEmpty), isTrue);
          }
          expect(
            session.filters.any(
              (f) => f.extensions.isEmpty && !f.tags.containsKey('#e'),
            ),
            isTrue,
          );
        },
      );
    }
  }

  test(
    'HTTPS channel uses authenticated transport for row provenance',
    () async {
      final original = _event(9, 10, content: 'relay-served post');
      final row = NostrEvent.fromJson({...original.toJson(), 'sig': '0' * 128});
      final session = _Session((f) => _window(f, [row]));
      final c = _container(session);
      addTearDown(c.dispose);
      final r = await c.read(forumPostsProvider(_channel).future);
      expect(r.posts.single.content, 'relay-served post');
      expect(session.filters.single.extensions['top_level'], isTrue);
    },
  );

  for (final http in [false, true]) {
    test(
      '${http ? 'HTTP' : 'HTTPS'} thread resolves signed out-of-page ancestry and excludes references',
      () async {
        final root = _event(9, 1);
        final parent = _event(9, 10, parent: root.id);
        final nested = _event(45003, 20, parent: parent.id, root: root.id);
        final quote = _event(
          9,
          30,
          tags: [
            ['q', root.id],
          ],
        );
        final mention = _event(
          9,
          40,
          tags: [
            ['e', root.id, '', 'mention'],
          ],
        );
        final session = _Session((f) {
          if (f.ids != null) {
            return [root, parent].where((e) => f.ids!.contains(e.id)).toList();
          }
          return _window(f, [mention, quote, nested], thread: true);
        });
        final c = _container(
          session,
          relayUrl: http ? 'http://relay.example' : _relay,
        );
        addTearDown(c.dispose);
        final r = await c.read(
          forumThreadProvider((channelId: _channel, eventId: root.id)).future,
        );
        expect(r.replies.single.eventId, nested.id);
        expect(r.unresolvedReplies, 2);
        expect(r.isIncomplete, isTrue);
        expect(
          session.filters.any((f) => f.ids?.contains(parent.id) == true),
          isTrue,
        );
      },
    );
  }

  test(
    'an exhausted head refresh makes an old continuation error obsolete',
    () async {
      final head = _event(9, 10);
      var exhausted = false;
      final session = _Session((f) {
        if (f.until != null) throw RelayException(500, 'older page failed');
        return _window(f, [head], next: exhausted ? null : head);
      });
      final c = _container(session);
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      await c.read(p.future);
      await c.read(p.notifier).loadMore();
      expect(c.read(p).requireValue.loadMoreError, isNotNull);
      exhausted = true;
      await c.read(p.notifier).refresh();
      expect(c.read(p).requireValue.loadMoreError, isNull);
      expect(c.read(p).requireValue.hasMore, isFalse);
    },
  );

  test(
    'HTTP thread retains per-page deletion authority while older replies load',
    () async {
      final root = _event(9, 1);
      final head = _event(45003, 100, parent: root.id, content: 'new reply');
      final older = _signedAs(45003, 50, _second, [
        ['e', root.id, '', 'reply'],
      ], 'older reply');
      final deletion = _event(
        5,
        200,
        tags: [
          ['e', head.id],
          ['e', older.id],
        ],
      );
      final session = _Session((f) {
        if (f.ids != null) return [root];
        return f.until == null
            ? _window(f, [head], next: head, thread: true, aux: [deletion])
            : _window(f, [older], thread: true, aux: [deletion]);
      });
      final c = _container(session, relayUrl: 'http://relay.example');
      addTearDown(c.dispose);
      final p = forumThreadProvider((channelId: _channel, eventId: root.id));
      expect((await c.read(p.future)).replies, isEmpty);
      await c.read(p.notifier).loadMore();
      final r = c.read(p).requireValue;
      expect(r.replies.map((reply) => reply.eventId), [older.id]);
      expect(r.hasUnconfirmedHistory, isTrue);
      expect(r.refreshError, isNull);
    },
  );

  for (final kind in [40003, 5, 9005]) {
    test(
      'HTTP thread preserves its root for unverified auxiliary kind $kind without reply rows',
      () async {
        final root = _event(9, 1, content: 'original root');
        final effect = _signedAs(kind, 10, _second, [
          ['e', root.id],
        ], 'replacement');
        final session = _Session(
          (f) => f.ids != null
              ? [root]
              : _window(f, [], thread: true, aux: [effect]),
        );
        final c = _container(session, relayUrl: 'http://relay.example');
        addTearDown(c.dispose);
        final r = await c.read(
          forumThreadProvider((channelId: _channel, eventId: root.id)).future,
        );
        expect(r.post.content, 'original root');
        expect(r.hasUnconfirmedHistory, isTrue);
        expect(r.refreshError, isNull);
      },
    );
  }

  for (final http in [false, true]) {
    test(
      '${http ? 'HTTP posts' : 'legacy fallback'} reuse root classification for replies and references',
      () async {
        final root = _event(9, 1);
        final agent = _event(40002, 2);
        final mobile = _event(45001, 3);
        final reply = _event(9, 4, parent: root.id);
        final nested = _event(40002, 5, parent: reply.id, root: root.id);
        final quote = _event(
          9,
          6,
          tags: [
            ['q', root.id],
          ],
        );
        final mention = _event(
          40002,
          7,
          tags: [
            ['e', root.id, '', 'mention'],
          ],
        );
        final session = _Session((f) {
          if (f.extensions.isNotEmpty || f.tags.containsKey('#e')) return [];
          return [mention, quote, nested, reply, mobile, agent, root];
        });
        final c = _container(
          session,
          relayUrl: http ? 'http://relay.example' : _relay,
        );
        addTearDown(c.dispose);
        final r = await c.read(forumPostsProvider(_channel).future);
        expect(r.posts.map((p) => p.eventId), [
          mention.id,
          quote.id,
          mobile.id,
          agent.id,
          root.id,
        ]);
        expect(r.hasUnconfirmedContinuation, isTrue);
      },
    );
  }
}
