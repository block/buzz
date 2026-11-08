import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/known_communities_store.dart';
import 'package:buzz/shared/hosted/known_communities_sync.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/hosted_journey_fixture.dart';

void main() {
  testWidgets('the account list joins silently and a refused leave is parked', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final f = HostedJourneyFixture();
    await tester.runAsync(
      () => http.runWithClient(() async {
        try {
          await f.initialize();
          f.rows.addAll([f.row('one'), f.row('two')]);
          final home = Community.create(
            name: 'one',
            relayUrl: 'wss://one.communities.buzz.xyz',
            pubkey: f.keys.public,
            nsec: f.keys.nsec,
          );
          await f.storage.save(home);
          await f.storage.saveActiveId(home.id);
          await f.mount(tester, openEntry: false);
          final sync = f.container.listen(
            knownCommunitiesSyncProvider,
            (_, _) {},
          );
          addTearDown(sync.close);
          await f.settle(tester);

          // `two`, saved on another device, is joined with this account's
          // key through the production admission and persistence path.
          Future<List<Community>> saved() async {
            await f.settle(tester);
            return f.storage.loadAll();
          }

          final two = (await saved())
              .where((item) => item.relayUrl.contains('two.'))
              .single;
          expect(two.pubkey, f.keys.public);
          expect(f.probes.map((item) => item.relayUrl), contains(two.relayUrl));
          // Silently: the active community, its route and the profile stay.
          expect(await f.storage.loadActiveId(), home.id);
          expect(f.container.read(authProvider).value?.community?.id, home.id);
          expect(find.text('Community home'), findsOneWidget);
          expect(f.profileWrites, isEmpty);
          // `one` was already on the account's list, so nothing goes up.
          expect(f.sent('/known-communities/add'), isEmpty);

          // A leave the service refuses in its body is parked, not retried.
          f.removeError = 'invalid_request';
          await f.container
              .read(communityListProvider.notifier)
              .removeCommunity(two.id);
          await saved();
          expect(f.removedKnown, ['wss://two.communities.buzz.xyz']);
          f.container.invalidate(communityListProvider);
          await saved();
          expect(f.removedKnown, hasLength(1));
          expect(
            await f.container.read(knownCommunitiesStoreProvider).pending(),
            [
              (
                url: 'wss://two.communities.buzz.xyz',
                removed: true,
                pubkey: f.keys.public,
              ),
            ],
          );
        } finally {
          await f.dispose(tester);
        }
      }, f.client),
    );
  }, skip: HostedEnvironment.authDomains.isEmpty);
}
