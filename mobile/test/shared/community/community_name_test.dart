import 'dart:async';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_name.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'community_storage_test.dart';

void main() {
  final community = Community(
    id: 'one',
    name: 'buzz',
    relayUrl: 'wss://buzz.example.com',
    addedAt: DateTime(2026),
  );
  test('canonical name persists and previous label survives fallback', () {
    final named = reconcileCommunityName(
      community,
      const CommunityProfile('ATL BitLab'),
    );
    expect(named.name, 'ATL BitLab');
    expect(named.fallbackName, 'buzz');
    expect(Community.fromJson(named.toJson()).name, 'ATL BitLab');
    expect(
      reconcileCommunityName(named, const CommunityProfile(null)).name,
      'buzz',
    );
    final fresh = Community.create(
      name: 'Invite label',
      relayUrl: community.relayUrl,
    );
    expect(
      reconcileCommunityName(fresh, const CommunityProfile('Shared')).name,
      'Shared',
    );
    final legacy = reconcileCommunityName(
      community.copyWith(name: 'My label'),
      const CommunityProfile('Shared'),
    );
    expect(legacy.name, 'My label');
    expect(legacy.localName, 'My label');
    final nicknamed = named.copyWith(localName: 'Mine');
    expect(
      reconcileCommunityName(nicknamed, const CommunityProfile('New')).name,
      'Mine',
    );
    expect(
      Community.nameFromUrl('ws://100.64.1.2:3000'),
      'Community (100.64.1.2)',
    );
  });
  test(
    'read path distinguishes legacy, unnamed, malformed, and network errors',
    () async {
      var body = '{"community_profile":{"name":"ATL BitLab"}}';
      var status = 200;
      final container = ProviderContainer(
        overrides: [
          communityIconHttpClientProvider.overrideWithValue(
            MockClient((request) async {
              expect(request.url.toString(), 'https://buzz.example.com');
              expect(request.headers['Accept'], 'application/nostr+json');
              return http.Response(body, status);
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      final fetch = container.read(communityProfileFetcherProvider);
      expect((await fetch(community.relayUrl))?.name, 'ATL BitLab');
      body = '{"name":"Buzz Relay"}';
      expect(await fetch(community.relayUrl), isNull);
      body = '{"community_profile":{"name":null}}';
      expect(await fetch(community.relayUrl), isA<CommunityProfile>());
      body = '{"community_profile":{"name":32}}';
      await expectLater(fetch(community.relayUrl), throwsFormatException);
      status = 503;
      await expectLater(fetch(community.relayUrl), throwsStateError);
    },
  );
  test(
    'refresh caches names, survives offline, and drops superseded reads',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      await storage.save(community);
      var response = Future<CommunityProfile?>.value(
        const CommunityProfile('Shared'),
      );
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          communityProfileFetcherProvider.overrideWithValue((_) => response),
        ],
      );
      addTearDown(container.dispose);
      await container.read(communityListProvider.future);
      final notifier = container.read(communityListProvider.notifier);
      await notifier.refreshCommunityNames();
      expect((await storage.loadAll()).single.name, 'Shared');
      response = Future.error(StateError('offline'));
      await notifier.refreshCommunityNames();
      expect(
        container.read(communityListProvider).value!.single.name,
        'Shared',
      );
      final stale = Completer<CommunityProfile?>();
      response = stale.future;
      final first = notifier.refreshCommunityNames();
      response = Future.value(const CommunityProfile('Newest'));
      await notifier.refreshCommunityNames();
      stale.complete(const CommunityProfile('Stale'));
      await first;
      expect((await storage.loadAll()).single.name, 'Newest');
      // A completing public read must preserve a concurrent nickname and
      // credentials change, including the persisted snapshot.
      final pending = Completer<CommunityProfile?>();
      response = pending.future;
      final refresh = notifier.refreshCommunityNames();
      await notifier.renameCommunity(community.id, 'My nickname');
      await notifier.addCommunity(community.copyWith(pubkey: 'new-identity'));
      pending.complete(const CommunityProfile('Renamed again'));
      await refresh;
      final stored = (await storage.loadAll()).single;
      expect(stored.name, 'My nickname');
      expect(stored.canonicalName, 'Renamed again');
      expect(stored.pubkey, 'new-identity');
    },
  );
}
