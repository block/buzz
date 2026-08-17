import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'package:buzz/shared/relay/relay.dart';

void main() {
  const relaySecret =
      '0000000000000000000000000000000000000000000000000000000000000003';
  final relayPubkey = nostr.Keys(relaySecret).public;
  const archivedPubkey =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

  NostrEvent snapshot({
    int kind = 13535,
    List<List<String>>? tags,
    String? secretKey,
  }) {
    final event = nostr.Event.from(
      kind: kind,
      content: '',
      tags:
          tags ??
          const [
            ['-'],
            ['p', archivedPubkey],
          ],
      secretKey: secretKey ?? relaySecret,
      createdAt: 1_700_000_000,
    );
    return NostrEvent.fromJson(event.toMap());
  }

  test('reads a valid NIP-11 relay self key', () {
    expect(relaySelfPubkeyFromDocument('{"self":"$relayPubkey"}'), relayPubkey);
    expect(relaySelfPubkeyFromDocument('{"self":"not-a-key"}'), isNull);
    expect(relaySelfPubkeyFromDocument('not-json'), isNull);
  });

  test('accepts a signed kind:13535 snapshot from NIP-11 self', () {
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(),
        relaySelfPubkey: relayPubkey,
      ),
      {archivedPubkey},
    );
  });

  test('rejects snapshots with the wrong kind, author, or NIP-70 tag', () {
    final otherSecret =
        '0000000000000000000000000000000000000000000000000000000000000004';
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(kind: 13534),
        relaySelfPubkey: relayPubkey,
      ),
      isNull,
    );
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(secretKey: otherSecret),
        relaySelfPubkey: relayPubkey,
      ),
      isNull,
    );
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(
          tags: const [
            ['p', archivedPubkey],
          ],
        ),
        relaySelfPubkey: relayPubkey,
      ),
      isNull,
    );
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(
          tags: const [
            ['-'],
            ['-'],
            ['p', archivedPubkey],
          ],
        ),
        relaySelfPubkey: relayPubkey,
      ),
      isNull,
    );
  });

  test('ignores malformed p tags without invalidating a trusted snapshot', () {
    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: snapshot(
          tags: const [
            ['-'],
            ['p'],
            ['p', 'not-a-key'],
            ['p', archivedPubkey, 'ignored-metadata'],
          ],
        ),
        relaySelfPubkey: relayPubkey,
      ),
      {archivedPubkey},
    );
  });

  test('rejects a snapshot whose signed event data was altered', () {
    final signed = snapshot();
    final altered = NostrEvent(
      id: signed.id,
      pubkey: signed.pubkey,
      createdAt: signed.createdAt,
      kind: signed.kind,
      tags: const [
        ['-'],
        ['p', archivedPubkey],
        [
          'p',
          'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        ],
      ],
      content: signed.content,
      sig: signed.sig,
    );

    expect(
      archivedIdentityPubkeysFromSnapshot(
        event: altered,
        relaySelfPubkey: relayPubkey,
      ),
      isNull,
    );
  });

  test('provider binds the snapshot query to NIP-11 self', () async {
    final relaySession = _ArchiveFakeRelaySession(events: [snapshot()]);
    final httpClient = MockClient((request) async {
      expect(request.url, Uri.parse('https://relay.example'));
      expect(request.headers['accept'], 'application/nostr+json');
      return http.Response('{"self":"$relayPubkey"}', 200);
    });
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        identityArchiveHttpClientProvider.overrideWithValue(httpClient),
      ],
    );
    addTearDown(container.dispose);
    addTearDown(httpClient.close);
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://relay.example', nsec: null);

    final archived = await container.read(
      relayArchivedIdentityPubkeysProvider.future,
    );

    expect(archived, {archivedPubkey});
    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.kinds, [13535]);
    expect(relaySession.filters.single.authors, [relayPubkey]);
  });

  test('provider fails open when the relay snapshot is untrusted', () async {
    final signed = snapshot();
    final altered = NostrEvent(
      id: signed.id,
      pubkey: signed.pubkey,
      createdAt: signed.createdAt,
      kind: signed.kind,
      tags: const [
        ['-'],
        ['p', archivedPubkey],
        [
          'p',
          'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        ],
      ],
      content: signed.content,
      sig: signed.sig,
    );
    final relaySession = _ArchiveFakeRelaySession(events: [altered]);
    final httpClient = MockClient(
      (_) async => http.Response('{"self":"$relayPubkey"}', 200),
    );
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        identityArchiveHttpClientProvider.overrideWithValue(httpClient),
      ],
    );
    addTearDown(container.dispose);
    addTearDown(httpClient.close);
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://relay.example', nsec: null);

    expect(
      await container.read(relayArchivedIdentityPubkeysProvider.future),
      isEmpty,
    );
  });
}

class _ArchiveFakeRelaySession extends RelaySessionNotifier {
  _ArchiveFakeRelaySession({required this.events});

  final List<NostrEvent> events;
  List<NostrFilter> filters = const [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    this.filters = filters;
    return events;
  }
}
