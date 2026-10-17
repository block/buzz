import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  for (final useAuth in [true, false]) {
    for (final host in [
      'team.communities.buzz.xyz',
      'team.communities.public.builderlabstaging.xyz',
      'team.communities.public.builderlabstaging.xyz.',
    ]) {
      test(
        'unsigned hosted entry cannot persist through '
        '${useAuth ? "authentication" : "community addition"}: $host',
        () async {
          FlutterSecureStorage.setMockInitialValues({});
          final storage = CommunityStorage(secure: FakeSecureStorage());
          final container = ProviderContainer(
            overrides: [communityStorageProvider.overrideWithValue(storage)],
          );
          addTearDown(container.dispose);
          await container.read(authProvider.future);
          await container.read(communityListProvider.future);
          var transitions = 0;
          container.read(communityTransitionProvider).register(() async {
            transitions++;
          });
          final keys = nostr.Keys.generate();
          final community = Community.create(
            name: 'Hosted',
            relayUrl: 'wss://$host',
            pubkey: keys.public,
            nsec: keys.nsec,
          );
          final pending = useAuth
              ? container
                    .read(authProvider.notifier)
                    .authenticateWithCommunity(community)
              : container
                    .read(communityListProvider.notifier)
                    .addCommunity(community);
          await expectLater(pending, throwsA(isA<AccountFailure>()));
          expect(await storage.loadAll(), isEmpty);
          expect(await storage.loadActiveId(), isNull);
          expect(transitions, 0);
          expect(
            container.read(authProvider).value!.status,
            AuthStatus.unauthenticated,
          );
        },
      );
    }
  }
}
