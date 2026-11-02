import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/me/me_chat_title.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('first-message title follows desktop plain-text rules', () {
    for (final (message, title) in [
      ('  Plan\n  my trip  ', 'Plan my trip'),
      ('@Alice help me', '@Alice help me'),
      (
        'buzz://message?channel=source Plan the next step',
        'Plan the next step',
      ),
      ('buzz://message?channel=source', 'Chat'),
      ('### Heading', 'Heading'),
      ('🐝' * 90, '🐝' * 80),
    ]) {
      expect(startingMeChatTitle(message), title);
    }
  });
  test(
    'accepted first message survives a lost acknowledgement and restart',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final owner = nostr.Keys.generate();
      final agent = nostr.Keys.generate().public;
      final relay = _MessageRelay();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: owner.nsec,
      );
      MeSync service() => MeSync(relay, config, prefs: prefs);
      final message = NostrEvent.fromJson(
        signNostrEvent(
          kind: 9,
          content: 'Plan my trip',
          tags: [
            ['h', 'chat'],
            ['p', agent],
          ],
          secretKey: owner.secret,
        ).toMap(),
      );
      final intent = MeChatCreation(
        'chat',
        'Plan my trip',
        agent,
        initialMessage: message,
      );
      Future<String> send(MeChatCreation intent) => intent.complete(
        service(),
        findChannel: () async => 'chat',
        createChannel: () async => throw StateError('already created'),
        hasAgent: () async => true,
        addAgent: () async => throw StateError('already joined'),
        isPlaced: () async => true,
        place: () async => throw StateError('already placed'),
      );
      await expectLater(send(intent), throwsStateError);
      await prefs.reload();
      final pending = MeChatCreation.pending(service())!;
      expect(pending.initialMessage?.toJson(), message.toJson());
      expect(
        prefs.getString(MeChatCreation.storageKey(service())),
        isNot(contains('Plan my trip')),
      );
      await expectLater(
        MeChatCreation('chat', intent.name, agent).save(service()),
        throwsStateError,
      );
      expect(await send(pending), 'chat');
      expect(relay.attempts, [message.id, message.id]);
      expect(relay.messages.length, 1);
      expect(MeChatCreation.pending(service()), isNull);
    },
  );
  for (final accepted in [false, true]) {
    test(
      'expired first message resumes without duplication: accepted=$accepted',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final owner = nostr.Keys.generate();
        final relay = _MessageRelay();
        final sync = MeSync(
          relay,
          RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
          prefs: prefs,
        );
        final old = NostrEvent.fromJson(
          signNostrEvent(
            kind: 9,
            content: 'Yesterday’s draft',
            tags: [
              ['h', 'chat'],
              ['p', 'agent'],
            ],
            secretKey: owner.secret,
            createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 86400,
          ).toMap(),
        );
        if (accepted) relay.messages[old.id] = old;
        final intent = MeChatCreation(
          'chat',
          'Yesterday’s draft',
          'agent',
          initialMessage: old,
        );
        Future<String> run(MeChatCreation intent) => intent.complete(
          sync,
          findChannel: () async => 'chat',
          createChannel: () async => throw StateError('already created'),
          hasAgent: () async => true,
          addAgent: () async => throw StateError('already joined'),
          isPlaced: () async => true,
          place: () async => throw StateError('already placed'),
        );
        if (!accepted) {
          await expectLater(run(intent), throwsStateError);
          await prefs.reload();
          final pending = MeChatCreation.pending(sync)!;
          expect(pending.initialMessage!.id, isNot(old.id));
          expect(pending.initialMessage!.content, old.content);
          expect(pending.initialMessage!.pubkey, owner.public);
          expect(pending.initialMessage!.tags, old.tags);
          expect(await run(pending), 'chat');
          expect(relay.attempts.toSet(), {pending.initialMessage!.id});
        } else {
          expect(await run(intent), 'chat');
          expect(relay.attempts, isEmpty);
        }
        expect(relay.messages.length, 1);
        expect(MeChatCreation.pending(sync), isNull);
      },
    );
  }
  test(
    'retry confirms persistence after a failed cached journal write',
    () async {
      SharedPreferences.setMockInitialValues({});
      final store = _FailingStore();
      final previousStore = SharedPreferencesStorePlatform.instance;
      SharedPreferencesStorePlatform.instance = store;
      addTearDown(
        () => SharedPreferencesStorePlatform.instance = previousStore,
      );
      final prefs = await SharedPreferences.getInstance();
      final sync = MeSync(
        RelaySessionNotifier(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Keys.generate().nsec,
        ),
        prefs: prefs,
      );
      const intent = MeChatCreation('id', 'Name', 'agent');
      var reachedRelay = false;
      Future<String> run() => intent.complete<String>(
        sync,
        findChannel: () async {
          reachedRelay = true;
          await prefs.reload();
          expect(MeChatCreation.pending(sync)?.id, intent.id);
          return null;
        },
        createChannel: () async => throw StateError('Simulated process stop'),
        hasAgent: () async => false,
        addAgent: () async {},
        isPlaced: () async => false,
        place: () async {},
      );
      await expectLater(run(), throwsStateError);
      expect(reachedRelay, isFalse);
      expect(
        MeChatCreation.pending(sync)?.id,
        intent.id,
      ); // Failed write remains cached.
      await expectLater(run(), throwsStateError);
      expect(reachedRelay, isTrue);
      await prefs.reload();
      expect(MeChatCreation.pending(sync)?.id, intent.id);
    },
  );
  test('missing durable storage blocks every relay side effect', () async {
    final sync = MeSync(
      RelaySessionNotifier(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      ),
    );
    var touchedRelay = false;
    await expectLater(
      const MeChatCreation('id', 'Name', 'agent').complete<String>(
        sync,
        findChannel: () async {
          touchedRelay = true;
          return null;
        },
        createChannel: () async => 'id',
        hasAgent: () async => false,
        addAgent: () async {},
        isPlaced: () async => false,
        place: () async {},
      ),
      throwsStateError,
    );
    expect(touchedRelay, isFalse);
  });
  test(
    'discard preserves partial relay state and permits a different intent offline',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      );
      final online = MeSync(RelaySessionNotifier(), config, prefs: prefs);
      const intent = MeChatCreation('old', 'Old', 'revoked-agent');
      await intent.save(online);
      final offline = MeSync(
        RelaySessionNotifier(),
        config,
        prefs: prefs,
        connected: false,
      );
      await expectLater(
        const MeChatCreation('wrong', 'Old', 'revoked-agent').discard(offline),
        throwsStateError,
      );
      expect(MeChatCreation.pending(offline)?.id, 'old');
      await intent.discard(offline);
      expect(MeChatCreation.pending(offline), isNull);
      await const MeChatCreation('new', 'New', 'other-agent').save(online);
      expect(MeChatCreation.pending(online)?.id, 'new');
    },
  );
  for (final crashAfter in [0, 1, 2, 3]) {
    test('restart resumes the same chat after boundary $crashAfter', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      );
      MeSync service() => MeSync(RelaySessionNotifier(), config, prefs: prefs);
      const intent = MeChatCreation(
        'same-id',
        'Private name',
        'agent',
        project: 'mobile',
      );
      var created = false, member = false, placed = false, crash = true;
      final writes = [0, 0, 0];
      void boundary(int point) {
        expect(MeChatCreation.pending(service())?.id, intent.id);
        if (crash && crashAfter == point) throw StateError('process stopped');
      }

      Future<String> run(MeSync sync, MeChatCreation intent) =>
          intent.complete<String>(
            sync,
            findChannel: () async {
              boundary(0);
              return created ? intent.id : null;
            },
            createChannel: () async {
              created = true;
              writes[0]++;
              boundary(1);
              return intent.id;
            },
            hasAgent: () async => member,
            addAgent: () async {
              member = true;
              writes[1]++;
              boundary(2);
            },
            isPlaced: () async => placed,
            place: () async {
              placed = true;
              writes[2]++;
              boundary(3);
            },
          );
      await expectLater(run(service(), intent), throwsStateError);
      final restarted = service();
      final pending = MeChatCreation.pending(restarted)!;
      expect(pending.name, intent.name);
      expect(pending.agent, intent.agent);
      expect(pending.project, intent.project);
      await expectLater(
        MeChatCreation(
          intent.id,
          intent.name,
          intent.agent,
          project: 'other',
        ).save(restarted),
        throwsStateError,
      );
      expect(
        prefs.getString(MeChatCreation.storageKey(restarted)),
        isNot(contains(intent.name)),
      );
      crash = false;
      expect(await run(restarted, pending), intent.id);
      expect(writes, [1, 1, 1]);
      expect(MeChatCreation.pending(restarted), isNull);
      final other = MeSync(
        RelaySessionNotifier(),
        RelayConfig(baseUrl: 'https://other.test', nsec: config.nsec),
        prefs: prefs,
      );
      expect(MeChatCreation.pending(other), isNull);
    });
  }
}

class _FailingStore extends InMemorySharedPreferencesStore {
  _FailingStore() : super.empty();
  bool failNext = true;
  @override
  Future<bool> setValue(String valueType, String key, Object value) async {
    if (failNext && key.startsWith('flutter.me-chat-creation:')) {
      failNext = false;
      return false;
    }
    return super.setValue(valueType, key, value);
  }
}

class _MessageRelay extends RelaySessionNotifier {
  final messages = <String, NostrEvent>{};
  final attempts = <String>[];
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    for (final message in messages.values)
      if (filter.ids!.contains(message.id)) message,
  ];
  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    attempts.add(event.id);
    messages[event.id] = event;
    if (attempts.length == 1) throw StateError('Acknowledgement lost');
    return event;
  }
}
