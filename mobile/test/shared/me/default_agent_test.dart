import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/default_agent_writer.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'me_sync_test.dart' show FakeRelay;

void main() {
  const secret =
      '0000000000000000000000000000000000000000000000000000000000000001';
  const origin = 'https://relay.test';
  final public = nostr.Keys(secret).public;
  final config = RelayConfig(
    baseUrl: origin,
    nsec: nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: secret),
  );
  NostrEvent signed(
    String? choice,
    int time, {
    String coordinate = 'main-agent',
    bool missing = false,
  }) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: 30078,
      content: nip44Encrypt(
        getConversationKey(secret, public),
        jsonEncode({'version': 1, if (!missing) 'agent': choice}),
      ),
      secretKey: secret,
      createdAt: time,
      tags: [
        ['d', coordinate],
        ['t', 'main-agent'],
      ],
      verify: false,
    ).toMap(),
  );

  test('writes the desktop main-agent wire format, including clear', () async {
    final relay = FakeRelay();
    final sync = MeSync(relay, config);
    for (final choice in ['a' * 64, null]) {
      relay.events = [];
      await saveDefaultAgent(sync, choice);
      final event = relay.events.single;
      expect(event.kind, 30078);
      expect(event.pubkey, public);
      expect(event.tags, [
        ['d', 'main-agent'],
        ['t', 'main-agent'],
      ]);
      expect(jsonDecode(nip44Decrypt(sync.key, event.content)), {
        'version': 1,
        'agent': choice,
      });
    }
  });

  test(
    'only signed owner preferences at the desktop coordinate and exact schema are accepted',
    () {
      final sync = MeSync(FakeRelay(), config);
      final valid = signed('a' * 64, 1);
      expect(decodeDefaultAgent(valid, sync), 'a' * 64);
      expect(decodeDefaultAgent(signed(null, 2), sync), isNull);
      for (final invalid in [
        signed('a' * 64, 1, coordinate: 'buzz-default-agent-v1:legacy'),
        signed(null, 1, missing: true),
        signed('invalid', 1),
        NostrEvent.fromJson({...valid.toJson(), 'sig': '0' * 128}),
        NostrEvent.fromJson({...valid.toJson(), 'pubkey': 'b' * 64}),
      ]) {
        expect(() => decodeDefaultAgent(invalid, sync), throwsA(anything));
      }
    },
  );

  test(
    'live choice wins over late history, persists offline, and can be cleared',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final history = Completer<List<NostrEvent>>();
      final relay = FakeRelay()..pendingRead = history.future;
      final sync = MeSync(relay, config, prefs: prefs);
      final container = ProviderContainer(
        overrides: [
          meSyncProvider.overrideWithValue(sync),
          relaySessionProvider.overrideWith(() => relay),
        ],
      );
      addTearDown(container.dispose);
      final seen = <String?>[];
      container.listen(defaultAgentProvider, (_, next) {
        if (next.hasValue) seen.add(next.value);
      });
      await Future<void>.delayed(Duration.zero);
      relay.emit!(signed('b' * 64, 2));
      history.complete([signed('a' * 64, 1)]);
      await container.read(defaultAgentProvider.future);
      await Future<void>.delayed(Duration.zero);
      expect(container.read(defaultAgentProvider).value, 'b' * 64);
      expect(seen, isNot(contains('a' * 64)));
      expect(prefs.getString('main-agent:$origin:$public'), isNotNull);
      relay.emit!(signed(null, 3));
      await Future<void>.delayed(Duration.zero);
      expect(container.read(defaultAgentProvider).value, isNull);
      expect(relay.writes, 0);
    },
  );
}
