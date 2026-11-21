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
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  for (final outcome in ['loading', 'absent', 'error']) {
    testWidgets('enabled community can turn push off with $outcome capability', (
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
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      final stored = (await storage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 8);
      expect(tombstones, [8]);
      expect(snapshots.last.single.pushNotificationsEnabled, isFalse);
      // A rejected opt-out ends in a failure, not an endless "Turning off":
      // the switch shows the last confirmed setting, with a Retry.
      void expectFailedOptOut() {
        final shownSwitch = tester.widget<Switch>(find.byType(Switch));
        expect(shownSwitch.value, isTrue);
        expect(shownSwitch.onChanged, isNotNull);
        expect(find.text("Couldn't turn off notifications"), findsOneWidget);
        expect(find.text('They may continue.'), findsOneWidget);
        expect(find.widgetWithText(TextButton, 'Retry'), findsOneWidget);
        expect(find.widgetWithText(TextButton, 'Keep on'), findsOneWidget);
      }

      expectFailedOptOut();
      // A restart keeps the failure and makes no attempt of its own.
      await tester.pumpWidget(const SizedBox());
      await launch();
      await tester.pumpAndSettle();
      expectFailedOptOut();
      expect(tombstones, [8]);
      // Keep on abandons the opt-out and turns push back on.
      await tester.tap(find.widgetWithText(TextButton, 'Keep on'));
      await tester.pumpAndSettle();
      expect((await storage.loadAll()).single.pushNotificationsEnabled, isTrue);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text("Couldn't turn off notifications"), findsNothing);
      expect(tombstones, [8]);
      // An attempt that never answers is bounded: it shows as pending, then
      // as a failure, and is not retried.
      hung = Completer<void>();
      await tester.tap(find.byType(Switch));
      // Let the attempt start, then rebuild once more. No settle: the
      // timeout timer is still running.
      for (var i = 0; i < 5 && tombstones.length < 2; i++) {
        await tester.pump();
      }
      await tester.pump();
      expect(find.text('Turning off…'), findsOneWidget);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      final attempted = List.of(tombstones);
      expect(attempted, hasLength(2));
      await tester.pump(pushOptOutTimeout);
      await tester.pumpAndSettle();
      expectFailedOptOut();
      expect(find.text('Turning off…'), findsNothing);
      expect(tombstones, attempted);
      hung = null;
      // Retry makes one explicit attempt at a new generation.
      relayAvailable = true;
      await tester.tap(find.widgetWithText(TextButton, 'Retry'));
      await tester.pumpAndSettle();
      expect(tombstones, [...attempted, attempted.last! + 1]);
      expect(
        (await storage.loadAll())
            .single
            .pushSubscriptionState
            .pendingTombstoneGeneration,
        isNull,
      );
      expect(find.text('Notifications'), findsNothing);
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    });
  }
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
