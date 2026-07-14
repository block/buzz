import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';

import 'community_storage_test.dart';

void main() {
  late FakeSecureStorage fakeSecure;
  late CommunityStorage communityStorage;
  late ProviderContainer container;

  setUp(() {
    fakeSecure = FakeSecureStorage();
    communityStorage = CommunityStorage(secure: fakeSecure);
  });

  tearDown(() => container.dispose());

  ProviderContainer createContainer() {
    return ProviderContainer(
      overrides: [communityStorageProvider.overrideWithValue(communityStorage)],
    );
  }

  group('CommunityListNotifier', () {
    test('loads empty list initially', () async {
      container = createContainer();
      final communities = await container.read(communityListProvider.future);
      expect(communities, isEmpty);
    });

    test('addCommunity adds to list', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await container.read(communityListProvider.notifier).addCommunity(ws);

      final communities = await container.read(communityListProvider.future);
      expect(communities, hasLength(1));
      expect(communities.first.name, 'Test');
    });

    test('removeCommunity removes from list', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      await container.read(communityListProvider.notifier).addCommunity(ws);
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(ws.id);

      final communities = await container.read(communityListProvider.future);
      expect(communities, isEmpty);
    });

    test('renameCommunity updates name', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws = Community.create(
        name: 'Original',
        relayUrl: 'https://test.example.com',
      );
      await container.read(communityListProvider.notifier).addCommunity(ws);
      await container
          .read(communityListProvider.notifier)
          .renameCommunity(ws.id, 'Renamed');

      final communities = await container.read(communityListProvider.future);
      expect(communities.first.name, 'Renamed');
    });

    test('switchCommunity updates active ID', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws1 = Community.create(
        name: 'One',
        relayUrl: 'https://one.example.com',
      );
      final ws2 = Community.create(
        name: 'Two',
        relayUrl: 'https://two.example.com',
      );

      final notifier = container.read(communityListProvider.notifier);
      await notifier.addCommunity(ws1);
      await notifier.addCommunity(ws2);
      await notifier.switchCommunity(ws2.id);

      final activeId = await communityStorage.loadActiveId();
      expect(activeId, ws2.id);
    });
  });

  group('activeCommunityProvider', () {
    test('returns null when no communities', () async {
      container = createContainer();
      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNull);
    });

    test('returns community matching active ID', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws = Community.create(
        name: 'Test',
        relayUrl: 'https://test.example.com',
      );
      final notifier = container.read(communityListProvider.notifier);
      await notifier.addCommunity(ws);
      await notifier.switchCommunity(ws.id);

      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
      expect(active.name, 'Test');
    });

    test('falls back to first community if active ID is invalid', () async {
      container = createContainer();
      await container.read(communityListProvider.future);

      final ws = Community.create(
        name: 'Fallback',
        relayUrl: 'https://test.example.com',
      );
      final notifier = container.read(communityListProvider.notifier);
      await notifier.addCommunity(ws);

      // Set an invalid active ID.
      await communityStorage.saveActiveId('nonexistent-id');

      // Re-read — should fall back.
      container.invalidate(activeCommunityProvider);
      final active = await container.read(activeCommunityProvider.future);
      expect(active, isNotNull);
      expect(active!.id, ws.id);
    });
  });
}
