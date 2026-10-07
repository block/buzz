import 'dart:async';

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

    test(
      'waits for transition teardown before removing active community',
      () async {
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
        await notifier.switchCommunity(ws1.id);
        final teardown = Completer<void>();
        container
            .read(communityTransitionProvider)
            .register(() => teardown.future);

        final removing = notifier.removeCommunity(ws1.id);
        await Future<void>.delayed(Duration.zero);

        expect(await communityStorage.loadActiveId(), ws1.id);
        expect(
          (await communityStorage.loadAll()).map((community) => community.id),
          contains(ws1.id),
        );
        teardown.complete();
        await removing;
        expect(await communityStorage.loadActiveId(), ws2.id);
        expect(
          (await communityStorage.loadAll()).map((community) => community.id),
          isNot(contains(ws1.id)),
        );
      },
    );

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

    test('waits for transition teardown before updating active ID', () async {
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
      await notifier.switchCommunity(ws1.id);
      final teardown = Completer<void>();
      container
          .read(communityTransitionProvider)
          .register(() => teardown.future);

      final switching = notifier.switchCommunity(ws2.id);
      await Future<void>.delayed(Duration.zero);

      expect(await communityStorage.loadActiveId(), ws1.id);
      teardown.complete();
      await switching;
      expect(await communityStorage.loadActiveId(), ws2.id);
    });

    test('overlapping transitions share one callback run', () async {
      container = createContainer();
      final coordinator = container.read(communityTransitionProvider);
      final teardown = Completer<void>();
      var calls = 0;
      coordinator.register(() {
        calls++;
        return teardown.future;
      });

      var firstCompleted = false;
      var secondCompleted = false;
      final first = coordinator.run().then((_) => firstCompleted = true);
      final second = coordinator.run().then((_) => secondCompleted = true);
      await Future<void>.delayed(Duration.zero);

      expect(calls, 1);
      expect(firstCompleted, isFalse);
      expect(secondCompleted, isFalse);
      teardown.complete();
      await Future.wait([first, second]);
      expect(firstCompleted, isTrue);
      expect(secondCompleted, isTrue);
    });

    test('a later transition starts a new callback run', () async {
      container = createContainer();
      final coordinator = container.read(communityTransitionProvider);
      var calls = 0;
      coordinator.register(() async => calls++);

      await coordinator.run();
      await coordinator.run();

      expect(calls, 2);
    });

    test(
      'transition callback failure does not block the active ID update',
      () async {
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
        await notifier.switchCommunity(ws1.id);
        container.read(communityTransitionProvider).register(() async {
          throw StateError('native cleanup failed');
        });

        await notifier.switchCommunity(ws2.id);

        expect(await communityStorage.loadActiveId(), ws2.id);
      },
    );

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
