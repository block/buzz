import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';

import '../community/community_storage_test.dart';

void main() {
  test('waits for transition teardown before replacing credentials', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final active = Community.create(
      name: 'Active',
      relayUrl: 'https://active.example',
      nsec: nostr.Keys.generate().nsec,
    );
    final replacement = Community.create(
      name: 'Replacement',
      relayUrl: 'https://replacement.example',
      nsec: nostr.Keys.generate().nsec,
    );
    await storage.save(active);
    await storage.saveActiveId(active.id);
    final container = ProviderContainer(
      overrides: [communityStorageProvider.overrideWithValue(storage)],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final teardown = Completer<void>();
    container.read(communityTransitionProvider).register(() => teardown.future);

    final authenticating = container
        .read(authProvider.notifier)
        .authenticateWithCommunity(replacement);
    await Future<void>.delayed(Duration.zero);

    expect(await storage.loadActiveId(), active.id);
    expect(
      (await storage.loadAll()).map((community) => community.id),
      isNot(contains(replacement.id)),
    );
    teardown.complete();
    await authenticating;
    expect(await storage.loadActiveId(), replacement.id);
    expect(
      (await storage.loadAll()).map((community) => community.id),
      contains(replacement.id),
    );
  });

  test('waits for transition teardown before removing credentials', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final community = Community.create(
      name: 'Active',
      relayUrl: 'https://relay.example',
      nsec: nostr.Keys.generate().nsec,
    );
    await storage.save(community);
    await storage.saveActiveId(community.id);
    final container = ProviderContainer(
      overrides: [communityStorageProvider.overrideWithValue(storage)],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final teardown = Completer<void>();
    container.read(communityTransitionProvider).register(() => teardown.future);

    final signingOut = container.read(authProvider.notifier).signOut();
    await Future<void>.delayed(Duration.zero);

    expect(await storage.loadActiveId(), community.id);
    final saved = await storage.loadAll();
    expect(saved, hasLength(1));
    expect(saved.single.id, community.id);
    teardown.complete();
    await signingOut;
    expect(await storage.loadActiveId(), isNull);
    expect(await storage.loadAll(), isEmpty);
  });

  test('serializes overlapping switch and sign-out mutations', () async {
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final active = Community.create(
      name: 'Active',
      relayUrl: 'https://active.example',
      nsec: nostr.Keys.generate().nsec,
    );
    final replacement = Community.create(
      name: 'Replacement',
      relayUrl: 'https://replacement.example',
      nsec: nostr.Keys.generate().nsec,
    );
    await storage.save(active);
    await storage.save(replacement);
    await storage.saveActiveId(active.id);
    final container = ProviderContainer(
      overrides: [communityStorageProvider.overrideWithValue(storage)],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    await container.read(communityListProvider.future);
    final teardown = Completer<void>();
    container.read(communityTransitionProvider).register(() => teardown.future);

    final switching = container
        .read(communityListProvider.notifier)
        .switchCommunity(replacement.id);
    final signingOut = container.read(authProvider.notifier).signOut();
    await Future<void>.delayed(Duration.zero);

    expect(await storage.loadActiveId(), active.id);
    teardown.complete();
    await Future.wait([switching, signingOut]);

    expect(await storage.loadActiveId(), active.id);
    expect(
      (await storage.loadAll()).map((community) => community.id),
      unorderedEquals([active.id]),
    );
    final auth = await container.read(authProvider.future);
    expect(auth.status, AuthStatus.authenticated);
    expect(auth.community?.id, active.id);
  });

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
