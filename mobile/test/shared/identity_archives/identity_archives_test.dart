import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/identity_archives/identity_archives.dart';
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
      var createdAt = 1700000000;
      NostrEvent snapshot(nostr.Keys signer, {bool marker = true}) =>
          NostrEvent.fromJson(
            nostr.Event.from(
              kind: 13535,
              content: '',
              secretKey: signer.secret,
              createdAt: createdAt++,
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
          relayInformationClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      expect(await container.read(identityArchivesProvider.future), {target});
      expect(relay.author, authority.public);
      for (final invalid in <NostrEvent Function()>[
        () => snapshot(impostor),
        () => snapshot(authority, marker: false),
        () => NostrEvent.fromJson(
          snapshot(authority).toJson()..['sig'] = '0' * 128,
        ),
      ]) {
        // Each case starts from a warm cache, so it must also drop the known
        // list: once the relay has nothing newer, nobody stays hidden.
        relay.event = snapshot(authority);
        container.invalidate(identityArchivesProvider);
        expect(await container.read(identityArchivesProvider.future), {target});
        relay.event = invalid();
        container.invalidate(identityArchivesProvider);
        expect(await container.read(identityArchivesProvider.future), isEmpty);
        relay.event = null;
        container.invalidate(identityArchivesProvider);
        expect(await container.read(identityArchivesProvider.future), isEmpty);
      }
      relay.event = snapshot(authority);
      container.invalidate(identityArchivesProvider);
      expect(await container.read(identityArchivesProvider.future), {target});
    },
  );

  test(
    'hides archived identities for the current community only, never the viewer',
    () async {
      final viewer = nostr.Keys.generate();
      final other = nostr.Keys.generate();
      final authority = nostr.Keys.generate();
      final archivedInA = 'a' * 64;
      final archivedInB = 'b' * 64;
      NostrEvent snapshot(List<String> keys, {int? createdAt}) =>
          NostrEvent.fromJson(
            nostr.Event.from(
              kind: 13535,
              content: '',
              secretKey: authority.secret,
              createdAt: createdAt,
              tags: [
                ['-'],
                for (final key in keys) ['p', key],
              ],
            ).toMap(),
          );
      // Community A's list also names the viewer, who must stay visible.
      final relay = _ArchiveRelay(snapshot([archivedInA, viewer.public]));
      final communityB = Completer<void>();
      final client = MockClient((request) async {
        if (request.url.host == 'b.test') await communityB.future;
        return http.Response(jsonEncode({'self': authority.public}), 200);
      });
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(
            () => _Config(
              RelayConfig(baseUrl: 'https://a.test', nsec: viewer.nsec),
            ),
          ),
          meSyncProvider.overrideWith(
            (ref) => MeSync(relay, ref.watch(relayConfigProvider)),
          ),
          relayInformationClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      var hiddenChanges = 0;
      container.listen(hiddenArchivedIdentitiesProvider, (_, _) {
        hiddenChanges++;
      });
      Future<Set<String>> settled() async {
        await container.read(identityArchivesProvider.future);
        await pumpEventQueue();
        return container.read(hiddenArchivedIdentitiesProvider);
      }

      expect(await settled(), {archivedInA});
      // A refresh in the same community keeps the known set meanwhile.
      container.invalidate(identityArchivesProvider);
      expect(container.read(hiddenArchivedIdentitiesProvider), {archivedInA});
      expect(await settled(), {archivedInA});
      // An unchanged list leaves the hidden set alone for every watcher.
      relay.event = snapshot([
        archivedInA,
      ], createdAt: relay.event!.createdAt + 10);
      container.invalidate(identityArchivesProvider);
      expect(await settled(), {archivedInA});
      final changes = hiddenChanges;
      container.invalidate(identityArchivesProvider);
      expect(await settled(), {archivedInA});
      expect(hiddenChanges, changes);

      // Switch community and account: A's list no longer applies, so nobody
      // is hidden until B's list arrives.
      relay.event = snapshot([archivedInB]);
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://b.test', nsec: other.nsec);
      await pumpEventQueue();
      expect(container.read(hiddenArchivedIdentitiesProvider), isEmpty);
      communityB.complete();
      expect(await settled(), {archivedInB});
    },
  );

  test('a refresh downloads the list again only when it changed', () async {
    final owner = nostr.Keys.generate();
    final authority = nostr.Keys.generate();
    NostrEvent snapshot(String target, int createdAt) => NostrEvent.fromJson(
      nostr.Event.from(
        kind: 13535,
        content: '',
        secretKey: authority.secret,
        createdAt: createdAt,
        tags: [
          ['-'],
          ['p', target],
        ],
      ).toMap(),
    );
    final relay = _ArchiveRelay(snapshot('a' * 64, 1700000000));
    final client = MockClient(
      (_) async => http.Response(jsonEncode({'self': authority.public}), 200),
    );
    final container = ProviderContainer(
      overrides: [
        meSyncProvider.overrideWithValue(
          MeSync(
            relay,
            RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
          ),
        ),
        relayInformationClientProvider.overrideWithValue(client),
      ],
    );
    addTearDown(container.dispose);
    addTearDown(client.close);
    expect(await container.read(identityArchivesProvider.future), {'a' * 64});
    expect(relay.since, isNull);

    // Unchanged: the relay has nothing newer, and the known list stands.
    container.invalidate(identityArchivesProvider);
    expect(await container.read(identityArchivesProvider.future), {'a' * 64});
    expect(relay.since, 1700000001);

    // Replaced: the newer list wins and becomes the new baseline.
    relay.event = snapshot('b' * 64, 1700000100);
    container.invalidate(identityArchivesProvider);
    expect(await container.read(identityArchivesProvider.future), {'b' * 64});
    container.invalidate(identityArchivesProvider);
    expect(await container.read(identityArchivesProvider.future), {'b' * 64});
    expect(relay.since, 1700000101);
  });

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
        relayInformationClientProvider.overrideWithValue(client),
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
        .read(identityArchivesProvider.future)
        .timeout(const Duration(seconds: 5));
    expect(result, isEmpty);
    expect(cancelled, isTrue);
    expect(relay.author, isNull);
  });

  test('a refused metadata response is aborted, not left unread', () async {
    final owner = nostr.Keys.generate();
    final relay = _ArchiveRelay(null);
    Future<void>? aborted;
    final client = MockClient.streaming((request, _) async {
      aborted = (request as http.Abortable).abortTrigger;
      return http.StreamedResponse(StreamController<List<int>>().stream, 503);
    });
    final container = ProviderContainer(
      overrides: [
        meSyncProvider.overrideWithValue(
          MeSync(
            relay,
            RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
          ),
        ),
        relayInformationClientProvider.overrideWithValue(client),
      ],
    );
    addTearDown(container.dispose);
    addTearDown(client.close);
    expect(await container.read(identityArchivesProvider.future), isEmpty);
    await aborted!.timeout(const Duration(seconds: 1));
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
          relayInformationClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      addTearDown(client.close);
      expect(await container.read(identityArchivesProvider.future), isEmpty);
      expect(relay.author, isNull);
    },
  );
}

class _ArchiveRelay extends RelaySessionNotifier {
  _ArchiveRelay(this.event);
  NostrEvent? event;
  String? author;
  int? since;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filter.kinds, [13535]);
    author = filter.authors!.single;
    since = filter.since;
    final event = this.event;
    if (event == null || event.createdAt < (filter.since ?? 0)) return [];
    return [event];
  }
}

class _Config extends RelayConfigNotifier {
  _Config(this.initial);
  final RelayConfig initial;
  @override
  RelayConfig build() => initial;
}
