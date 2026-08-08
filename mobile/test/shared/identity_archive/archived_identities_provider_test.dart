import 'package:buzz/shared/identity_archive/archived_identities_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('stays loading until the relay session connects', () async {
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(_DisconnectedRelaySession.new),
      ],
    );
    addTearDown(container.dispose);

    final subscription = container.listen(
      archivedIdentityPubkeysProvider,
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(subscription.close);

    expect(subscription.read(), const AsyncLoading<Set<String>>());
  });

  test('accepts only valid pubkeys from a relay-signed archive snapshot', () {
    final relay = nostr.Keys.generate();
    final archived = 'a' * 64;
    final uppercase = 'B' * 64;
    final event = nostr.Event.from(
      kind: EventKind.archivedIdentities,
      content: '',
      tags: [
        ['-'],
        ['p', archived],
        ['p', uppercase],
        ['p', 'not-a-pubkey'],
      ],
      secretKey: relay.secret,
      verify: false,
    );

    expect(
      archivedPubkeysFromSnapshot(
        NostrEvent.fromJson(event.toMap()),
        relay.public,
      ),
      {archived, uppercase.toLowerCase()},
    );
  });

  test('rejects snapshots without exactly one valid NIP-70 tag', () {
    final relay = nostr.Keys.generate();
    final archived = 'a' * 64;

    NostrEvent snapshotWith(List<List<String>> tags) {
      final event = nostr.Event.from(
        kind: EventKind.archivedIdentities,
        content: '',
        tags: tags,
        secretKey: relay.secret,
        verify: false,
      );
      return NostrEvent.fromJson(event.toMap());
    }

    expect(
      archivedPubkeysFromSnapshot(
        snapshotWith([
          ['p', archived],
        ]),
        relay.public,
      ),
      isEmpty,
    );
    expect(
      archivedPubkeysFromSnapshot(
        snapshotWith([
          ['-', 'malformed'],
          ['p', archived],
        ]),
        relay.public,
      ),
      isEmpty,
    );
    expect(
      archivedPubkeysFromSnapshot(
        snapshotWith([
          ['-'],
          ['-'],
          ['p', archived],
        ]),
        relay.public,
      ),
      isEmpty,
    );
  });

  test('rejects a snapshot with a forged signature', () {
    final relay = nostr.Keys.generate();
    final attacker = nostr.Keys.generate();
    final signed = nostr.Event.from(
      kind: EventKind.archivedIdentities,
      content: '',
      tags: [
        ['-'],
        ['p', 'a' * 64],
      ],
      secretKey: attacker.secret,
      verify: false,
    );
    final forged = NostrEvent.fromJson({
      ...signed.toMap(),
      'pubkey': relay.public,
    });

    expect(archivedPubkeysFromSnapshot(forged, relay.public), isEmpty);
  });

  test(
    'loads the active relay archive snapshot using its NIP-11 identity',
    () async {
      final relay = nostr.Keys.generate();
      final archived = 'c' * 64;
      final signed = nostr.Event.from(
        kind: EventKind.archivedIdentities,
        content: '',
        tags: [
          ['-'],
          ['p', archived],
        ],
        secretKey: relay.secret,
        verify: false,
      );
      final relaySession = _ArchiveRelaySession([
        NostrEvent.fromJson(signed.toMap()),
      ]);
      late http.Request nip11Request;
      final container = ProviderContainer(
        overrides: [
          archivedIdentitiesHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              nip11Request = request;
              return http.Response('{"self":"${relay.public}"}', 200);
            }),
          ),
          relayConfigProvider.overrideWith(
            () => _FakeRelayConfigNotifier('wss://relay.example.com'),
          ),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      );
      addTearDown(container.dispose);

      final result = await container.read(
        archivedIdentityPubkeysProvider.future,
      );

      expect(nip11Request.url, Uri.parse('https://relay.example.com'));
      expect(nip11Request.headers['Accept'], 'application/nostr+json');
      expect(relaySession.filters.single.toJson(), {
        'kinds': [EventKind.archivedIdentities],
        'limit': 1,
        'authors': [relay.public],
      });
      expect(result, {archived});
    },
  );
}

class _ArchiveRelaySession extends RelaySessionNotifier {
  final List<NostrEvent> events;
  final List<NostrFilter> filters = [];

  _ArchiveRelaySession(this.events);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    this.filters.addAll(filters);
    return events;
  }
}

class _DisconnectedRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  final String baseUrl;

  _FakeRelayConfigNotifier(this.baseUrl);

  @override
  RelayConfig build() => RelayConfig(baseUrl: baseUrl);
}
