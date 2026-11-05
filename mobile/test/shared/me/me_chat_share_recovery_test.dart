import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/me_chat_share.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';

void main() {
  for (final drift in [-901, -900, 900, 901]) {
    testWidgets('relay enforces timestamp drift $drift seconds', (
      tester,
    ) async {
      final now = DateTime(2026, 11, 4);
      final relay = FakeRelay(now: () => now);
      final container = ProviderContainer(overrides: relay.overrides);
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      await tester.pumpAndSettle();
      final event = relay.sign(
        kind: 9,
        content: 'link',
        tags: [
          ['h', 'team'],
        ],
        createdAt: now.millisecondsSinceEpoch ~/ 1000 + drift,
      );
      final result = expectLater(
        relay.session.publish(event),
        drift.abs() > 900 ? throwsA(isA<RelayEventRejected>()) : completes,
      );
      await tester.pumpAndSettle();
      await result;
      expect(
        relay.stored.any((stored) => stored.id == event.id),
        drift.abs() <= 900,
      );
    });
  }

  testWidgets(
    'a rejected stale share is re-signed; uncertain retry keeps its exact ID',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final container = ProviderContainer(overrides: relay.overrides);
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      await tester.pumpAndSettle();
      final sync = MeSync(
        relay.session,
        RelayConfig(baseUrl: 'https://relay.test', nsec: relay.keys.nsec),
        prefs: prefs,
      );
      final original = MeChatShare.create(
        sync,
        source: 'solo',
        name: 'Chat',
        destination: 'team',
        destinationName: 'team',
      );
      await expectLater(
        original.complete(
          sync,
          prepare: (_) async {},
          send: (_) async => throw TimeoutException('offline'),
        ),
        throwsA(isA<TimeoutException>()),
      );
      final key = prefs.getKeys().singleWhere(
        (key) => key.startsWith('me-chat-share:'),
      );
      final data =
          jsonDecode(nip44Decrypt(sync.key, prefs.getString(key)!))
              as Map<String, dynamic>;
      final old = NostrEvent.fromJson(
        signNostrEvent(
          secretKey: relay.keys.secret,
          kind: 9,
          content: original.message.content,
          tags: original.message.tags,
          createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 901,
        ).toMap(),
      );
      data['message'] = old.toJson();
      // Old builds stored recipients; new builds retain only the link, never grants.
      data['audience'] = {for (var i = 0; i < 102; i++) '$i': 'member'};
      await prefs.setString(key, nip44Encrypt(sync.key, jsonEncode(data)));
      final stale = MeChatShare.pending(sync, 'solo')!;
      final rejected = expectLater(
        stale.complete(
          sync,
          prepare: (_) async {},
          send: (event) async {
            await relay.session.publish(event);
          },
        ),
        throwsA(isA<RelayEventRejected>()),
      );
      await tester.pumpAndSettle();
      await rejected;
      final fresh = MeChatShare.pending(sync, 'solo')!;
      expect(fresh.message.id, isNot(old.id));
      expect(fresh.message.content, old.content);
      expect(fresh.destination, stale.destination);
      expect(
        (jsonDecode(nip44Decrypt(sync.key, prefs.getString(key)!)) as Map)
            .containsKey('audience'),
        isFalse,
      );
      await expectLater(stale.discard(sync), throwsStateError);
      final hold = relay.holdOk();
      final interrupted = fresh.complete(
        sync,
        prepare: (_) async {},
        send: (event) async {
          await relay.session.publish(event);
        },
      );
      await tester.pumpAndSettle();
      expect(MeChatShare.pending(sync, 'solo')!.message.id, fresh.message.id);
      hold.complete();
      await tester.pumpAndSettle();
      await interrupted;
      expect(MeChatShare.pending(sync, 'solo'), isNull);
      expect(
        relay.stored.where((e) => e.kind == 9).single.id,
        fresh.message.id,
      );
      // The rejected old link was never stored and is still outside the window.
      final accepted = expectLater(
        relay.session.publish(old),
        throwsA(isA<RelayEventRejected>()),
      );
      await tester.pumpAndSettle();
      await accepted;
    },
  );

  testWidgets('a stored duplicate is accepted outside the timestamp window', (
    tester,
  ) async {
    var now = DateTime(2026, 11, 4);
    final relay = FakeRelay(now: () => now);
    final container = ProviderContainer(overrides: relay.overrides);
    addTearDown(container.dispose);
    container.read(relaySessionProvider);
    await tester.pumpAndSettle();
    final event = relay.sign(
      kind: 9,
      content: 'stored link',
      tags: [
        ['h', 'team'],
      ],
      createdAt: now.millisecondsSinceEpoch ~/ 1000,
    );
    final first = relay.session.publish(event);
    await tester.pumpAndSettle();
    await first;
    now = now.add(const Duration(minutes: 16));
    final duplicate = relay.session.publish(event);
    await tester.pumpAndSettle();
    await duplicate;
    expect(relay.stored.where((stored) => stored.id == event.id), hasLength(1));
  });

  test(
    'cancelling an in-flight share stops its remaining publications',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final sync = MeSync(
        relay.session,
        RelayConfig(baseUrl: 'https://relay.test', nsec: relay.keys.nsec),
        prefs: prefs,
      );
      final intent = MeChatShare.create(
        sync,
        source: 'solo',
        name: 'Chat',
        destination: 'team',
        destinationName: 'team',
      );
      final started = Completer<void>(), release = Completer<void>();
      var posts = 0;
      final attempt = expectLater(
        intent.complete(
          sync,
          prepare: (_) async {
            started.complete();
            await release.future;
          },
          send: (_) async {
            posts++;
          },
        ),
        throwsStateError,
      );
      await started.future;
      await intent.discard(sync);
      release.complete();
      await attempt;
      expect(posts, 0);
      expect(MeChatShare.pending(sync, 'solo'), isNull);
    },
  );
}
