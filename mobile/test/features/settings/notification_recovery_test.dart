import 'dart:async';

import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  for (final outcome in ['loading', 'absent', 'error', 'large text']) {
    final setup = outcome == 'large text'
        ? 'at a large text size'
        : 'with $outcome capability';
    testWidgets('opt-out failures recover $setup', (tester) async {
      final semantics = tester.ensureSemantics();
      if (outcome == 'large text') {
        // A small phone at a large text size: Retry must stay reachable and
        // leave the warning readable.
        tester.view.physicalSize = const Size(750, 3600);
        tester.view.devicePixelRatio = 2;
        tester.platformDispatcher.textScaleFactorTestValue = 2;
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      } else {
        tester.view.physicalSize = const Size(430, 1800);
        tester.view.devicePixelRatio = 1;
      }
      addTearDown(tester.view.reset);
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      PackageInfo.setMockInitialValues(
        appName: 'Buzz',
        packageName: 'buzz',
        version: '1.0.0',
        buildNumber: '1',
        buildSignature: '',
      );
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final subscriptions = [
        BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        ),
      ];
      final community =
          Community.create(
            name: 'Team',
            relayUrl: 'https://relay.example',
          ).copyWith(
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: subscriptions,
            ).withAccepted(subscriptions: subscriptions, generation: 7),
          );
      await storage.save(community);
      await storage.saveActiveId(community.id);
      final pending = Completer<BuzzPushLeaseDescriptor?>();
      final snapshots = <List<Community>>[];
      final tombstones = <int?>[];
      var relayAvailable = false;
      Completer<void>? hung;
      // A new ProviderScope over the same storage is a process restart.
      Future<void> launch() => tester.pumpWidget(
        ProviderScope(
          key: UniqueKey(),
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            authProvider.overrideWith(_Auth.new),
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((
              communities,
            ) async {
              snapshots.add(List.of(communities));
            }),
            communityPushLeaseDeactivatorProvider.overrideWithValue((
              community, {
              generation,
              deadline,
            }) async {
              tombstones.add(generation);
              if (hung != null) await hung.future;
              if (!relayAvailable) throw StateError('relay unavailable');
            }),
            currentRelayPushDescriptorProvider.overrideWith((ref) {
              if (outcome == 'loading') return pending.future;
              if (outcome == 'error') throw StateError('discovery failed');
              return Future.value(null);
            }),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
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
      await launch();
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(SettingsPage)),
      );
      expect(
        container.read(activeCommunityProvider).value?.pushNotificationsEnabled,
        isTrue,
        reason:
            '${container.read(activeCommunityProvider)} / ${container.read(communityListProvider)}',
      );
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      const warning = 'Could not confirm that notifications were turned off.';
      void expectWarning({required bool shown}) {
        expect(find.text(warning), shown ? findsOneWidget : findsNothing);
        expect(
          find.text('Notifications may continue.'),
          shown ? findsOneWidget : findsNothing,
        );
        expect(
          find.widgetWithText(TextButton, 'Retry'),
          shown ? findsOneWidget : findsNothing,
        );
        if (!shown) return;
        expect(
          tester.getSemantics(
            find.byKey(const ValueKey('push-opt-out-failed')),
          ),
          isSemantics(isLiveRegion: true),
        );
      }

      void expectSwitch({required bool on}) {
        final shownSwitch = tester.widget<Switch>(find.byType(Switch));
        expect(shownSwitch.value, on);
        expect(shownSwitch.onChanged, isNotNull);
      }

      // The relay refused the tombstone, so it never took effect: the switch
      // goes back to the confirmed On, with the warning and a Retry.
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      var stored = (await storage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isTrue);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
      expect(tombstones, [8]);
      expect(snapshots.last.single.pushNotificationsEnabled, isTrue);
      expectSwitch(on: true);
      expectWarning(shown: true);
      // A restart shows the saved On without the warning, and makes no
      // attempt of its own.
      await tester.pumpWidget(const SizedBox());
      await launch();
      await tester.pumpAndSettle();
      expectSwitch(on: true);
      expectWarning(shown: false);
      expect(tombstones, [8]);
      // An attempt that never answers is bounded: it shows as pending, with
      // the switch held, then fails at the deadline and is not retried.
      hung = Completer<void>();
      await tester.tap(find.byType(Switch));
      // Let the attempt start, then rebuild once more. No settle: the
      // deadline timer is still running.
      for (var i = 0; i < 5 && tombstones.length < 2; i++) {
        await tester.pump();
      }
      await tester.pump();
      expect(find.text('Turning off…'), findsOneWidget);
      final heldSwitch = tester.widget<Switch>(find.byType(Switch));
      expect(heldSwitch.value, isFalse);
      expect(heldSwitch.onChanged, isNull);
      expect(tombstones, [8, 9]);
      await tester.pump(pushOptOutTimeout + const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(find.text('Turning off…'), findsNothing);
      expect(tombstones, [8, 9]);
      // The relay may have stored it, so the opt-out stays saved as a guard,
      // while the switch shows the confirmed On and the warning.
      stored = (await storage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 9);
      expectSwitch(on: true);
      expectWarning(shown: true);
      // That warning survives a restart, which makes no attempt.
      await tester.pumpWidget(const SizedBox());
      await launch();
      await tester.pumpAndSettle();
      expectSwitch(on: true);
      expectWarning(shown: true);
      expect(tombstones, [8, 9]);
      hung = null;
      // Retry makes one explicit attempt at a new generation.
      relayAvailable = true;
      await tester.tap(find.widgetWithText(TextButton, 'Retry'));
      await tester.pumpAndSettle();
      expect(tombstones, [8, 9, 10]);
      stored = (await storage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
      expect(find.text('Notifications'), findsNothing);
      expect(tester.takeException(), isNull);
      semantics.dispose();
      debugDefaultTargetPlatformOverride = null;
    });
  }
  testWidgets('an accepted opt-out never shows the failure warning', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(430, 1800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    PackageInfo.setMockInitialValues(
      appName: 'Buzz',
      packageName: 'buzz',
      version: '1.0.0',
      buildNumber: '1',
      buildSignature: '',
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final secure = _SlowSecureStorage();
    final storage = CommunityStorage(secure: secure);
    final subscriptions = [
      BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
        notificationClass: 'default',
      ),
    ];
    final community =
        Community.create(
          name: 'Team',
          relayUrl: 'https://relay.example',
        ).copyWith(
          pushNotificationsEnabled: true,
          pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
            desired: subscriptions,
          ).withAccepted(subscriptions: subscriptions, generation: 7),
        );
    await storage.save(community);
    await storage.saveActiveId(community.id);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          authProvider.overrideWith(_Auth.new),
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          communityPushLeaseDeactivatorProvider.overrideWithValue(
            (community, {generation, deadline}) async {},
          ),
          currentRelayPushDescriptorProvider.overrideWith(
            (ref) => Future.value(null),
          ),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
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
    expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    // On a device the active community trails the list by a keychain read.
    secure.slow = true;
    await tester.tap(find.byType(Switch));
    var flashed = false;
    for (var i = 0; i < 200; i++) {
      await tester.pump(const Duration(milliseconds: 1));
      flashed |= find
          .text('Could not confirm that notifications were turned off.')
          .evaluate()
          .isNotEmpty;
    }
    expect(flashed, isFalse);
    await tester.pumpAndSettle();
    expect(find.text('Notifications'), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);
}

class _SlowSecureStorage extends FakeSecureStorage {
  bool slow = false;

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (slow) await Future<void>.delayed(const Duration(milliseconds: 2));
    return super.read(key: key);
  }
}
