import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay_query_response.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/forum/forum_ancestry.dart';
import 'package:buzz/features/forum/forum_root_reader.dart';
import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/features/forum/forum_reader.dart';
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
              ['h', _channel],
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
  return [...rows, ...aux, bound];
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

NostrEvent _otherAuthorEvent(
  int kind,
  int time,
  String target, {
  String content = '',
}) => NostrEvent.fromJson(
  jsonDecode(
        signNostrEvent(
          kind: kind,
          createdAt: time,
          secretKey:
              '0000000000000000000000000000000000000000000000000000000000000002',
          tags: [
            ['h', _channel],
            ['e', target],
          ],
          content: content,
        ).toJson(),
      )
      as Map<String, dynamic>,
);

class _PendingAuth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;
}

class _SignedConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: _relay, nsec: nostr.Keys(_secret).nsec);
}

void main() {
  test('relay-clamped auxiliary reads must not silently omit edits', () async {
    final root = _event(9, 1, content: 'obsolete content');
    final roots = [root, for (var i = 2; i <= 50; i++) _event(9, i)];
    final edit = _event(
      40003,
      2,
      tags: [
        ['e', root.id],
      ],
      content: 'current content',
    );
    final auxiliary = [
      for (var i = 0; i < 105; i++)
        _event(
          7,
          1000 - i,
          tags: [
            ['e', root.id],
          ],
          content: '+',
        ),
      edit,
    ];
    final session = _Session((f) {
      if (f.extensions.isNotEmpty) return [];
      if (!f.tags.containsKey('#e')) return f.until == null ? roots : [];
      return f.tags['#e']!.contains(root.id)
          ? auxiliary
                .where(
                  (event) => f.until == null || event.createdAt <= f.until!,
                )
                .take(100)
                .toList()
          : [];
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumPostsProvider(_channel);
    final r = await c.read(p.future);
    expect(
      r.posts.singleWhere((post) => post.eventId == root.id).content,
      'current content',
      reason:
          'A standard relay cap below 1000 truncates the aux closure without any error/incomplete state.',
    );
    expect(r.refreshError, isNotNull);
    await c.read(p.notifier).loadMore();
    expect(c.read(p).requireValue.hasMore, isFalse);
    expect(c.read(p).requireValue.refreshError, isNotNull);
  });

  test('fallback must not treat a relay-clamped page as exhaustion', () async {
    final root = _event(9, 1);
    final events = [
      for (var i = 0; i < 205; i++) _event(9, 1000 - i, parent: root.id),
      root,
    ];
    final session = _Session((f) {
      if (f.extensions.isNotEmpty || f.tags.containsKey('#e')) return [];
      return events
          .where((e) => f.until == null || e.createdAt <= f.until!)
          .take(100)
          .toList();
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final r = await c.read(forumPostsProvider(_channel).future);
    expect(
      r.posts.isNotEmpty || r.hasMore,
      isTrue,
      reason:
          'A 100-result relay cap hid the root behind busy replies and was reported as exhausted.',
    );
  });

  test('saturated legacy auxiliary closure is an error', () async {
    final root = _event(9, 1);
    final aux = _event(
      7,
      2,
      tags: [
        ['e', root.id],
      ],
      content: '+',
    );
    final session = _Session((f) {
      if (f.extensions.isNotEmpty) return [];
      return f.tags.containsKey('#e') ? List.filled(1000, aux) : [root];
    });
    final c = _container(session);
    addTearDown(c.dispose);
    await expectLater(
      c.read(forumPostsProvider(_channel).future),
      throwsStateError,
    );
  });

  test('a head gap is exposed and filling it retains old rows', () async {
    final a = _event(9, 100), b = _event(45001, 90), old = _event(40002, 80);
    final newest = _event(9, 200), gap = _event(40002, 150);
    var refresh = false;
    final session = _Session((f) {
      if (f.until == null) {
        return refresh
            ? _window(f, [newest], next: newest)
            : _window(f, [a, b], next: b);
      }
      if (f.until == 200) return _window(f, [gap, a], next: a);
      if (f.until == 100) return _window(f, [b, old]);
      return _window(f, [old]);
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumPostsProvider(_channel);
    await c.read(p.future);
    await c.read(p.notifier).loadMore();
    refresh = true;
    await c.read(p.notifier).refresh();
    final r = c.read(p).requireValue;
    expect(r.nextCursor?.eventId, newest.id);
    expect(r.posts.map((p) => p.eventId), [newest.id, a.id, b.id, old.id]);
    await c.read(p.notifier).loadMore();
    await c.read(p.notifier).loadMore();
    expect(c.read(p).requireValue.posts.map((p) => p.eventId), [
      newest.id,
      gap.id,
      a.id,
      b.id,
      old.id,
    ]);
    expect(c.read(p).requireValue.hasMore, isFalse);
  });

  test('deletion within head refresh range removes stale row', () async {
    final a = _event(9, 100), b = _event(45001, 90), old = _event(40002, 80);
    var refresh = false;
    final session = _Session((f) {
      if (f.until != null) return _window(f, [old]);
      return refresh ? _window(f, [a], next: b) : _window(f, [a, b], next: b);
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumPostsProvider(_channel);
    await c.read(p.future);
    await c.read(p.notifier).loadMore();
    refresh = true;
    await c.read(p.notifier).refresh();
    expect(c.read(p).requireValue.posts.map((p) => p.eventId), [a.id, old.id]);
  });

  test('load older preserves a failed head refresh warning', () async {
    final root = _event(9, 1);
    final head = _event(9, 100, parent: root.id),
        older = _event(45003, 50, parent: root.id);
    var failHead = false;
    var failOlder = true;
    final session = _Session((f) {
      if (f.ids != null) return [root];
      if (f.until != null) {
        if (failOlder) throw RelayException(500, 'temporary failure');
        return _window(f, [older], thread: true);
      }
      if (failHead) throw RelayException(503, '{"error":"query timed out"}');
      return _window(f, [head], next: head, thread: true);
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumThreadProvider((channelId: _channel, eventId: root.id));
    await c.read(p.future);
    failHead = true;
    await c.read(p.notifier).refresh();
    expect(c.read(p).requireValue.refreshError, isNotNull);
    expect(forumPollingPaused(c.read(p)), isTrue);
    await c.read(p.notifier).loadMore();
    expect(c.read(p).requireValue.loadMoreError, isNotNull);
    expect(c.read(p).requireValue.refreshError, isNotNull);
    expect(forumPollingPaused(c.read(p)), isTrue);
    failOlder = false;
    await c.read(p.notifier).loadMore();
    expect(
      c.read(p).requireValue.refreshError,
      isNotNull,
      reason:
          'Success at the older cursor says nothing about the failed head refresh.',
    );
    expect(forumPollingPaused(c.read(p)), isTrue);
    failHead = false;
    await c.read(p.notifier).refresh();
    expect(c.read(p).requireValue.refreshError, isNull);
    expect(forumPollingPaused(c.read(p)), isFalse);
  });

  test(
    'production HTTP session excludes retired ancestor IDs after a community switch',
    () async {
      final root = _event(9, 10);
      final head = _event(9, 20, parent: root.id);
      final parent = _event(40002, 30, parent: root.id);
      final child = _event(45003, 40, parent: parent.id);
      late ProviderContainer c;
      var refresh = false;
      final requests = <http.Request>[];
      var scheduled = false;
      final session = RelaySessionNotifier(
        httpClientFactory: () => http_testing.MockClient((request) async {
          requests.add(request);
          final raw = (jsonDecode(request.body) as List).single as Map;
          List<NostrEvent> result;
          if (request.url.host != 'relay.example') {
            result = [];
          } else if ((raw['ids'] as List?)?.contains(root.id) == true) {
            result = [root];
          } else if (raw['ids'] != null) {
            result = [parent];
          } else {
            result = _window(
              NostrFilters.forumThread(root.id, _channel),
              refresh ? [child, head] : [head],
              thread: true,
            );
          }
          return http.Response(
            jsonEncode(result.map((e) => e.toJson()).toList()),
            200,
            headers: {'content-type': 'application/json'},
          );
        }),
        queryDecoder: (response) async {
          final events = await decodeRelayQueryResponse(response);
          if (refresh && !scheduled && events.any((e) => e.kind == 39007)) {
            scheduled = true;
            Timer.run(() {
              c
                  .read(relayConfigProvider.notifier)
                  .update(
                    baseUrl: 'https://other.example',
                    nsec: nostr.Keys(_secret).nsec,
                  );
              c.read(
                forumThreadProvider((channelId: _channel, eventId: root.id)),
              );
            });
          }
          return events;
        },
      );
      c = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          authProvider.overrideWith(_PendingAuth.new),
          relayConfigProvider.overrideWith(_SignedConfig.new),
          relaySessionProvider.overrideWith(() => session),
          relayAuthorProvider(_relay).overrideWith((ref) async => _author),
        ],
      );
      addTearDown(c.dispose);
      final p = forumThreadProvider((channelId: _channel, eventId: root.id));
      await c.read(p.future);
      refresh = true;
      await c.read(p.notifier).refresh();
      final leaked = requests.where(
        (r) => r.url.host == 'other.example' && r.body.contains(parent.id),
      );
      expect(
        leaked,
        isEmpty,
        reason:
            'An actual POST /query containing old-community parent/channel IDs reached the new origin.',
      );
    },
  );

  test('reject an unsigned parent before it can extend ancestry', () async {
    final root = _event(9, 1);
    final parent = _event(9, 2, parent: root.id);
    final child = _event(45003, 3, parent: parent.id);
    final invalidParent = NostrEvent.fromJson({
      ...parent.toJson(),
      'sig': '0' * 128,
    });
    final session = _Session((f) {
      if (f.ids?.contains(root.id) == true) return [root];
      if (f.ids != null) return [invalidParent];
      return _window(f, [child], thread: true);
    });
    final c = _container(session);
    addTearDown(c.dispose);
    await expectLater(
      c.read(
        forumThreadProvider((channelId: _channel, eventId: root.id)).future,
      ),
      throwsA(isA<FormatException>()),
    );
  });

  test('legacy same-second saturation remains incomplete', () async {
    final roots = [for (var i = 0; i < 201; i++) _event(9, 100, content: '$i')]
      ..sort((a, b) => a.id.compareTo(b.id));
    final session = _Session((f) {
      if (f.extensions.isNotEmpty || f.tags.containsKey('#e')) return [];
      return roots.take(200).toList();
    });
    final c = _container(session);
    addTearDown(c.dispose);
    final p = forumPostsProvider(_channel);
    await c.read(p.future);
    for (var i = 0; i < 4; i++) {
      await c.read(p.notifier).loadMore();
    }
    final r = c.read(p).requireValue;
    expect(r.posts, hasLength(200));
    expect(r.hasMore, isTrue);
    expect(r.loadMoreError, isA<StateError>());
  });

  test('malformed channel cursor must trigger clean fallback', () async {
    final root = _event(9, 100);
    var clean = 0;
    final session = _Session((f) {
      if (f.extensions.isEmpty) {
        clean++;
        return f.tags.containsKey('#e') ? [] : [root];
      }
      final events = _window(f, [root], next: root);
      final bound = events.last;
      final malformed = NostrEvent.fromJson(
        jsonDecode(
              signNostrEvent(
                kind: 39006,
                createdAt: 9000,
                secretKey: _secret,
                tags: bound.tags,
                content:
                    '{"has_more":true,"next_cursor":{"created_at":-1.5,"id":"bad"}}',
              ).toJson(),
            )
            as Map<String, dynamic>,
      );
      return [root, malformed];
    });
    final c = _container(session);
    addTearDown(c.dispose);
    await c.read(forumPostsProvider(_channel).future);
    expect(
      clean,
      greaterThan(0),
      reason:
          'Malformed cursor must be discarded, not accepted as a successful page.',
    );
  });

  for (final kind in [40003, 5]) {
    test(
      'signed foreign-author auxiliary kind $kind cannot alter the root',
      () async {
        final root = _event(9, 100, content: 'authentic original');
        final aux = _otherAuthorEvent(
          kind,
          200,
          root.id,
          content: 'attacker replacement',
        );
        final session = _Session((f) {
          if (f.extensions.isNotEmpty) return [];
          if (!f.tags.containsKey('#e')) return [root];
          return f.tags['#e']!.contains(root.id) ? [aux] : [];
        });
        final c = _container(session);
        addTearDown(c.dispose);
        c
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'http://relay.example');
        await expectLater(
          c.read(forumPostsProvider(_channel).future),
          throwsA(
            isA<StateError>().having(
              (e) => e.message,
              'message',
              contains('author'),
            ),
          ),
        );
      },
    );
  }

  test('retired ancestry reads must not query the new community', () async {
    final root = _event(9, 10);
    final head = _event(9, 20, parent: root.id);
    final parent = _event(40002, 30, parent: root.id);
    final child = _event(45003, 40, parent: parent.id);
    late ProviderContainer c;
    var refresh = false;
    final sentToOther = <NostrFilter>[];
    final switched = Completer<void>();
    final session = _Session((f) {
      final origin = c.read(relayConfigProvider).baseUrl;
      if (origin != _relay) {
        sentToOther.add(f);
        return [];
      }
      if (f.ids?.contains(root.id) == true) return [root];
      if (f.ids != null) return [parent];
      if (!refresh) return _window(f, [head], thread: true);
      Timer.run(() {
        c
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'https://other.example');
        // Reading starts the new scope's build before the old compute finishes.
        c.read(forumThreadProvider((channelId: _channel, eventId: root.id)));
        switched.complete();
      });
      return _window(f, [child, head], thread: true);
    });
    c = _container(session);
    addTearDown(c.dispose);
    final p = forumThreadProvider((channelId: _channel, eventId: root.id));
    await c.read(p.future);
    refresh = true;
    await c.read(p.notifier).refresh();
    await switched.future;
    expect(
      sentToOther.where((f) => f.ids?.contains(parent.id) == true),
      isEmpty,
      reason: 'Old community ancestor IDs were sent after scope retirement.',
    );
  });

  test(
    'cross-channel window rows fail without selecting sticky fallback',
    () async {
      final root = _event(9, 100);
      final foreign = _event(
        9,
        90,
        channel: '22222222-2222-4222-8222-222222222222',
      );
      var invalid = true;
      final session = _Session((f) => _window(f, invalid ? [foreign] : [root]));
      final reader = ForumRootReader(session, _channel, _relay, () {});
      await expectLater(reader.fetch(null), throwsFormatException);
      expect(session.filters, hasLength(1));
      invalid = false;
      expect((await reader.fetch(null)).rows.single.event.id, root.id);
      expect(session.filters.last.extensions['top_level'], isTrue);
    },
  );

  test(
    'bad summary with usable bounds fails without selecting fallback',
    () async {
      final root = _event(9, 100);
      final badSummary = _event(
        39005,
        200,
        content: 'invalid json',
        tags: [
          ['e', root.id],
        ],
      );
      var invalid = true;
      final session = _Session(
        (f) => _window(f, [root], aux: invalid ? [badSummary] : []),
      );
      final reader = ForumRootReader(session, _channel, _relay, () {});
      await expectLater(reader.fetch(null), throwsA(anything));
      expect(session.filters, hasLength(1));
      invalid = false;
      expect((await reader.fetch(null)).rows.single.event.id, root.id);
      expect(session.filters.last.extensions['top_level'], isTrue);
    },
  );

  test(
    'exhausted head refresh removes absent retained rows in the full range',
    () async {
      final head = _event(9, 100),
          old = _event(45001, 50),
          newest = _event(40002, 200);
      var refresh = false;
      final session = _Session((f) {
        if (f.until != null) return _window(f, [old]);
        return refresh ? _window(f, [newest]) : _window(f, [head], next: head);
      });
      final c = _container(session);
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      await c.read(p.future);
      await c.read(p.notifier).loadMore();
      expect(c.read(p).requireValue.posts, hasLength(2));
      refresh = true;
      await c.read(p.notifier).refresh();
      expect(c.read(p).requireValue.posts.single.eventId, newest.id);
      expect(c.read(p).requireValue.hasMore, isFalse);
    },
  );

  for (final thread in [false, true]) {
    test(
      'served ${thread ? 'thread' : 'channel'} windows retain relay-authorized non-author deletion',
      () async {
        final root = _event(9, 1), row = _event(9, 2);
        final reply = _event(45003, 3, parent: root.id);
        final target = thread ? reply : row;
        final deletion = _otherAuthorEvent(9005, 10, target.id);
        final session = _Session((f) {
          if (f.ids != null) return [root];
          return _window(
            f,
            thread ? [reply] : [row, root],
            aux: [deletion],
            thread: thread,
          );
        });
        final c = _container(session);
        addTearDown(c.dispose);
        if (thread) {
          final r = await c.read(
            forumThreadProvider((channelId: _channel, eventId: root.id)).future,
          );
          expect(r.replies, isEmpty);
          expect(r.isIncomplete, isFalse);
        } else {
          final r = await c.read(forumPostsProvider(_channel).future);
          expect(r.posts.single.eventId, root.id);
        }
      },
    );
  }

  test(
    'ancestry requests stop at exactly one thousand unique parents',
    () async {
      final root = _event(9, 1);
      final ancestors = <String, NostrEvent>{};
      final rows = <NostrEvent>[];
      // 37 independent chains make the final batch smaller than a normal batch.
      for (var chain = 0; chain < 37; chain++) {
        var parent = root;
        for (var depth = 0; depth < 31; depth++) {
          final event = _event(
            9,
            depth + 2,
            parent: parent.id,
            root: root.id,
            content: '$chain:$depth',
          );
          ancestors[event.id] = event;
          parent = event;
        }
        rows.add(parent);
      }
      final session = _Session(
        (f) => [for (final id in f.ids!) ?ancestors[id]],
      );
      final ancestry = ForumAncestry(root);
      await ancestry.resolve(session, rows, () {});
      expect(session.filters.expand((f) => f.ids!).length, 1000);
      expect(ancestry.verified, {root.id});
    },
  );

  test(
    'empty clamping legacy relay proves empty forum without continuation',
    () async {
      final session = _Session((f) => <NostrEvent>[].take(100).toList());
      final c = _container(session);
      addTearDown(c.dispose);
      final r = await c.read(forumPostsProvider(_channel).future);
      expect(r.posts, isEmpty);
      expect(r.hasMore, isFalse);
      expect(r.refreshError, isNull);
      expect(session.filters, hasLength(2));
      expect(session.filters.last.extensions, isEmpty);
    },
  );

  test(
    'clamped fallback scan budget leaves an empty page incomplete and continuable',
    () async {
      final root = _event(9, 1);
      final rows = [
        for (var i = 0; i < 2005; i++) _event(9, 10000 - i, parent: root.id),
        root,
      ];
      final session = _Session((f) {
        if (f.extensions.isNotEmpty || f.tags.containsKey('#e')) return [];
        return rows
            .where((e) => f.until == null || e.createdAt <= f.until!)
            .take(100)
            .toList();
      });
      final c = _container(session);
      addTearDown(c.dispose);
      final p = forumPostsProvider(_channel);
      final r = await c.read(p.future);
      expect(r.posts, isEmpty);
      expect(r.hasMore, isTrue);
      expect(session.filters, hasLength(21));
      await c.read(p.notifier).loadMore();
      expect(c.read(p).requireValue.posts.single.eventId, root.id);
    },
  );

  for (final raw in [
    {'created_at': -1, 'id': 'a' * 64},
    {'created_at': 1.5, 'id': 'a' * 64},
    {'created_at': 1, 'id': 'A' * 64},
    {'created_at': 1, 'id': 'a' * 63},
    {'created_at': '1', 'id': 'a' * 64},
  ]) {
    test(
      'forum channel cursor grammar rejects $raw with clean fallback',
      () async {
        var clean = 0;
        final session = _Session((f) {
          if (f.extensions.isEmpty) {
            clean++;
            return [];
          }
          final original = _window(f, []).single;
          final malformed = _event(
            39006,
            9000,
            tags: original.tags.where((t) => t[0] != 'h').toList(),
            content: jsonEncode({'has_more': true, 'next_cursor': raw}),
          );
          return [malformed];
        });
        final c = _container(session);
        addTearDown(c.dispose);
        final r = await c.read(forumPostsProvider(_channel).future);
        expect(clean, 1);
        expect(r.hasMore, isFalse);
      },
    );
  }

  test(
    'fallback accepts author-signed channel-less deletion and rejects foreign deletion of aux',
    () async {
      final root = _event(9, 10);
      final deletion = NostrEvent.fromJson(
        jsonDecode(
              signNostrEvent(
                kind: 5,
                content: '',
                createdAt: 20,
                secretKey: _secret,
                tags: [
                  ['e', root.id],
                ],
              ).toJson(),
            )
            as Map<String, dynamic>,
      );
      final session = _Session((f) {
        if (f.extensions.isNotEmpty) return [];
        if (!f.tags.containsKey('#e')) return [root];
        expect(f.tags.containsKey('#h'), isFalse);
        return f.tags['#e']!.contains(root.id) ? [deletion] : [];
      });
      final c = _container(session);
      addTearDown(c.dispose);
      final r = await c.read(forumPostsProvider(_channel).future);
      expect(r.posts, isEmpty);
      expect(r.hasMore, isTrue);
      expect(r.refreshError, isNotNull);

      final reaction = _event(
        7,
        15,
        tags: [
          ['e', root.id],
        ],
        content: '+',
      );
      final foreignDeletion = _otherAuthorEvent(5, 30, reaction.id);
      final otherSession = _Session((f) {
        if (f.extensions.isNotEmpty) return [];
        if (!f.tags.containsKey('#e')) return [root];
        return f.tags['#e']!.contains(root.id) ? [reaction] : [foreignDeletion];
      });
      final other = _container(otherSession);
      addTearDown(other.dispose);
      await expectLater(
        other.read(forumPostsProvider(_channel).future),
        throwsA(
          isA<StateError>().having(
            (e) => e.message,
            'message',
            contains('author'),
          ),
        ),
      );
    },
  );

  for (final switchIdentity in [false, true]) {
    for (final stage in ['scan', 'aux', 'aux deletion']) {
      test(
        'production HTTP cancels retired $stage follow-up on ${switchIdentity ? 'identity' : 'community'} switch',
        () async {
          final root = _event(9, 10);
          final row = stage == 'scan' ? _event(9, 20, parent: root.id) : root;
          final edit = _event(
            40003,
            30,
            tags: [
              ['e', root.id],
            ],
            content: 'edited',
          );
          final oldNsec = nostr.Keys(_secret).nsec;
          late ProviderContainer c;
          final requests = <http.Request>[];
          var scheduled = false;
          var retired = false;
          var refresh = false;
          final retiredRequests = <http.Request>[];
          final session = RelaySessionNotifier(
            httpClientFactory: () => http_testing.MockClient((request) async {
              requests.add(request);
              if (retired) retiredRequests.add(request);
              final raw = (jsonDecode(request.body) as List).single as Map;
              final events = retired
                  ? <NostrEvent>[]
                  : (raw['#e'] as List?)?.contains(root.id) == true
                  ? (refresh ? [edit] : <NostrEvent>[])
                  : [refresh ? row : root];
              return http.Response(
                jsonEncode(events.map((e) => e.toJson()).toList()),
                200,
                headers: {'content-type': 'application/json'},
              );
            }),
            queryDecoder: (response) async {
              final events = await decodeRelayQueryResponse(response);
              if (refresh &&
                  !scheduled &&
                  (stage != 'aux deletion' ||
                      events.any((e) => e.kind == 40003))) {
                scheduled = true;
                Timer.run(() {
                  retired = true;
                  c
                      .read(relayConfigProvider.notifier)
                      .update(
                        baseUrl: switchIdentity
                            ? 'http://relay.example'
                            : 'http://other.example',
                        nsec: switchIdentity
                            ? nostr.Keys('0' * 63 + '2').nsec
                            : oldNsec,
                      );
                  c.read(forumPostsProvider(_channel));
                });
              }
              return events;
            },
          );
          c = ProviderContainer(
            retry: (_, _) => null,
            overrides: [
              authProvider.overrideWith(_PendingAuth.new),
              relayConfigProvider.overrideWith(_SignedConfig.new),
              relaySessionProvider.overrideWith(() => session),
            ],
          );
          addTearDown(c.dispose);
          c
              .read(relayConfigProvider.notifier)
              .update(baseUrl: 'http://relay.example', nsec: oldNsec);
          final p = forumPostsProvider(_channel);
          await c.read(p.future);
          refresh = true;
          await c.read(p.notifier).refresh();
          expect(scheduled, isTrue);
          expect(requests, isNotEmpty);
          expect(
            retiredRequests.where((request) {
              final raw = (jsonDecode(request.body) as List).single as Map;
              return raw['until'] != null || raw['#e'] != null;
            }),
            isEmpty,
            reason:
                'No follow-up from the retired source may use the replacement identity or community.',
          );
        },
      );
    }
  }
}
