import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'dart:async';
import 'dart:convert';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../helpers/hosted_communities_fixture.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));
  for (final backup in [true, false]) {
    test(
      'hosted pairing requires backup=$backup through real coordinator',
      () async {
        if (!HostedEnvironment.requiresAuth('paired.communities.buzz.xyz')) {
          return;
        }
        final f = HostedCommunitiesFixture();
        await f.initialize(backup: backup);
        addTearDown(f.dispose);
        var legacyProbes = 0;
        final notifier = PairingNotifier(
          credentialValidator: ({required relayUrl, required nsec}) async {
            legacyProbes++;
          },
        );
        final container = ProviderContainer(
          overrides: [
            hostedFlowProvider.overrideWithValue(f.flow),
            pairingProvider.overrideWith(() => notifier),
          ],
        );
        addTearDown(container.dispose);
        final settled = Completer<void>();
        f.flow.addListener(() {
          if ((f.flow.stage == HostedFlowStage.complete ||
                  f.flow.stage == HostedFlowStage.blocked) &&
              !settled.isCompleted) {
            settled.complete();
          }
        });
        await container.read(pairingProvider.notifier).pair(_payload(f.keys));
        await settled.future.timeout(const Duration(seconds: 2));
        expect(legacyProbes, 0);
        expect(f.admitted, backup ? ['paired'] : isEmpty);
        expect((await f.store.read())?.pubkey, f.keys.public);
      },
    );
  }
  test(
    'verified paired destination survives restart with identity constraint',
    () async {
      final f = HostedCommunitiesFixture();
      await f.initialize();
      addTearDown(f.dispose);
      await f.flow.stagePairedCommunity(
        Community.create(
          name: 'Paired',
          relayUrl: 'https://paired.communities.buzz.xyz',
          pubkey: f.keys.public,
          nsec: f.keys.nsec,
          sensitiveActionPolicy: SensitiveActionPolicy.enabled,
        ),
        isCurrent: () => true,
      );
      final browser = DiscoveryBrowser();
      final admitted = <String>[];
      final resumed = HostedFlow(
        account: f.account,
        browser: browser,
        store: f.store,
        identity: f.flow.identity,
        enterCommunity: (link, subject, pubkey, current) async {
          if (current()) admitted.add(pubkey);
        },
      );
      addTearDown(() async {
        resumed.dispose();
        await browser.dispose();
      });
      await resumed.initialize();
      expect(resumed.stage, HostedFlowStage.signIn);
      expect(
        resumed.pairedSensitiveActionPolicy,
        SensitiveActionPolicy.enabled,
      );
      await resumed.retry();
      expect(admitted, [f.keys.public]);
      expect(resumed.stage, HostedFlowStage.complete);
    },
  );

  test(
    'hosted pairing retains transfer but blocks conflicting account admission',
    () async {
      if (!HostedEnvironment.requiresAuth('paired.communities.buzz.xyz')) {
        return;
      }
      final f = HostedCommunitiesFixture();
      await f.initialize();
      addTearDown(f.dispose);
      final container = ProviderContainer(
        overrides: [hostedFlowProvider.overrideWithValue(f.flow)],
      );
      addTearDown(container.dispose);
      final transferred = nostr.Keys('2'.padLeft(64, '0'));
      final settled = Completer<void>();
      f.flow.addListener(() {
        if ((f.flow.stage == HostedFlowStage.blocked ||
                f.flow.stage == HostedFlowStage.complete) &&
            !settled.isCompleted) {
          settled.complete();
        }
      });
      await container
          .read(pairingProvider.notifier)
          .pair(_payload(transferred));
      await settled.future.timeout(const Duration(seconds: 2));
      expect(f.flow.stage, HostedFlowStage.blocked);
      expect(f.flow.problem, HostedFlowProblem.identityMismatch);
      expect(
        (await f.store.forPairing(transferred.public).read())?.pubkey,
        transferred.public,
      );
      expect((await f.store.read())?.pubkey, f.keys.public);
      expect(f.admitted, isEmpty);
    },
  );
}

String _payload(nostr.Keys keys) => base64Url.encode(
  utf8.encode(
    jsonEncode({
      'relayUrl': 'https://paired.communities.buzz.xyz',
      'nsec': keys.nsec,
      'pubkey': keys.public,
    }),
  ),
);
