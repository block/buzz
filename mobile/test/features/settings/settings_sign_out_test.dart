import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/hosted_communities_fixture.dart';

/// Runs in every build. A public build also signs out of Builderlab:
/// `--dart-define=BUZZ_DISTRIBUTION=public`
/// `--dart-define=BUZZ_BUILDERLAB_URL=https://app.builderlab.xyz`
/// `--dart-define=BUZZ_BUILDERLAB_RELAYS=communities.buzz.xyz`
void main() {
  testWidgets(
    'Sign out removes every community but keeps each key on this device',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 1600);
      addTearDown(tester.view.reset);
      FlutterSecureStorage.setMockInitialValues({});
      SharedPreferences.setMockInitialValues({});
      PackageInfo.setMockInitialValues(
        appName: 'Buzz',
        packageName: 'xyz.block.buzz',
        version: '0.16.0',
        buildNumber: '1',
        buildSignature: '',
      );
      final prefs = await SharedPreferences.getInstance();
      // Neither key is exported or backed up anywhere.
      final activeKeys = nostr.Keys.generate();
      final otherKeys = nostr.Keys.generate();
      final active = Community.create(
        name: 'Active',
        relayUrl: 'wss://active.example.org',
        nsec: activeKeys.nsec,
      );
      final other = Community.create(
        name: 'Other',
        relayUrl: 'wss://other.example.org',
        nsec: otherKeys.nsec,
      );
      final storage = CommunityStorage();
      await storage.save(active);
      await storage.save(other);
      await storage.saveActiveId(active.id);

      HostedCommunitiesFixture? hosted;
      if (HostedEnvironment.enabled) {
        hosted = HostedCommunitiesFixture();
        await hosted.initialize();
        addTearDown(hosted.dispose);
        await hosted.flow.leave();
        expect(hosted.account.account, isNotNull);
      }
      // Revocations must run while the Builderlab session can still
      // authenticate them on a hosted relay.
      final sessionAtRevocation = <bool>[];
      final removed = <String>[];

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            communityPushLeaseRevocationEnqueuerProvider.overrideWithValue((
              community,
            ) async {
              removed.add(community.id);
              return true;
            }),
            communityPushLeaseRevocationTriggerProvider.overrideWithValue(
              () async {
                // Ending the session without waiting would land in here.
                await Future<void>.delayed(const Duration(seconds: 1));
                sessionAtRevocation.add(hosted?.account.account != null);
              },
            ),
            if (hosted != null)
              hostedFlowProvider.overrideWithValue(hosted.flow),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: SettingsPage(
              profileHeader: const SizedBox.shrink(),
              identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      // Cancelling keeps everything.
      await tester.tap(find.byKey(const Key('settings-sign-out')));
      await tester.pumpAndSettle();
      expect(find.text('Sign out?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(await storage.loadAll(), hasLength(2));

      await tester.tap(find.byKey(const Key('settings-sign-out')));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await tester.pumpAndSettle();
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();

      expect(await storage.loadAll(), isEmpty);
      // The active community goes last, so nothing switches to the other.
      expect(removed, [other.id, active.id]);
      final saved = await AccountIdentityStore().identities();
      expect(
        saved.map((identity) => identity.pubkey),
        containsAll([activeKeys.public, otherKeys.public]),
      );
      if (hosted != null) {
        expect(sessionAtRevocation, isNotEmpty);
        expect(sessionAtRevocation, everyElement(isTrue));
        expect(hosted.account.account, isNull);
        expect(await hosted.account.restore(), isNull);
      }
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
    },
  );
}
