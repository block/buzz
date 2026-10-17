import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
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
      const intent = MeChatCreation('same-id', 'Private name', 'agent');
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
