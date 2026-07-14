import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';

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
      final container = ProviderContainer(
        overrides: [communityStorageProvider.overrideWithValue(storage)],
      );
      addTearDown(container.dispose);

      final auth = await container.read(authProvider.future);

      expect(auth.status, AuthStatus.unauthenticated);
      expect(await storage.loadAll(), isEmpty);
      expect(await storage.loadActiveId(), isNull);
    },
  );

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
