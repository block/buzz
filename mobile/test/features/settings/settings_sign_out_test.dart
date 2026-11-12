import 'dart:async';

import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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
      final accountStorage = _AccountStorage();
      if (HostedEnvironment.enabled) {
        hosted = HostedCommunitiesFixture();
        // A later launch whose session restore hasn't succeeded yet: the
        // session is saved but not in memory.
        await hosted.initialize(
          accountStorage: accountStorage,
          coldAccount: true,
        );
        addTearDown(hosted.dispose);
        await hosted.flow.leave();
        hosted.unavailable = true;
        await expectLater(hosted.account.restore(), throwsA(anything));
        expect(hosted.account.account, isNull);
      }
      // Hosted setup must never cover Settings while it signs out.
      final pushes = _Pushes();
      // The Builderlab session must be gone before the Sign in landing can
      // show it or be interrupted by its end.
      final sessionAtRemoval = <bool>[];
      final removed = <String>[];
      // Revocations stay in flight until the landing has been checked.
      final revocations = Completer<void>();
      addTearDown(() {
        if (!revocations.isCompleted) revocations.complete();
      });

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
              sessionAtRemoval.add(hosted?.account.account != null);
              return true;
            }),
            communityPushLeaseRevocationTriggerProvider.overrideWithValue(
              () => revocations.future,
            ),
            relaySessionProvider.overrideWith(_OfflineSession.new),
            if (hosted != null)
              hostedFlowProvider.overrideWithValue(hosted.flow),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            navigatorObservers: [pushes],
            home: const _Root(),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open Settings'));
      await tester.pumpAndSettle();

      // Cancelling keeps everything.
      await tester.tap(find.byKey(const Key('settings-sign-out')));
      await tester.pumpAndSettle();
      expect(find.text('Sign out?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(await storage.loadAll(), hasLength(2));

      Future<void> confirmSignOut() async {
        await tester.tap(find.byKey(const Key('settings-sign-out')));
        await tester.pumpAndSettle();
        pushes.count = 0;
        await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      }

      // Each failure keeps every community and shows its error in Settings.
      Future<void> signOutFails() async {
        await confirmSignOut();
        await tester.pumpAndSettle();
        expect(pushes.count, 0);
        expect(find.byType(SettingsPage), findsOneWidget);
        expect(find.textContaining('Failed to sign out'), findsOneWidget);
        expect(await storage.loadAll(), hasLength(2));
        expect(removed, isEmpty);
        ScaffoldMessenger.of(
          tester.element(find.byKey(const Key('settings-sign-out'))),
        ).removeCurrentSnackBar();
        await tester.pumpAndSettle();
      }

      if (hosted != null) {
        // The saved session can't be deleted, though it isn't restored yet.
        accountStorage.failing = true;
        await signOutFails();
        hosted.unavailable = false;
        expect(await hosted.account.restore(), isNotNull);
        // Nor once it is.
        await signOutFails();
        expect(hosted.account.account, isNotNull);
        accountStorage.failing = false;
        // A pending browser sign-in can't be forgotten.
        hosted.browser.clearIntentError = PlatformException(code: 'keychain');
        await signOutFails();
        expect(hosted.account.account, isNotNull);
        hosted.browser.clearIntentError = null;
      }

      await confirmSignOut();
      // The landing animates, so it never settles.
      await _pumpFor(tester);

      expect(pushes.count, 0);
      expect(await storage.loadAll(), isEmpty);
      expect(find.byType(SettingsPage), findsNothing);
      expect(find.byType(OnboardingAppEntry), findsOneWidget);
      expect(find.textContaining('Signed in as'), findsNothing);
      // The active community goes last, so nothing switches to the other.
      expect(removed, [other.id, active.id]);
      final saved = await AccountIdentityStore().identities();
      expect(
        saved.map((identity) => identity.pubkey),
        containsAll([activeKeys.public, otherKeys.public]),
      );
      if (hosted != null) {
        expect(sessionAtRemoval, everyElement(isFalse));
        expect(hosted.account.account, isNull);
        expect(await hosted.account.restore(), isNull);
      }
      // Revocations finishing later change nothing on the landing.
      revocations.complete();
      await _pumpFor(tester);
      expect(find.byType(OnboardingAppEntry), findsOneWidget);
      expect(find.textContaining('Signed in as'), findsNothing);
      expect(hosted?.account.account, isNull);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      await _pumpFor(tester);
    },
  );
}

Future<void> _pumpFor(WidgetTester tester) async {
  for (var i = 0; i < 20; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

/// Fails to delete the Builderlab session while [failing], as an unavailable
/// Keychain would.
class _AccountStorage extends FlutterSecureStorage {
  bool failing = false;

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) {
    if (failing && key.startsWith('buzz_hosted_session_')) {
      throw PlatformException(code: 'keychain', message: 'unavailable');
    }
    return super.delete(key: key);
  }
}

/// Swaps in the Sign in landing once the device is signed out, as the app does.
class _Root extends ConsumerWidget {
  const _Root();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = ref.watch(authProvider).value?.status;
    if (status == null) return const SizedBox.shrink();
    if (status != AuthStatus.authenticated) return const OnboardingAppEntry();
    return HostedAppGate(
      authenticated: true,
      child: Scaffold(
        body: TextButton(
          onPressed: () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => SettingsPage(
                profileHeader: const SizedBox.shrink(),
                identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
              ),
            ),
          ),
          child: const Text('Open Settings'),
        ),
      ),
    );
  }
}

/// The relay boundary: nothing connects.
class _OfflineSession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

/// Counts routes pushed since [count] was last reset.
class _Pushes extends NavigatorObserver {
  int count = 0;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) => count++;
}
