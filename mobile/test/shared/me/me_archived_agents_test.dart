import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/me/me_archived_agents.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test(
    'only a verified snapshot from NIP-11 self hides archived agents',
    () async {
      final owner = nostr.Keys.generate();
      final authority = nostr.Keys.generate();
      final impostor = nostr.Keys.generate();
      final target = 'a' * 64;
      NostrEvent snapshot(nostr.Keys signer, {bool marker = true}) =>
          NostrEvent.fromJson(
            nostr.Event.from(
              kind: 13535,
              content: '',
              secretKey: signer.secret,
              tags: [
                if (marker) ['-'],
                ['p', target],
              ],
            ).toMap(),
          );
      final relay = _ArchiveRelay(snapshot(authority));
      final client = MockClient((request) async {
        expect(request.url.origin, 'https://relay.test');
        expect(request.headers['Accept'], 'application/nostr+json');
        return http.Response(
          jsonEncode({'self': authority.public, 'pubkey': impostor.public}),
          200,
        );
      });
      final container = ProviderContainer(
        overrides: [
          meSyncProvider.overrideWithValue(
            MeSync(
              relay,
              RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
            ),
          ),
          meRelayInformationClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      expect(await container.read(meArchivedAgentsProvider.future), {target});
      expect(relay.author, authority.public);
      for (final invalid in [
        snapshot(impostor),
        snapshot(authority, marker: false),
        NostrEvent.fromJson(snapshot(authority).toJson()..['sig'] = '0' * 128),
      ]) {
        relay.event = invalid;
        container.invalidate(meArchivedAgentsProvider);
        expect(await container.read(meArchivedAgentsProvider.future), isEmpty);
      }
      relay.event = snapshot(authority);
      container.invalidate(meArchivedAgentsProvider);
      expect(await container.read(meArchivedAgentsProvider.future), {target});
    },
  );

  test('slow metadata has a total deadline and cancels its stream', () async {
    final owner = nostr.Keys.generate();
    final relay = _ArchiveRelay(null);
    var cancelled = false;
    final body = StreamController<List<int>>(onCancel: () => cancelled = true);
    final client = MockClient.streaming(
      (_, _) async => http.StreamedResponse(body.stream, 200),
    );
    final container = ProviderContainer(
      overrides: [
        meSyncProvider.overrideWithValue(
          MeSync(
            relay,
            RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
          ),
        ),
        meRelayInformationClientProvider.overrideWithValue(client),
      ],
    );
    addTearDown(container.dispose);
    addTearDown(client.close);
    addTearDown(body.close);
    final trickle = Timer.periodic(const Duration(milliseconds: 100), (_) {
      body.add([32]);
    });
    addTearDown(trickle.cancel);
    final result = await container
        .read(meArchivedAgentsProvider.future)
        .timeout(const Duration(seconds: 5));
    expect(result, isEmpty);
    expect(cancelled, isTrue);
    expect(relay.author, isNull);
  });

  test(
    'contact keys and unavailable metadata are not archive authority',
    () async {
      final owner = nostr.Keys.generate();
      final relay = _ArchiveRelay(null);
      final client = MockClient(
        (_) async => http.Response(jsonEncode({'pubkey': owner.public}), 200),
      );
      final container = ProviderContainer(
        overrides: [
          meSyncProvider.overrideWithValue(
            MeSync(
              relay,
              RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
            ),
          ),
          meRelayInformationClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      expect(await container.read(meArchivedAgentsProvider.future), isEmpty);
      expect(relay.author, isNull);
    },
  );
}

class _ArchiveRelay extends RelaySessionNotifier {
  _ArchiveRelay(this.event);
  NostrEvent? event;
  String? author;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filter.kinds, [13535]);
    author = filter.authors!.single;
    return [?event];
  }
}
