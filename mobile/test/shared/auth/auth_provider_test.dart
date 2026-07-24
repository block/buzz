import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/push_bridge.dart';

import '../community/community_storage_test.dart';

void main() {
  test(
    'removes an invalid saved community instead of authenticating',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final invalid = Community.create(
        name: 'Invalid',
        relayUrl: 'https://relay.example',
        nsec: 'not-an-nsec',
      );
      await storage.save(invalid);
      await storage.saveActiveId(invalid.id);
      final snapshots = <List<Community>>[];
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((
            communities,
          ) async {
            snapshots.add(List.of(communities));
          }),
        ],
      );
      addTearDown(container.dispose);

      final auth = await container.read(authProvider.future);

      expect(auth.status, AuthStatus.unauthenticated);
      expect(await storage.loadAll(), isEmpty);
      expect(await storage.loadActiveId(), isNull);
      expect(snapshots.last, isEmpty);
    },
  );

  test('authenticate exports the complete stored community snapshot', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final existing = Community.create(
      name: 'Existing',
      relayUrl: 'https://existing.example',
      nsec: nostr.Keys.generate().nsec,
    );
    final added = Community.create(
      name: 'Added',
      relayUrl: 'https://added.example',
      nsec: nostr.Keys.generate().nsec,
    );
    await storage.save(existing);
    final snapshots = <List<Community>>[];
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((communities) async {
          snapshots.add(List.of(communities));
        }),
      ],
    );
    addTearDown(container.dispose);

    await container
        .read(authProvider.notifier)
        .authenticateWithCommunity(added);

    expect(snapshots.last.map((community) => community.id), {
      existing.id,
      added.id,
    });
  });

  test(
    'sign out removes the active community from the shared snapshot',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final first = Community.create(
        name: 'First',
        relayUrl: 'https://first.example',
        nsec: nostr.Keys.generate().nsec,
      );
      final second = Community.create(
        name: 'Second',
        relayUrl: 'https://second.example',
        nsec: nostr.Keys.generate().nsec,
      );
      await storage.save(first);
      await storage.save(second);
      await storage.saveActiveId(first.id);
      final snapshots = <List<Community>>[];
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((
            communities,
          ) async {
            snapshots.add(List.of(communities));
          }),
        ],
      );
      addTearDown(container.dispose);
      await container.read(authProvider.future);

      await container.read(authProvider.notifier).signOut();

      expect(
        snapshots.any((snapshot) {
          return snapshot.length == 1 && snapshot.single.id == second.id;
        }),
        isTrue,
      );
      expect(
        snapshots.last.map((community) => community.id),
        isNot(contains(first.id)),
      );
    },
  );

  test(
    'snapshot export failure does not gate startup authentication',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final community = Community.create(
        name: 'Existing',
        relayUrl: 'https://existing.example',
        nsec: nostr.Keys.generate().nsec,
      );
      await storage.save(community);
      await storage.saveActiveId(community.id);
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {
            throw PlatformException(
              code: 'save_failed',
              message: 'Keychain unavailable',
            );
          }),
        ],
      );
      addTearDown(container.dispose);

      final auth = await container.read(authProvider.future);

      expect(auth.status, AuthStatus.authenticated);
      expect(auth.community?.id, community.id);
      expect(pushCommunitySnapshotError.value, contains('save_failed'));
    },
  );

  test('snapshot export failure does not gate direct authentication', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final community = Community.create(
      name: 'Added',
      relayUrl: 'https://added.example',
      nsec: nostr.Keys.generate().nsec,
    );
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        communitySnapshotWriterProvider.overrideWithValue((_) async {
          throw PlatformException(
            code: 'save_failed',
            message: 'Keychain unavailable',
          );
        }),
      ],
    );
    addTearDown(container.dispose);

    await container
        .read(authProvider.notifier)
        .authenticateWithCommunity(community);

    final auth = await container.read(authProvider.future);
    expect(auth.status, AuthStatus.authenticated);
    expect(auth.community?.id, community.id);
    expect((await storage.loadAll()).single.id, community.id);
  });

  test('falls through to the next valid saved community', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final invalid = Community.create(
      name: 'Invalid',
      relayUrl: 'https://invalid.example',
    );
    final valid = Community.create(
      name: 'Valid',
      relayUrl: 'https://valid.example',
      nsec: nostr.Keys.generate().nsec,
    );
    await storage.save(invalid);
    await storage.save(valid);
    await storage.saveActiveId(invalid.id);
    final container = ProviderContainer(
      overrides: [communityStorageProvider.overrideWithValue(storage)],
    );
    addTearDown(container.dispose);

    final auth = await container.read(authProvider.future);

    expect(auth.status, AuthStatus.authenticated);
    expect(auth.community?.id, valid.id);
    expect(await storage.loadActiveId(), valid.id);
  });
}
