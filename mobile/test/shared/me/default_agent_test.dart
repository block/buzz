import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/default_agent.dart';
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
    String community = origin,
    bool missing = false,
  }) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: 30078,
      content: nip44Encrypt(
        getConversationKey(secret, public),
        jsonEncode({
          'version': 1,
          'community': community,
          'deleted': false,
          'value': {
            'type': 'default-agent',
            'id': 'me',
            if (!missing) 'pubkey': choice,
          },
        }),
      ),
      secretKey: secret,
      createdAt: time,
      tags: [
        ['d', defaultAgentCoordinate(origin)],
        ['t', defaultAgentTag],
      ],
      verify: false,
    ).toMap(),
  );

  test(
    'only signed owner preferences for this community and exact schema are accepted',
    () {
      final sync = MeSync(FakeRelay(), config);
      final valid = signed('a' * 64, 1);
      expect(decodeDefaultAgent(valid, sync), 'a' * 64);
      expect(decodeDefaultAgent(signed(null, 2), sync), isNull);
      for (final invalid in [
        signed('a' * 64, 1, community: 'https://other.test'),
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
      expect(prefs.getString('default-agent:$origin:$public'), isNotNull);
      relay.emit!(signed(null, 3));
      await Future<void>.delayed(Duration.zero);
      expect(container.read(defaultAgentProvider).value, isNull);
      expect(relay.writes, 0);
    },
  );
}
