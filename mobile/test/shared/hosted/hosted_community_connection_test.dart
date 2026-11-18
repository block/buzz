import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/hosted_admission.dart';
import 'package:buzz/shared/hosted/hosted_community_connection.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/hosted_communities_fixture.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  for (final conflicting in [false, true]) {
    test(
      'real admission preserves saved :443 record (conflict=$conflicting)',
      () async {
        final f = HostedCommunitiesFixture();
        await f.initialize();
        addTearDown(f.dispose);
        final storage = CommunityStorage();
        final savedKeys = conflicting
            ? nostr.Keys('2'.padLeft(64, '0'))
            : f.keys;
        final existing = Community.create(
          name: 'Saved name',
          relayUrl: 'wss://one.communities.buzz.xyz:443/',
          pubkey: savedKeys.public,
          nsec: savedKeys.nsec,
        );
        await storage.save(existing);
        final unrelated = Community.create(
          name: 'Other',
          relayUrl: 'wss://other.example',
          pubkey: savedKeys.public,
          nsec: savedKeys.nsec,
        );
        await storage.save(unrelated);
        var probes = 0;
        final admission = HostedAdmission(
          account: f.account,
          relayAuth: HostedRelayAuth(f.account, enabled: false),
          store: f.store,
          probe: (_) async {
            probes++;
          },
        );
        addTearDown(admission.dispose);
        final operation = connectHostedCommunity(
          link: const AddCommunityDeepLink(
            relayUrl: 'wss://one.communities.buzz.xyz',
            name: 'one',
          ),
          subject: 'owner',
          pubkey: f.keys.public,
          isCurrent: () => true,
          storage: storage,
          admission: admission,
          recoverStarterChannels: (_, _) async =>
              fail('Saved non-invite community needs no starter repair'),
          persist: (community, current) async {
            if (current()) await storage.save(community);
          },
        );
        if (conflicting) {
          await expectLater(
            operation,
            throwsA(isA<SavedCommunityIdentityConflict>()),
          );
          expect(probes, 0);
        } else {
          await operation;
          expect(probes, 1);
        }
        final after = await storage.loadAll();
        expect(after.length, 2);
        expect(after.first.id, existing.id);
        expect(after.first.name, existing.name);
        expect(after.first.nsec, savedKeys.nsec);
        expect(after.last.toJson(), unrelated.toJson());
      },
    );
  }
}
