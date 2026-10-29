import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:buzz/app.dart';
import 'package:buzz/main.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'package:buzz/shared/push/push_setup_recovery.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import '../community/community_storage_test.dart' show FakeSecureStorage;
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/push/push_lease_revocation_outbox.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  final nativeChannels = <MethodChannel>[];
  setUp(() {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final args = call.arguments as Map;
        final channel = MethodChannel('${args['viewType']}/${args['id']}');
        nativeChannels.add(channel);
        messenger.setMockMethodCallHandler(channel, (_) async => null);
      }
      return null;
    });
    messenger.setMockMethodCallHandler(
      const MethodChannel('dev.fluttercommunity.plus/connectivity_status'),
      (_) async => null,
    );
  });
  tearDown(() {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
    for (final channel in nativeChannels) {
      messenger.setMockMethodCallHandler(channel, null);
    }
    nativeChannels.clear();
    messenger.setMockMethodCallHandler(
      const MethodChannel('dev.fluttercommunity.plus/connectivity_status'),
      null,
    );
    debugDefaultTargetPlatformOverride = null;
  });
  testWidgets('subscription setup failure does not replace the app', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final community = Community.create(
      name: 'Team',
      relayUrl: 'https://relay.example',
      pubkey: 'a' * 64,
    ).copyWith(pushNotificationsEnabled: true);
    final storage = _FailingStorage();
    await storage.save(community);
    storage.fail = true;
    final channels = _Channels();
    await tester.pumpWidget(
      ProviderScope(
        retry: (_, _) => null,
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          authProvider.overrideWith(_Auth.new),
          ageSignalProvider.overrideWith(_Age.new),
          relaySessionProvider.overrideWith(_Session.new),
          activeCommunityProvider.overrideWith((_) async => community),
          currentRelayPushDescriptorProvider.overrideWith(
            (_) async => _descriptor,
          ),
          channelsProvider.overrideWith(() => channels),
          channelMutesProvider.overrideWith(_Mutes.new),
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
        ],
        child: const BuzzPushBootstrap(child: App()),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(find.byType(HomePage), findsOneWidget);
    expect(tester.takeException(), isNull);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(App)),
    );
    expect(
      container.read(buzzPushSetupStatusProvider).error,
      isA<FormatException>(),
    );
    // The root navigator owns this route; Duo also keeps a detail navigator.
    final navigator = tester.state<NavigatorState>(
      find.byType(Navigator).first,
    );
    unawaited(
      navigator.push(
        MaterialPageRoute<void>(
          builder: (_) => const Text('Open conversation'),
        ),
      ),
    );
    await tester.pumpAndSettle();

    // Correcting the inputs retries the real provider, but asynchronous secure
    // storage failure must also remain observable without replacing the route.
    channels.useValidChannel();
    await tester.pump();
    await tester.pump();
    expect(
      container.read(buzzPushSetupStatusProvider).error,
      isA<StateError>(),
    );
    expect(
      (await storage.loadAll()).single.pushSubscriptionState.desired,
      isEmpty,
    );
    expect(find.text('Open conversation'), findsOneWidget);
    expect(
      tester.state<NavigatorState>(find.byType(Navigator).first),
      same(navigator),
    );

    storage.fail = false;
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();
    expect(container.read(buzzPushSetupStatusProvider).hasError, isFalse);
    expect(
      (await storage.loadAll()).single.pushSubscriptionState.desired,
      isNotEmpty,
    );
    expect(find.text('Open conversation'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'push dependency failure is visible and retry preserves navigation',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final semantics = tester.ensureSemantics();
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 844);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      await tester.runAsync(() async {
        await (FontLoader(
          'Inter',
        )..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'))).load();
        await (FontLoader(
          'BuzzTabler',
        )..addFont(rootBundle.load('assets/fonts/TablerIcons.ttf'))).load();
      });
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final community = Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example',
      ).copyWith(pushNotificationsEnabled: true);
      var fail = true;
      var attempts = 0;
      final journal = BuzzPushLeaseRevocationStorage(
        secure: FakeSecureStorage(),
      );
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final removals = [for (var i = 0; i < 2; i++) _removableCommunity(i)];
      for (final removal in removals) {
        await storage.save(removal);
      }
      final published = <String>[];
      const nativePush = MethodChannel('buzz/push');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        nativePush,
        (call) async {
          if (call.method != 'endpointGrants') return null;
          return [
            {
              'relayOrigin': 'wss://relay.example',
              'relayPubkey': 'a' * 64,
              'installationId': '0' * 32,
              'endpointGrant': 'opaque',
              'endpointHash': 'b' * 64,
              'endpointEpoch': 1,
              'generation': 1,
              'expiresAt': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 3600,
            },
          ];
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          nativePush,
          null,
        ),
      );
      await tester.pumpWidget(
        ProviderScope(
          retry: (_, _) => null,
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            authProvider.overrideWith(_Auth.new),
            relaySessionProvider.overrideWith(_Session.new),
            activeCommunityProvider.overrideWith((_) async => community),
            currentRelayPushDescriptorProvider.overrideWith(
              (_) async => _descriptor,
            ),
            buzzPushLeaseRevocationOutboxProvider.overrideWith((ref) {
              attempts++;
              if (fail) throw StateError('injected push dependency failure');
              final outbox = BuzzPushLeaseRevocationOutbox(
                storage: journal,
                publisher: (record) async =>
                    published.add(record.installationId),
              );
              ref.onDispose(outbox.dispose);
              return outbox;
            }),
          ],
          child: BuzzPushBootstrap(
            child: Consumer(
              builder: (context, ref, _) => MaterialApp(
                theme: AppTheme.light(),
                home: TextButton(
                  onPressed: () => retryBuzzPushSetup(ref),
                  child: const Text('Core app'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Core app'), findsOneWidget);
      final navigator = tester.state<NavigatorState>(
        find.byType(Navigator).first,
      );
      unawaited(
        navigator.push(
          MaterialPageRoute<void>(
            builder: (_) => SettingsPage(
              profileHeader: const SizedBox.shrink(),
              identityRecoveryPageBuilder: (_) => const SizedBox.shrink(),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Notification setup failed'), findsOneWidget);
      expect(
        tester
            .getSemantics(find.byKey(const ValueKey('push-setup-retry')))
            .getSemanticsData()
            .hasAction(ui.SemanticsAction.tap),
        isTrue,
      );
      await _capture(tester, 'notification-setup-failed');
      final container = ProviderScope.containerOf(
        tester.element(find.byType(SettingsPage)),
      );
      final communities = container.read(communityListProvider.notifier);
      // A removal attempted while initialization is broken caches a failed
      // callback provider. Recovery must refresh it as well as the worker.
      await expectLater(
        communities.removeCommunity(removals.first.id),
        throwsA(
          predicate(
            (error) =>
                error.toString().contains('injected push dependency failure'),
          ),
        ),
      );
      expect(
        (await storage.loadAll()).map((value) => value.id),
        contains(removals.first.id),
      );
      // Exhaust automatic retries. Persistent failures stay visible, but stop
      // waking forever. Manual recovery remains available after exhaustion.
      for (final seconds in [5, 10, 20, 60]) {
        await tester.pump(Duration(seconds: seconds));
        await tester.pump();
      }
      expect(attempts, 4);
      fail = false;
      await tester.ensureVisible(
        find.byKey(const ValueKey('push-setup-retry')),
      );
      await tester.tap(find.byKey(const ValueKey('push-setup-retry')));
      await tester.pumpAndSettle();
      expect(find.byType(SettingsPage), findsOneWidget);
      expect(find.text('Notification setup failed'), findsNothing);
      await _capture(tester, 'notification-setup-recovered');
      expect(
        tester.state<NavigatorState>(find.byType(Navigator).first),
        same(navigator),
      );
      await communities.removeCommunity(removals.first.id);
      await tester.pump();
      expect(published, [removals.first.pushLeaseInstallationId]);
      expect(await journal.loadAll(), isEmpty);

      // Community removal has now cached this worker's callbacks. Retrying an
      // unrelated setup operation must not dispose that healthy worker.
      navigator.pop();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Core app'));
      await tester.pumpAndSettle();
      await communities.removeCommunity(removals.last.id);
      await tester.pump();
      expect(published, [
        for (final removal in removals) removal.pushLeaseInstallationId,
      ]);
      expect(await journal.loadAll(), isEmpty);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
      semantics.dispose();
    },
  );

  testWidgets('cold-start native failure cannot prevent runApp', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    SharedPreferences.setMockInitialValues({});
    const channel = MethodChannel('buzz/push');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    var fail = true;
    final pendingResponse = Completer<Object?>();
    addTearDown(() => pendingPushNotificationLink.value = null);
    messenger.setMockMethodCallHandler(channel, (call) async {
      if (call.method == 'takePendingNotificationResponse' && fail) {
        return pendingResponse.future;
      }
      return call.method == 'takePendingNotificationResponse'
          ? {
              'eventId': 'event',
              'channelId': 'channel',
              'communityId': 'community',
            }
          : null;
    });
    addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
    await runBuzzApp(const MaterialApp(home: Text('Core app')));
    await tester.pump();
    await tester.pump();
    expect(find.text('Core app'), findsOneWidget);
    pendingResponse.completeError(
      PlatformException(code: 'injected_native_failure'),
    );
    await tester.pump();
    final container = ProviderScope.containerOf(
      tester.element(find.text('Core app')),
    );
    expect(
      container.read(buzzPushSetupStatusProvider).error,
      isA<PlatformException>(),
    );
    fail = false;
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();
    expect(container.read(buzzPushSetupStatusProvider).hasError, isFalse);
    expect(pendingPushNotificationLink.value?.messageId, 'event');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });

  test('setup retries stop after bounded backoff', () {
    final error = StateError('persistent failure');
    expect(
      [for (var i = 0; i < 5; i++) pushSetupRetry(i, error)],
      [
        const Duration(seconds: 5),
        const Duration(seconds: 10),
        const Duration(seconds: 20),
        null,
        null,
      ],
    );
  });
}

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _Age extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;
  @override
  Future<void> request() async {}
}

class _Session extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

const _descriptor = BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'key',
  executorPubkey: 'pubkey',
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  // The relay's NIP-11 limits, which fit a DM lease at the endpoint limit.
  maxContentLength: 65536,
  maxPlaintextLength: 32768,
  maxEndpointLength: 2048,
  maxStringLength: 512,
);

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_channel('invalid-channel')];
  void useValidChannel() =>
      state = AsyncData([_channel('123e4567-e89b-42d3-a456-426614174000')]);
}

class _Mutes extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(isReady: true);
}

Channel _channel(String id) => Channel(
  id: id,
  name: 'DM',
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: 'a' * 64,
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

class _FailingStorage extends CommunityStorage {
  _FailingStorage() : super(secure: FakeSecureStorage());
  bool fail = false;
  @override
  Future<void> save(Community community) async {
    if (fail) throw StateError('injected secure storage failure');
    await super.save(community);
  }
}

Future<void> _capture(WidgetTester tester, String name) async {
  final directory = Platform.environment['PUSH_SETUP_SCREENSHOTS'];
  if (directory == null) return;
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byType(RepaintBoundary).first,
  );
  await tester.runAsync(() async {
    final image = await boundary.toImage();
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    await Directory(directory).create(recursive: true);
    await File('$directory/$name.png').writeAsBytes(data!.buffer.asUint8List());
    image.dispose();
  });
}

Community _removableCommunity(int index) {
  final keys = nostr.Keys('${index + 1}' * 64);
  return Community.create(
    name: 'Removed $index',
    relayUrl: 'https://relay.example',
    nsec: keys.nsec,
    pubkey: keys.public,
  ).copyWith(
    pushNotificationsEnabled: true,
    pushSubscriptionState: const BuzzPushLeaseSubscriptionState.desired()
        .withReservedGeneration(4),
  );
}
