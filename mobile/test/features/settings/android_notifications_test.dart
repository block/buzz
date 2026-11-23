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
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/push/android_push_authority.dart';
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

import '../../helpers/android_push_fakes.dart';
import '../../shared/community/community_storage_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final fonts = FontLoader('Inter')
      ..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'));
    await fonts.load();
  });

  for (final attests in [true, false]) {
    testWidgets(
      attests
          ? 'an attesting Android phone enrolls and publishes its lease'
          : 'an Android phone that cannot attest keeps working and says so',
      (tester) async {
        tester.view.physicalSize = const Size(375, 1800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
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
        final session = _Connected();
        var advertised = _descriptor;
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final gateway = FakePushGateway(nowSeconds: () => now);
        debugAndroidPushAuthority = AndroidPushAuthority(
          keystore: FakeAndroidKeystore()..attests = attests,
          gateway: fakeGatewayClient(gateway),
          store: _MemoryStore(),
          sleep: (_) async {},
        );
        addTearDown(() {
          debugAndroidPushAuthority = null;
          fcmDeviceToken.value = null;
          pushUnavailableReason.value = null;
          pushEndpointGrants.value = const [];
        });
        fcmDeviceToken.value = 'test-fcm-token';
        const nativePush = MethodChannel('buzz/push_registration');
        final messenger =
            TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        messenger.setMockMethodCallHandler(
          nativePush,
          (call) async => switch (call.method) {
            'startRegistration' || 'refreshToken' || 'syncPushSnapshot' => null,
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
        await storage.save(community);
        await storage.saveActiveId(community.id);

        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              authProvider.overrideWith(_Auth.new),
              communityStorageProvider.overrideWithValue(storage),
              communitySnapshotWriterProvider.overrideWithValue(
                (communities) async {},
              ),
              buzzPushLeaseRevocationStorageProvider.overrideWithValue(
                BuzzPushLeaseRevocationStorage(secure: FakeSecureStorage()),
              ),
              relaySessionProvider.overrideWith(() => session),
              myPubkeyProvider.overrideWithValue(nostr.Keys('1' * 64).public),
              buzzPushDescriptorFetcherProvider.overrideWithValue(
                (_) async => advertised,
              ),
              appLifecycleProvider.overrideWith(_Lifecycle.new),
            ],
            child: BuzzPushBootstrap(
              child: MaterialApp(
                theme: AppTheme.light(),
                home: SettingsPage(
                  profileHeader: const SizedBox.shrink(),
                  identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();

        // Notification settings exist on Android, and the app stays usable
        // either way.
        expect(
          find.byKey(const ValueKey('push-notifications-setting')),
          findsOneWidget,
        );
        expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
        expect(tester.takeException(), isNull);
        if (attests) {
          expect(find.byKey(const ValueKey('push-unavailable')), findsNothing);
          expect(gateway.installations, hasLength(1));
          expect(session.leases.last['active'], isTrue);
          expect(session.leases.last['endpoint'], isNotEmpty);

          // The relay reconnects advertising a new executor key: the old key's
          // delegation is revoked while push stays on, and the lease carries a
          // grant for the new key.
          final oldExecutor = _descriptor.executorPubkey;
          final newExecutor = nostr.Keys('3' * 64).public;
          advertised = _descriptorFor(newExecutor);
          session.executorSecret = '3' * 64;
          final installationBefore = gateway.installations.values.single;
          session.setStatus(SessionStatus.reconnecting);
          await tester.pumpAndSettle();
          session.setStatus(SessionStatus.connected);
          await tester.pumpAndSettle();
          expect(installationBefore.revokedDelegations, {'$oldExecutor/1'});
          expect(installationBefore.delegations, contains(newExecutor));
          expect(session.leases.last['endpoint'], contains(newExecutor));

          // Turning push off keeps the installation: the community remains.
          // Removing that community afterwards is what revokes it.
          final installation = gateway.installations.values.single;
          await tester.tap(find.byType(Switch));
          await tester.pumpAndSettle();
          expect(installation.revokedDelegations, isNotEmpty);
          expect(installation.revoked, isFalse);
          final container = ProviderScope.containerOf(
            tester.element(find.byType(SettingsPage)),
          );
          await container
              .read(communityListProvider.notifier)
              .removeCommunity(community.id);
          await tester.pumpAndSettle();
          expect(installation.revoked, isTrue);
        } else {
          expect(
            find.byKey(const ValueKey('push-unavailable')),
            findsOneWidget,
          );
          expect(find.text('Push is unavailable on this phone'), findsOne);
          expect(gateway.installations, isEmpty);
          expect(session.leases.where((l) => l['active'] == true), isEmpty);
        }
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
        // Foundation checks its debug variables before teardowns run.
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }

  // Mounts only the push bootstrap over a community with the given relay URL
  // and opt-in, recording the native registration calls.
  Future<List<String>> pumpBootstrap(
    WidgetTester tester, {
    required TargetPlatform platform,
    required String relayUrl,
    required bool enabled,
    _Connected? session,
    bool advertisesPush = true,
    FakePushGateway? gateway,
    Future<void>? descriptorGate,
    List<BuzzPushSubscription>? subscriptions,
  }) async {
    debugDefaultTargetPlatformOverride = platform;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final pushGateway =
        gateway ??
        FakePushGateway(
          nowSeconds: () => DateTime.now().millisecondsSinceEpoch ~/ 1000,
        );
    debugAndroidPushAuthority = AndroidPushAuthority(
      keystore: FakeAndroidKeystore(),
      gateway: fakeGatewayClient(pushGateway),
      store: _MemoryStore(),
      sleep: (_) async {},
    );
    addTearDown(() => debugAndroidPushAuthority = null);
    var descriptorFetches = 0;
    final calls = <String>[];
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    for (final channel in const [
      MethodChannel('buzz/push_registration'),
      MethodChannel('buzz/push'),
    ]) {
      messenger.setMockMethodCallHandler(channel, (call) async {
        calls.add(call.method);
        return null;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
    }
    final community = Community.create(name: 'Team', relayUrl: relayUrl)
        .copyWith(
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '1' * 64,
          ),
          pushNotificationsEnabled: enabled,
          pushSubscriptionState: subscriptions == null
              ? null
              : BuzzPushLeaseSubscriptionState.desired(desired: subscriptions),
        );
    await storage.save(community);
    await storage.saveActiveId(community.id);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          authProvider.overrideWith(_Auth.new),
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue(
            (communities) async {},
          ),
          buzzPushLeaseRevocationStorageProvider.overrideWithValue(
            BuzzPushLeaseRevocationStorage(secure: FakeSecureStorage()),
          ),
          relaySessionProvider.overrideWith(() => session ?? _Connected()),
          myPubkeyProvider.overrideWithValue(nostr.Keys('1' * 64).public),
          buzzPushDescriptorFetcherProvider.overrideWithValue((_) async {
            // Discovery reads the descriptor first, which sets push up; the
            // fetch that publication makes next is the one held back.
            if (++descriptorFetches > 1) await descriptorGate;
            if (!advertisesPush) throw const BuzzPushNotAdvertised();
            return _descriptor;
          }),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
        child: const BuzzPushBootstrap(child: SizedBox()),
      ),
    );
    await tester.pumpAndSettle();
    return calls;
  }

  Future<void> unmount(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
    // Foundation checks its debug variables before teardowns run.
    debugDefaultTargetPlatformOverride = null;
  }

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets('a relay URL with a path does not break push on $platform', (
      tester,
    ) async {
      await pumpBootstrap(
        tester,
        platform: platform,
        relayUrl: 'https://relay.example/team',
        enabled: true,
      );

      expect(tester.takeException(), isNull);
      await unmount(tester);
    });
  }

  for (final (name, enabled, advertisesPush) in [
    ('refreshes the FCM token', true, true),
    ('fetches no token while opted out', false, true),
    ('fetches no token for a relay without push', true, false),
  ]) {
    testWidgets('resuming the app $name', (tester) async {
      fcmDeviceToken.value = 'test-fcm-token';
      addTearDown(() => fcmDeviceToken.value = null);
      final calls = await pumpBootstrap(
        tester,
        platform: TargetPlatform.android,
        relayUrl: 'https://relay.example',
        enabled: enabled,
        advertisesPush: advertisesPush,
      );
      calls.clear();

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();

      expect(calls.contains('refreshToken'), enabled && advertisesPush);
      await unmount(tester);
    });
  }

  testWidgets('turning push off while its enrollment waits publishes nothing', (
    tester,
  ) async {
    fcmDeviceToken.value = 'test-fcm-token';
    addTearDown(() => fcmDeviceToken.value = null);
    final gateway = FakePushGateway(
      nowSeconds: () => DateTime.now().millisecondsSinceEpoch ~/ 1000,
    );
    final session = _Connected();
    final descriptorGate = Completer<void>();
    await pumpBootstrap(
      tester,
      platform: TargetPlatform.android,
      relayUrl: 'https://relay.example',
      enabled: true,
      session: session,
      gateway: gateway,
      descriptorGate: descriptorGate.future,
      subscriptions: [
        BuzzPushSubscription(
          filter: BuzzPushFilter(kinds: const [9], pTags: ['a' * 64]),
          notificationClass: 'default',
        ),
      ],
    );
    final container = ProviderScope.containerOf(
      tester.element(find.byType(BuzzPushBootstrap)),
    );
    final notifier = container.read(communityListProvider.notifier);
    await notifier.setPushNotificationsEnabled(
      container.read(communityListProvider).value!.single.id,
      false,
    );

    // The descriptor arrives after the opt-out: nothing is enrolled, delegated
    // or published for a community that no longer wants push.
    descriptorGate.complete();
    await tester.pumpAndSettle();

    expect(gateway.requests, isEmpty);
    expect(session.leases.where((lease) => lease['active'] == true), isEmpty);
    await unmount(tester);
  });

  testWidgets('resuming while the relay reconnects refreshes the FCM token', (
    tester,
  ) async {
    fcmDeviceToken.value = 'test-fcm-token';
    addTearDown(() => fcmDeviceToken.value = null);
    final session = _Connected();
    final calls = await pumpBootstrap(
      tester,
      platform: TargetPlatform.android,
      relayUrl: 'https://relay.example',
      enabled: true,
      session: session,
    );
    session.setStatus(SessionStatus.reconnecting);
    await tester.pumpAndSettle();
    calls.clear();

    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpAndSettle();

    expect(calls, contains('refreshToken'));
    await unmount(tester);
  });
}

class _MemoryStore implements AndroidPushStateStore {
  String? value;

  @override
  Future<String?> read() async => value;

  @override
  Future<void> write(String? next) async => value = next;
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
  String executorSecret = '2' * 64;

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
  }) async {
    expect(event.kind, buzzPushLeaseKind);
    final plaintext = nip44Decrypt(
      getConversationKey(executorSecret, event.pubkey),
      event.content,
    );
    leases.add(jsonDecode(plaintext) as Map<String, dynamic>);
    return event;
  }

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

final _descriptor = _descriptorFor(nostr.Keys('2' * 64).public);

BuzzPushLeaseDescriptor _descriptorFor(String executorPubkey) =>
    BuzzPushLeaseDescriptor(
      origin: 'wss://relay.example',
      executorKeyId: 'relay-v1',
      executorPubkey: executorPubkey,
      transport: 'apns',
      maxLeaseTtlSeconds: 3600,
      maxContentLength: 4096,
      maxPlaintextLength: 4096,
      maxEndpointLength: 2048,
      maxStringLength: 512,
    );
