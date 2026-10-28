import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
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
