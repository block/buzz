import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'community_storage_test.dart';

void main() {
  test(
    'reserving a push lease does not notify relay config listeners',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final community = Community.create(
        name: 'Tom Town',
        relayUrl: 'https://relay.example.com',
      ).copyWith(pushNotificationsEnabled: true);
      await storage.save(community);
      await storage.saveActiveId(community.id);
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        ],
      );
      addTearDown(container.dispose);
      await container.read(activeCommunityProvider.future);
      final changes = <RelayConfig>[];
      container.listen(relayConfigProvider, (_, next) => changes.add(next));
      await container
          .read(communityListProvider.notifier)
          .reservePushLeaseGeneration(community.id);
      await container.read(activeCommunityProvider.future);
      await container.pump();
      expect(
        changes,
        isEmpty,
        reason: 'Push metadata must not reset the connected relay session',
      );
    },
  );
  for (final change in ['community', 'relay', 'identity']) {
    test('$change change notifies relay config listeners', () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final original = Community.create(
        name: 'Original',
        relayUrl: 'https://relay.example.com',
        nsec: 'first',
      );
      await storage.save(original);
      await storage.saveActiveId(original.id);
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        ],
      );
      addTearDown(container.dispose);
      await container.read(activeCommunityProvider.future);
      final changes = <RelayConfig>[];
      container.listen(relayConfigProvider, (_, next) => changes.add(next));
      final updated = switch (change) {
        'community' => Community.create(
          name: 'Other',
          relayUrl: original.relayUrl,
          nsec: original.nsec,
        ),
        'relay' => original.copyWith(relayUrl: 'https://other.example.com'),
        _ => original.copyWith(nsec: 'second'),
      };
      await storage.save(updated);
      await storage.saveActiveId(updated.id);
      container.invalidate(communityListProvider);
      await container.read(activeCommunityProvider.future);
      await container.pump();
      expect(changes, hasLength(1));
      expect(changes.single.baseUrl, updated.relayUrl);
      expect(changes.single.nsec, updated.nsec);
    });
  }
}
