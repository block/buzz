import 'dart:async';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _channel = MethodChannel('buzz/push');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    apnsDeviceToken.value = null;
    apnsRegistrationError.value = null;
    pushEndpointGrants.value = const [];
    pushEndpointGrantError.value = null;
    pushCommunitySnapshotError.value = null;
    pendingPushNotificationLink.value = null;
    installBuzzPushMethodHandler();
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, null);
    debugDefaultTargetPlatformOverride = null;
  });

  test('captures APNs token success and clears the previous error', () async {
    apnsRegistrationError.value = 'old error';
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          _channel.codec.encodeMethodCall(
            const MethodCall('apnsTokenChanged', {'token': '01ab'}),
          ),
          (_) {},
        );
    expect(apnsDeviceToken.value, '01ab');
    expect(apnsRegistrationError.value, isNull);
  });

  test(
    'starts native permission and APNs registration without a result gate',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_channel, (call) async {
            expect(call.method, 'startRegistration');
            return null;
          });

      await startBuzzPushRegistration();
    },
  );

  test('reads native notification authorization status', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          expect(call.method, 'notificationAuthorizationStatus');
          return 'denied';
        });

    expect(
      await readBuzzPushAuthorizationStatus(),
      BuzzPushAuthorizationStatus.denied,
    );
  });

  test('opens native notification settings', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          expect(call.method, 'openNotificationSettings');
          return true;
        });

    expect(await openBuzzPushNotificationSettings(), isTrue);
  });

  test('refreshes not-determined permission to denied on resume', () async {
    final statuses = [
      BuzzPushAuthorizationStatus.notDetermined,
      BuzzPushAuthorizationStatus.denied,
    ];
    final container = ProviderContainer(
      overrides: [
        appLifecycleProvider.overrideWith(_TestAppLifecycleNotifier.new),
        buzzPushAuthorizationStatusReaderProvider.overrideWithValue(
          () async => statuses.removeAt(0),
        ),
      ],
    );
    addTearDown(container.dispose);

    expect(
      await container.read(buzzPushAuthorizationStatusProvider.future),
      BuzzPushAuthorizationStatus.notDetermined,
    );
    final lifecycle =
        container.read(appLifecycleProvider.notifier)
            as _TestAppLifecycleNotifier;
    lifecycle.setState(AppLifecycleState.paused);
    lifecycle.setState(AppLifecycleState.resumed);
    await _waitForAuthorization(container, BuzzPushAuthorizationStatus.denied);
  });

  test('refreshes externally revoked permission on resume', () async {
    final statuses = [
      BuzzPushAuthorizationStatus.authorized,
      BuzzPushAuthorizationStatus.denied,
    ];
    final container = ProviderContainer(
      overrides: [
        appLifecycleProvider.overrideWith(_TestAppLifecycleNotifier.new),
        buzzPushAuthorizationStatusReaderProvider.overrideWithValue(
          () async => statuses.removeAt(0),
        ),
      ],
    );
    addTearDown(container.dispose);

    expect(
      await container.read(buzzPushAuthorizationStatusProvider.future),
      BuzzPushAuthorizationStatus.authorized,
    );
    final lifecycle =
        container.read(appLifecycleProvider.notifier)
            as _TestAppLifecycleNotifier;
    lifecycle.setState(AppLifecycleState.paused);
    lifecycle.setState(AppLifecycleState.resumed);
    await _waitForAuthorization(container, BuzzPushAuthorizationStatus.denied);
  });

  test('reads and exposes persisted endpoint grants on iOS', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(_channel, (call) async {
      expect(call.method, 'endpointGrants');
      return [_grantMap('opaque-grant')];
    });

    final grants = await readBuzzPushEndpointGrants();

    expect(grants, hasLength(1));
    expect(grants.single.relayOrigin, 'wss://relay.example');
    expect(grants.single.relayPubkey, 'a' * 64);
    expect(grants.single.installationId, 'c' * 32);
    expect(grants.single.endpointGrant, 'opaque-grant');
    expect(grants.single.endpointHash, 'b' * 64);
    expect(grants.single.appProfile, 'buzz-ios-dogfood');
    expect(grants.single.endpointEpoch, 1);
    expect(grants.single.generation, 1);
    expect(grants.single.expiresAt, 1752624000);
    expect(pushEndpointGrants.value.single.endpointGrant, 'opaque-grant');
    expect(pushEndpointGrantError.value, isNull);
  });

  test(
    'debug enrollment carries the configured relay and gateway URLs',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final methods = <String>[];
      final enrollmentArguments = <Object?>[];
      final snapshotArguments = <Object?>[];
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(_channel, (call) async {
        methods.add(call.method);
        if (call.method == 'enrollPush') {
          enrollmentArguments.add(call.arguments);
          return _grantMap('new-grant');
        }
        if (call.method == 'endpointGrants') {
          return [_grantMap('new-grant')];
        }
        if (call.method == 'syncPushSnapshot') {
          snapshotArguments.add(call.arguments);
          return null;
        }
        fail('Unexpected method ${call.method}');
      });

      final firstGrant = await enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway-one.example/',
      );
      final secondGrant = await enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway-two.example/',
        communitiesForSnapshotRefresh: [
          Community(
            id: 'community-id',
            name: 'Community',
            relayUrl: 'wss://relay.example/',
            pubkey: 'd' * 64,
            pushNotificationsEnabled: true,
            addedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
        ],
      );

      expect(firstGrant.endpointGrant, 'new-grant');
      expect(secondGrant.endpointGrant, 'new-grant');
      expect(enrollmentArguments, [
        {
          'relayUrl': 'wss://relay.example/',
          'gatewayUrl': 'https://gateway-one.example/',
        },
        {
          'relayUrl': 'wss://relay.example/',
          'gatewayUrl': 'https://gateway-two.example/',
        },
      ]);
      expect(methods, [
        'enrollPush',
        'endpointGrants',
        'enrollPush',
        'endpointGrants',
        'syncPushSnapshot',
      ]);
      expect(snapshotArguments, [
        {
          'section': 'communities',
          'communities': [
            {
              'id': 'community-id',
              'name': 'Community',
              'relayUrl': 'wss://relay.example/',
              'pubkey': 'd' * 64,
              'policies': <Object?>[],
            },
          ],
          'signingKeys': <String, String>{},
        },
      ]);
      expect(pushEndpointGrants.value.single.endpointGrant, 'new-grant');
    },
  );

  test('development push gateway matches the compiled configuration', () {
    const expectedGateway = String.fromEnvironment(
      'BUZZ_PUSH_GATEWAY_URL',
      defaultValue: 'https://push.buzz.xyz',
    );
    expect(Env.pushGatewayUrl, expectedGateway);
  });

  test('exposes APNs registration failure', () async {
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          _channel.codec.encodeMethodCall(
            const MethodCall('apnsRegistrationFailed', {'message': 'denied'}),
          ),
          (_) {},
        );
    expect(apnsRegistrationError.value, 'denied');
  });

  test('routes a warm notification response with opaque IDs', () async {
    final response = Completer<ByteData?>();
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          _channel.codec.encodeMethodCall(
            MethodCall('notificationOpened', {
              'eventId': 'MESSAGE-ID',
              'communityId': 'community-id',
              'channelId': 'CHANNEL/GENERAL',
            }),
          ),
          response.complete,
        );

    final envelope = await response.future;
    expect(envelope, isNotNull);
    expect(_channel.codec.decodeEnvelope(envelope!), 'handled');
    expect(
      pendingPushNotificationLink.value,
      MessageDeepLink(
        communityId: 'community-id',
        channelId: 'CHANNEL/GENERAL',
        messageId: 'MESSAGE-ID',
      ),
    );
  });

  test('rejects a notification response with an empty channel ID', () async {
    final response = Completer<ByteData?>();
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          _channel.codec.encodeMethodCall(
            MethodCall('notificationOpened', {
              'eventId': 'message-id',
              'communityId': 'community-id',
              'channelId': '',
            }),
          ),
          response.complete,
        );

    final envelope = await response.future;
    expect(envelope, isNotNull);
    expect(_channel.codec.decodeEnvelope(envelope!), 'ignored');
    expect(pendingPushNotificationLink.value, isNull);
  });

  test(
    'rejects a notification response with a non-string message ID',
    () async {
      final response = Completer<ByteData?>();
      await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
            _channel.name,
            _channel.codec.encodeMethodCall(
              MethodCall('notificationOpened', {
                'eventId': 42,
                'communityId': 'community-id',
                'channelId': 'channel-id',
              }),
            ),
            response.complete,
          );

      final envelope = await response.future;
      expect(envelope, isNotNull);
      expect(_channel.codec.decodeEnvelope(envelope!), 'ignored');
      expect(pendingPushNotificationLink.value, isNull);
    },
  );

  test('rejects a notification response with an absent message ID', () async {
    final response = Completer<ByteData?>();
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          _channel.codec.encodeMethodCall(
            const MethodCall('notificationOpened', {
              'communityId': 'community-id',
              'channelId': 'channel-id',
            }),
          ),
          response.complete,
        );

    final envelope = await response.future;
    expect(envelope, isNotNull);
    expect(_channel.codec.decodeEnvelope(envelope!), 'ignored');
    expect(pendingPushNotificationLink.value, isNull);
  });

  test('pulls a cold-start notification response from native iOS', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          expect(call.method, 'takePendingNotificationResponse');
          return {
            'eventId': 'b' * 64,
            'communityId': 'community-id',
            'channelId': '123e4567-e89b-42d3-a456-426614174000',
          };
        });

    await syncPendingBuzzPushNotificationResponse();

    expect(
      pendingPushNotificationLink.value,
      MessageDeepLink(
        communityId: 'community-id',
        channelId: '123e4567-e89b-42d3-a456-426614174000',
        messageId: 'b' * 64,
      ),
    );
  });
}

class _TestAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;

  void setState(AppLifecycleState value) => state = value;
}

Future<void> _waitForAuthorization(
  ProviderContainer container,
  BuzzPushAuthorizationStatus expected,
) async {
  for (var attempt = 0; attempt < 20; attempt++) {
    if (container.read(buzzPushAuthorizationStatusProvider).value == expected) {
      return;
    }
    await Future<void>.delayed(Duration.zero);
  }
  fail('Authorization status did not refresh to $expected');
}

Map<String, Object> _grantMap(String endpointGrant) => {
  'relayOrigin': 'wss://relay.example',
  'relayPubkey': 'a' * 64,
  'installationId': 'c' * 32,
  'endpointGrant': endpointGrant,
  'endpointHash': 'b' * 64,
  'appProfile': 'buzz-ios-dogfood',
  'endpointEpoch': 1,
  'generation': 1,
  'expiresAt': 1752624000,
};
