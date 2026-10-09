import 'dart:async';

import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final fonts = FontLoader('Inter')
      ..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'));
    await fonts.load();
  });
  for (final outcome in ['accepted', 'failed', 'pending', 'scaled']) {
    testWidgets('off then on remains usable with $outcome cleanup', (
      tester,
    ) async {
      final semantics = tester.ensureSemantics();
      // iPhone 12 mini width; use a wider screen for the 2x single-line case.
      tester.view.physicalSize = Size(outcome == 'scaled' ? 430 : 375, 1800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      if (outcome == 'scaled') {
        tester.platformDispatcher.textScaleFactorTestValue = 2;
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      }
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
      var discoveries = 0;
      final community =
          Community.create(
            name: 'Team',
            relayUrl: 'https://relay.example',
          ).copyWith(
            nsec: nostr.Nip19.encode(
              prefix: nostr.Nip19Prefix.nsec,
              data: '1' * 64,
            ),
            pushNotificationsEnabled: true,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: subscriptions,
            ).withAccepted(subscriptions: subscriptions, generation: 7),
          );
      await storage.save(community);
      await storage.saveActiveId(community.id);
      final pending = Completer<BuzzPushLeaseDescriptor>();
      final cleanup = Completer<void>();
      final showingSettings = ValueNotifier(true);
      addTearDown(showingSettings.dispose);
      final snapshots = <List<Community>>[];
      final tombstones = <int?>[];
      await tester.pumpWidget(
        ProviderScope(
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
              if (outcome == 'failed') throw StateError('relay unavailable');
              if (outcome == 'pending') await cleanup.future;
            }),
            relaySessionProvider.overrideWith(_Connected.new),
            myPubkeyProvider.overrideWithValue('a' * 64),
            buzzPushDescriptorFetcherProvider.overrideWithValue((_) async {
              discoveries++;
              if (discoveries > 1) {
                return pending.future;
              }
              return _descriptor;
            }),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
            buzzPushAuthorizationStatusReaderProvider.overrideWithValue(
              () async => BuzzPushAuthorizationStatus.denied,
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: ValueListenableBuilder(
              valueListenable: showingSettings,
              builder: (_, showing, _) => showing
                  ? SettingsPage(
                      profileHeader: const SizedBox.shrink(),
                      identityRecoveryPageBuilder: (_) =>
                          const SizedBox.shrink(),
                    )
                  : const SizedBox.shrink(),
            ),
          ),
        ),
      );
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
      // Bootstrap also observes capability while Settings is closed.
      final capabilitySubscription = container.listen(
        currentRelayPushDescriptorProvider,
        (_, _) {},
      );
      addTearDown(capabilitySubscription.close);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(
        tester.getSemantics(
          find.byKey(const ValueKey('push-notifications-setting')),
        ),
        isSemantics(
          label: 'Notifications',
          hasToggledState: true,
          isToggled: true,
        ),
      );
      expect(
        tester.getSize(find.byType(Switch)).height,
        greaterThanOrEqualTo(48),
      );
      // Compare rendered row bounds, not just their padding parameters.
      final notificationHeight = tester
          .getSize(find.byKey(const ValueKey('push-notifications-enabled')))
          .height;
      final statusHeight = tester
          .getSize(find.byKey(const ValueKey('settings-set-status')))
          .height;
      expect(
        notificationHeight,
        statusHeight,
        reason: 'single-line settings rows share a height',
      );
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      final stored = (await storage.loadAll()).single;
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(
        stored.pushSubscriptionState.pendingTombstoneGeneration,
        outcome == 'failed' || outcome == 'pending' ? 8 : isNull,
      );
      expect(tombstones, [8]);
      expect(snapshots.last.single.pushNotificationsEnabled, isFalse);
      final offSwitch = tester.widget<Switch>(find.byType(Switch));
      expect(offSwitch.value, isFalse);
      expect(offSwitch.onChanged, isNotNull);
      // Closing and reopening Settings must not strand the off control either.
      showingSettings.value = false;
      await tester.pumpAndSettle();
      showingSettings.value = true;
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNotNull);
      await tester.tap(find.text('Notifications'));
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect((await storage.loadAll()).single.pushNotificationsEnabled, isTrue);
      if (outcome == 'pending') {
        cleanup.complete();
        await tester.pumpAndSettle();
        expect(
          (await storage.loadAll()).single.pushNotificationsEnabled,
          isTrue,
        );
      }
      expect(discoveries, 1);
      // A real reconnect must still discover capability again.
      (container.read(relaySessionProvider.notifier) as _Connected).setStatus(
        SessionStatus.disconnected,
      );
      await tester.pumpAndSettle();
      expect(container.read(currentRelayPushDescriptorProvider).value, isNull);
      (container.read(relaySessionProvider.notifier) as _Connected).setStatus(
        SessionStatus.connected,
      );
      await tester.pump();
      expect(discoveries, 2);
      pending.complete(_descriptor);
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNotNull);
      expect(tester.takeException(), isNull);
      semantics.dispose();
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

class _Connected extends RelaySessionNotifier {
  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

const _descriptor = BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'relay-v1',
  executorPubkey:
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: 4096,
  maxPlaintextLength: 4096,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);
