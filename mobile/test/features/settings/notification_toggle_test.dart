import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
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
  for (final outcome in [
    'accepted',
    'refused',
    'unanswered',
    'still active',
    'scaled',
    'unset preference',
  ]) {
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
      final secure = FakeSecureStorage();
      final storage = CommunityStorage(secure: secure);
      final subscriptions = [
        BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        ),
      ];
      var discoveries = 0;
      final session = _Connected();
      apnsDeviceToken.value = 'test-apns-token';
      addTearDown(() => apnsDeviceToken.value = null);
      final grant = <String, Object>{
        'relayOrigin': 'wss://relay.example',
        'relayPubkey': _descriptor.executorPubkey,
        'installationId': 'b' * 32,
        'endpointGrant': 'test-endpoint-grant',
        'endpointHash': 'c' * 64,
        'endpointEpoch': 1,
        'generation': 1,
        'expiresAt': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
      };
      const nativePush = MethodChannel('buzz/push');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(
        nativePush,
        (call) async => switch (call.method) {
          'enrollPush' => grant,
          'endpointGrants' => [grant],
          'startRegistration' ||
          'syncPushSnapshot' ||
          'takePendingNotificationResponse' => null,
          _ => throw StateError('Unexpected native call ${call.method}'),
        },
      );
      addTearDown(() => messenger.setMockMethodCallHandler(nativePush, null));
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
            pushLeaseInstallationId: 'd' * 32,
            pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
              desired: subscriptions,
            ).withAccepted(subscriptions: subscriptions, generation: 7),
          );
      if (outcome == 'unset preference') {
        final saved = community.toJson()..remove('pushNotificationsEnabledV2');
        saved['pushNotificationsEnabled'] = false;
        secure['buzz_communities'] = jsonEncode([saved]);
      } else {
        await storage.save(community);
      }
      await storage.saveActiveId(community.id);

      final reports = _RecordingDiagnostics();
      final cleanup = Completer<void>();
      final showingSettings = ValueNotifier(true);
      addTearDown(showingSettings.dispose);
      final snapshots = <List<Community>>[];
      final tombstones = <int?>[];
      // What the relay says the lease is when the app checks after an
      // unanswered opt-out.
      var leaseReads = 0;
      Object? leaseReadError = StateError('relay unreachable');
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
              deadline,
            }) async {
              tombstones.add(generation);
              if (outcome == 'refused') {
                throw const RelayEventRejectedException('blocked');
              }
              if (outcome == 'still active') {
                throw SignedEventUnacknowledged(TimeoutException('no OK'));
              }
              // Hangs past the deadline, then lands late.
              if (outcome == 'unanswered') await cleanup.future;
              await publishBuzzPushLeaseTombstone(
                descriptor: _descriptor,
                installationId: community.pushLeaseInstallationId!,
                generation: generation!,
                nsec: community.nsec!,
                memberPubkey: pubkeyFromNsec(community.nsec)!,
                submit: SignedEventRelay(
                  session: session,
                  nsec: community.nsec,
                ).submit,
              );
            }),
            communityPushLeaseReaderProvider.overrideWithValue((
              community,
            ) async {
              leaseReads++;
              if (leaseReadError case final error?) throw error;
              return (active: outcome == 'still active', generation: 9);
            }),
            buzzPushLeaseRevocationStorageProvider.overrideWithValue(
              BuzzPushLeaseRevocationStorage(secure: FakeSecureStorage()),
            ),
            relaySessionProvider.overrideWith(() => session),
            myPubkeyProvider.overrideWithValue(nostr.Keys('1' * 64).public),
            buzzPushDescriptorFetcherProvider.overrideWithValue((_) async {
              discoveries++;
              return _descriptor;
            }),
            diagnosticReporterProvider.overrideWithValue(reports),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
          ],
          child: BuzzPushBootstrap(
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
      expect(session.leases.single['active'], isTrue);
      // Capability discovery plus the active publication.
      expect(discoveries, 2);
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
      Future<void> reconnect() async {
        session.setStatus(SessionStatus.disconnected);
        await tester.pumpAndSettle();
        session.setStatus(SessionStatus.connected);
        await tester.pumpAndSettle();
      }

      const warning = 'Could not confirm that notifications were turned off.';
      await tester.tap(find.byType(Switch));
      if (outcome == 'unanswered') {
        // Until the relay answers, the row says off is not yet confirmed,
        // and holds the switch so Off, On, Off can't race the first attempt.
        await tester.pump();
        await tester.pump();
        expect(find.text('Turning off…'), findsOneWidget);
        expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNull);
        await tester.tap(find.text('Notifications'));
        await tester.pump();
        expect(tombstones, [9]);
        await tester.pump(pushOptOutTimeout + const Duration(seconds: 1));
      }
      await tester.pumpAndSettle();
      expect(find.text('Turning off…'), findsNothing);
      expect(tombstones, [9]);
      expect(discoveries, 2);
      if (outcome != 'accepted' &&
          outcome != 'scaled' &&
          outcome != 'unset preference') {
        // A failed opt-out shows the confirmed On, with the warning.
        expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
        expect(find.text(warning), findsOneWidget);
        expect(find.text('Notifications may continue.'), findsOneWidget);
        expect(
          tester.getSemantics(
            find.byKey(const ValueKey('push-opt-out-failed')),
          ),
          isSemantics(isLiveRegion: true),
        );
      }
      var stored = (await storage.loadAll()).single;
      if (outcome == 'refused') {
        // The relay never stored it, so push is simply back on, and the
        // community stays in the native snapshot.
        expect(stored.pushNotificationsEnabled, isTrue);
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        expect(snapshots.last.single.pushNotificationsEnabled, isTrue);
        // A reconnect retries nothing and reads nothing.
        await reconnect();
        expect(tombstones, [9]);
        expect(leaseReads, 0);
        expect(find.text(warning), findsOneWidget);
        await tester.pumpWidget(const SizedBox.shrink());
        semantics.dispose();
        debugDefaultTargetPlatformOverride = null;
        return;
      }
      expect(stored.pushNotificationsEnabled, isFalse);
      expect(snapshots.last.single.pushNotificationsEnabled, isFalse);
      if (outcome == 'unanswered' || outcome == 'still active') {
        // The relay may hold it: the opt-out stays saved as a guard.
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, 9);
        // The app asks the relay right away, and again on each sync until it
        // answers. A read that fails changes nothing and retries nothing.
        expect(leaseReads, 1);
        await reconnect();
        expect(leaseReads, 2);
        expect(tombstones, [9]);
        expect(find.text(warning), findsOneWidget);
        // The next sync that can shows what the relay holds.
        leaseReadError = null;
        await reconnect();
        expect(leaseReads, 3);
        expect(tombstones, [9]);
        stored = (await storage.loadAll()).single;
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        if (outcome == 'still active') {
          expect(stored.pushNotificationsEnabled, isTrue);
          expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
          expect(find.text(warning), findsOneWidget);
          await tester.pumpWidget(const SizedBox.shrink());
          semantics.dispose();
          debugDefaultTargetPlatformOverride = null;
          return;
        }
        expect(stored.pushNotificationsEnabled, isFalse);
        expect(stored.pushSubscriptionState.acceptedGeneration, 9);
        expect(find.text(warning), findsNothing);
        // A settled opt-out isn't read again.
        await reconnect();
        expect(leaseReads, 3);
      } else {
        expect(stored.pushSubscriptionState.pendingTombstoneGeneration, isNull);
        await reconnect();
        expect(leaseReads, 0);
      }
      expect(
        session.leases.where((lease) => lease['active'] == true),
        hasLength(1),
      );
      // Each reconnect discovers capability again.
      final discovered = discoveries;
      expect(discovered, greaterThan(2));
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
      expect(
        session.leases.where((lease) => lease['active'] == true).length,
        2,
        reason:
            're-enabling must publish an active lease before the renewal timer',
      );
      expect(session.leases.last['generation'], greaterThan(tombstones.last!));
      if (outcome == 'unanswered') {
        cleanup.complete();
        await tester.pumpAndSettle();
        expect(
          (await storage.loadAll()).single.pushNotificationsEnabled,
          isTrue,
        );
      }
      final latestLease = session.leases.reduce(
        (a, b) => (a['generation'] as int) > (b['generation'] as int) ? a : b,
      );
      expect(
        latestLease['active'],
        isTrue,
        reason: 'a delayed old tombstone cannot supersede the new active lease',
      );
      expect(discoveries, discovered + 1);
      // A real reconnect must still discover capability again.
      (container.read(relaySessionProvider.notifier) as _Connected).setStatus(
        SessionStatus.disconnected,
      );
      await tester.pumpAndSettle();
      expect(container.read(currentRelayPushDescriptorProvider).value, isNull);
      (container.read(relaySessionProvider.notifier) as _Connected).setStatus(
        SessionStatus.connected,
      );
      await tester.pumpAndSettle();
      expect(discoveries, discovered + 2);
      expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNotNull);
      // A gateway reply the app cannot read is reported while the signing
      // member is still the reported member, and not after a community switch.
      messenger.setMockMethodCallHandler(
        nativePush,
        (call) async => switch (call.method) {
          'enrollPush' => throw PlatformException(
            code: 'gateway_response_invalid',
          ),
          'startRegistration' ||
          'syncPushSnapshot' ||
          'takePendingNotificationResponse' => null,
          _ => throw StateError('Unexpected native call ${call.method}'),
        },
      );
      reports.member = nostr.Keys('1' * 64).public;
      apnsDeviceToken.value = 'test-apns-token-2';
      await tester.pumpAndSettle();
      expect(reports.operations, [DiagnosticOperation.pushGatewayResponse]);
      reports.member = 'e' * 64;
      apnsDeviceToken.value = 'test-apns-token-3';
      await tester.pumpAndSettle();
      expect(reports.operations, hasLength(1));
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      semantics.dispose();
      debugDefaultTargetPlatformOverride = null;
    });
  }
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final operations = <DiagnosticOperation>[];

  @override
  String? member;

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    operations.add(operation);
    super.reportError(error, stackTrace, operation: operation);
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
  final leases = <Map<String, dynamic>>[];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    expect(event.kind, buzzPushLeaseKind);
    final plaintext = nip44Decrypt(
      getConversationKey('2' * 64, event.pubkey),
      event.content,
    );
    leases.add(jsonDecode(plaintext) as Map<String, dynamic>);
    return event;
  }

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

final _descriptor = BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'relay-v1',
  executorPubkey: nostr.Keys('2' * 64).public,
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: 4096,
  maxPlaintextLength: 4096,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);
