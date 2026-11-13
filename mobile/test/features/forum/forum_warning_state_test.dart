// Warning-state regressions through production providers and notifier methods.

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

void main() {
  test(
    'P3 1: head gap preserves a failed older-page error until continuation succeeds or history is exhausted',
    () async {
      final old = _event(9, 10, content: 'retained older post');
      final newer = _event(9, 100, content: 'new head');
      var gap = false;
      var failOlder = true;
      var exhausted = false;
      final session = _Session((filter) {
        if (filter.until != null) {
          if (failOlder) throw RelayException(500, 'older page failed');
          // A successful continuation clears the error even when more history
          // remains. It closes the head gap and returns to the older cursor.
          return _window(filter, [old], next: old);
        }
        if (exhausted) return _window(filter, [newer, old]);
        return _window(filter, gap ? [newer] : [old], next: gap ? newer : old);
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumPostsProvider(_channel);
      final notifier = container.read(provider.notifier);

      await container.read(provider.future);
      await notifier.loadMore();
      final originalError = container.read(provider).requireValue.loadMoreError;
      expect(originalError, isNotNull);

      gap = true;
      await notifier.refresh();
      final afterGap = container.read(provider).requireValue;
      expect(afterGap.posts.map((post) => post.eventId), [newer.id, old.id]);
      expect(afterGap.hasMore, isTrue);
      expect(afterGap.nextCursor!.eventId, newer.id);
      expect(afterGap.nextCursor!.createdAt, newer.createdAt);
      expect(afterGap.refreshError, isNull);

      // Run both legitimate clearing controls before the red assertion, so a
      // blanket "never clear errors" fix cannot satisfy the regression.
      failOlder = false;
      await notifier.loadMore();
      final afterContinuation = container.read(provider).requireValue;
      expect(afterContinuation.hasMore, isTrue);
      expect(afterContinuation.nextCursor!.eventId, old.id);
      expect(afterContinuation.loadMoreError, isNull);

      failOlder = true;
      await notifier.loadMore();
      expect(container.read(provider).requireValue.loadMoreError, isNotNull);
      exhausted = true;
      await notifier.refresh();
      final afterExhaustion = container.read(provider).requireValue;
      expect(afterExhaustion.hasMore, isFalse);
      expect(afterExhaustion.loadMoreError, isNull);

      expect(
        afterGap.loadMoreError,
        isNotNull,
        reason:
            'The head-gap refresh changed the cursor but neither retried '
            'the failed older range nor proved history exhausted.',
      );
      expect(afterGap.loadMoreError, same(originalError));
    },
  );

  for (final thread in [false, true]) {
    test(
      'P3 2: ${thread ? "thread" : "channel"} mismatch uncertainty survives retained targets and clears after an exhausted reread',
      () async {
        final root = _event(9, 1, content: 'thread root');
        final target = _event(
          thread ? 45003 : 9,
          10,
          parent: thread ? root.id : null,
          content: 'original target content',
        );
        // This is a real signed event by the edit author. In thread mode it is
        // outside the requested thread; in channel mode both targets are loaded.
        final own = _event(9, 5, content: 'authorized first target');
        final edit = _event(
          40003,
          20,
          tags: [
            ['e', own.id],
            ['e', target.id],
          ],
          content: 'excluded mismatched edit',
        );
        final newer = _event(
          thread ? 45003 : 9,
          100,
          parent: thread ? root.id : null,
          content: 'new head content',
        );
        var stage = 0;
        final session = _Session((filter) {
          if (filter.ids != null) return [root];
          if (stage == 0) {
            return _window(
              filter,
              [target, if (!thread) own],
              thread: thread,
              aux: [edit],
            );
          }
          if (stage == 1) {
            // This head page does not reread the affected older range. Omission
            // of its edit cannot clear uncertainty for retained affected rows.
            return _window(filter, [newer], next: newer, thread: thread);
          }
          // Valid exhausted bounds prove every previously loaded row absent.
          return _window(filter, [], thread: thread);
        });
        final container = _container(session);
        addTearDown(container.dispose);

        if (thread) {
          final provider = forumThreadProvider((
            channelId: _channel,
            eventId: root.id,
          ));
          final notifier = container.read(provider.notifier);
          final initial = await container.read(provider.future);
          expect(initial.replies.single.content, 'original target content');
          expect(initial.hasUnconfirmedHistory, isTrue);

          stage = 1;
          await notifier.refresh();
          final retained = container.read(provider).requireValue;
          expect(
            retained.replies.map((reply) => reply.eventId),
            contains(target.id),
          );
          expect(retained.hasMore, isTrue);
          expect(
            retained.hasUnconfirmedHistory,
            isTrue,
            reason: 'An affected older reply is still retained.',
          );

          stage = 2;
          await notifier.refresh();
          final exhausted = container.read(provider).requireValue;
          expect(exhausted.post.eventId, root.id);
          expect(exhausted.post.content, 'thread root');
          expect(exhausted.replies, isEmpty);
          expect(exhausted.hasMore, isFalse);
          expect(exhausted.refreshError, isNull);
          expect(exhausted.loadMoreError, isNull);
          expect(exhausted.unresolvedReplies, 0);
          expect(
            exhausted.hasUnconfirmedHistory,
            isFalse,
            reason:
                'The exhausted reread removed every affected thread row; '
                'the retained mismatched edit must not keep the warning alive.',
          );
          expect(exhausted.isIncomplete, isFalse);
        } else {
          final provider = forumPostsProvider(_channel);
          final notifier = container.read(provider.notifier);
          final initial = await container.read(provider.future);
          expect(initial.posts.first.content, 'original target content');
          expect(initial.hasUnconfirmedHistory, isTrue);

          stage = 1;
          await notifier.refresh();
          final retained = container.read(provider).requireValue;
          expect(
            retained.posts.map((post) => post.eventId),
            containsAll([target.id, own.id]),
          );
          expect(retained.hasMore, isTrue);
          expect(
            retained.hasUnconfirmedHistory,
            isTrue,
            reason: 'Affected older posts are still retained.',
          );

          stage = 2;
          await notifier.refresh();
          final exhausted = container.read(provider).requireValue;
          expect(exhausted.posts, isEmpty);
          expect(exhausted.hasMore, isFalse);
          expect(exhausted.refreshError, isNull);
          expect(exhausted.loadMoreError, isNull);
          expect(
            exhausted.hasUnconfirmedHistory,
            isFalse,
            reason:
                'The exhausted reread removed both edit targets; '
                'the retained mismatched edit must not keep the warning alive.',
          );
        }
      },
    );

    for (final removeTarget in [false, true]) {
      test(
        '${thread ? "thread" : "channel"} clean ${removeTarget ? "removal" : "reread"} retires both edit targets independently',
        () async {
          final root = _event(9, 0, content: 'root');
          final target = _event(
            thread ? 45003 : 9,
            10,
            parent: thread ? root.id : null,
            content: 'original target',
          );
          final first = _event(
            thread ? 45003 : 9,
            5,
            parent: thread ? root.id : null,
            content: 'original first target',
          );
          final older = _event(
            thread ? 45003 : 9,
            1,
            parent: thread ? root.id : null,
            content: 'unaffected older row',
          );
          final newer = _event(
            thread ? 45003 : 9,
            20,
            parent: thread ? root.id : null,
          );
          final edit = _event(
            40003,
            30,
            tags: [
              ['e', 'not-an-event-id'],
              ['e', first.id.toUpperCase()],
              ['e', older.id],
              ['e', target.id],
            ],
            content: 'excluded edit',
          );
          var stage = 0;
          final session = _Session((filter) {
            if (filter.ids != null) return [root];
            return _window(
              filter,
              stage == 0
                  ? [target, first, older]
                  : [
                      if (removeTarget) newer else target,
                      if (stage == 2 && !removeTarget) first,
                    ],
              next: stage == 0 ? older : (stage == 1 ? target : first),
              thread: thread,
              aux: stage == 0 ? [edit] : [],
            );
          });
          final container = _container(session);
          addTearDown(container.dispose);
          if (thread) {
            final provider = forumThreadProvider((
              channelId: _channel,
              eventId: root.id,
            ));
            final initial = await container.read(provider.future);
            expect(initial.hasUnconfirmedHistory, isTrue);
            expect(
              initial.replies.map((row) => row.content),
              containsAll(['original target', 'original first target']),
            );
            expect(
              initial.replies.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
            stage = 1;
            await container.read(provider.notifier).refresh();
            final result = container.read(provider).requireValue;
            expect(result.refreshError, isNull);
            expect(
              result.replies.map((row) => row.eventId),
              contains(older.id),
            );
            expect(
              result.hasUnconfirmedHistory,
              isTrue,
              reason: 'The first edit target has not been reread or removed.',
            );
            expect(
              result.replies
                  .singleWhere((row) => row.eventId == first.id)
                  .content,
              'original first target',
            );
            expect(result.hasMore, isTrue);
            expect(
              result.replies.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
            expect(
              result.replies.any((row) => row.eventId == target.id),
              !removeTarget,
            );
            stage = 2;
            await container.read(provider.notifier).refresh();
            final cleared = container.read(provider).requireValue;
            expect(cleared.refreshError, isNull);
            expect(cleared.hasUnconfirmedHistory, isFalse);
            expect(
              cleared.replies.map((row) => row.eventId),
              contains(older.id),
            );
            expect(cleared.hasMore, isTrue);
            expect(
              cleared.replies.any((row) => row.eventId == first.id),
              !removeTarget,
            );
            expect(
              cleared.replies.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
          } else {
            final provider = forumPostsProvider(_channel);
            final initial = await container.read(provider.future);
            expect(initial.hasUnconfirmedHistory, isTrue);
            expect(
              initial.posts.map((row) => row.content),
              containsAll(['original target', 'original first target']),
            );
            expect(
              initial.posts.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
            stage = 1;
            await container.read(provider.notifier).refresh();
            final result = container.read(provider).requireValue;
            expect(result.refreshError, isNull);
            expect(result.posts.map((row) => row.eventId), contains(older.id));
            expect(
              result.hasUnconfirmedHistory,
              isTrue,
              reason: 'The first edit target has not been reread or removed.',
            );
            expect(
              result.posts
                  .singleWhere((row) => row.eventId == first.id)
                  .content,
              'original first target',
            );
            expect(result.hasMore, isTrue);
            expect(
              result.posts.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
            expect(
              result.posts.any((row) => row.eventId == target.id),
              !removeTarget,
            );
            stage = 2;
            await container.read(provider.notifier).refresh();
            final cleared = container.read(provider).requireValue;
            expect(cleared.refreshError, isNull);
            expect(cleared.hasUnconfirmedHistory, isFalse);
            expect(cleared.posts.map((row) => row.eventId), contains(older.id));
            expect(cleared.hasMore, isTrue);
            expect(
              cleared.posts.any((row) => row.eventId == first.id),
              !removeTarget,
            );
            expect(
              cleared.posts.any((row) => row.content == 'excluded edit'),
              isFalse,
            );
          }
        },
      );
    }
  }

  test(
    'root mismatch warning clears on a clean head reread with older replies retained',
    () async {
      final root = _event(9, 0, content: 'original root');
      final reply = _event(45003, 10, parent: root.id);
      final older = _event(45003, 1, parent: root.id);
      final edit = _event(
        40003,
        20,
        tags: [
          ['e', reply.id],
          ['e', root.id],
        ],
        content: 'excluded root edit',
      );
      var refreshed = false;
      final session = _Session((filter) {
        if (filter.ids != null) return [root];
        if (filter.until != null) {
          return _window(filter, [older], thread: true);
        }
        return _window(
          filter,
          [reply],
          next: reply,
          thread: true,
          aux: refreshed ? [] : [edit],
        );
      });
      final container = _container(session);
      addTearDown(container.dispose);
      final provider = forumThreadProvider((
        channelId: _channel,
        eventId: root.id,
      ));
      final notifier = container.read(provider.notifier);
      final initial = await container.read(provider.future);
      expect(initial.post.content, 'original root');
      expect(initial.hasUnconfirmedHistory, isTrue);
      await notifier.loadMore();
      expect(
        container.read(provider).requireValue.hasUnconfirmedHistory,
        isTrue,
      );
      refreshed = true;
      await notifier.refresh();
      final result = container.read(provider).requireValue;
      expect(result.replies.map((row) => row.eventId), contains(older.id));
      expect(result.post.content, 'original root');
      expect(result.hasUnconfirmedHistory, isFalse);
    },
  );
}
