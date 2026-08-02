import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel('buzz/push');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    apnsDeviceToken.value = null;
    apnsRegistrationError.value = null;
    pushEndpointGrants.value = const [];
    pushEndpointGrantError.value = null;
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
    expect(grants.single.appProfile, 'buzz-ios-sandbox');
    expect(grants.single.endpointEpoch, 1);
    expect(grants.single.generation, 1);
    expect(grants.single.publishedGeneration, isNull);
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
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(_channel, (call) async {
        methods.add(call.method);
        if (call.method == 'devEnrollPush') {
          enrollmentArguments.add(call.arguments);
          return _grantMap('new-grant');
        }
        if (call.method == 'endpointGrants') {
          return [_grantMap('new-grant')];
        }
        fail('Unexpected method ${call.method}');
      });

      final firstGrant = await enrollBuzzDevPush(
        'wss://relay.example/',
        'https://gateway-one.example/',
      );
      final secondGrant = await enrollBuzzDevPush(
        'wss://relay.example/',
        'https://gateway-two.example/',
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
        'devEnrollPush',
        'endpointGrants',
        'devEnrollPush',
        'endpointGrants',
      ]);
      expect(pushEndpointGrants.value.single.endpointGrant, 'new-grant');
    },
  );

  test('development push gateway matches the compiled configuration', () {
    const expectedGateway = String.fromEnvironment(
      'BUZZ_PUSH_GATEWAY_URL',
      defaultValue: 'http://localhost:8080',
    );
    expect(Env.pushGatewayUrl, expectedGateway);
  });

  test('persists the acknowledged push lease generation', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final methods = <String>[];
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(_channel, (call) async {
      methods.add(call.method);
      if (call.method == 'devMarkPushLeasePublished') {
        expect(call.arguments, {
          'relayOrigin': 'wss://relay.example',
          'appProfile': 'buzz-ios-sandbox',
          'generation': 1,
        });
        return null;
      }
      if (call.method == 'endpointGrants') {
        return [_grantMap('opaque-grant', publishedGeneration: 1)];
      }
      fail('Unexpected method ${call.method}');
    });

    final grant = BuzzPushEndpointGrant.fromMap(_grantMap('opaque-grant'));
    await markBuzzDevPushLeasePublished(grant);

    expect(methods, ['devMarkPushLeasePublished', 'endpointGrants']);
    expect(pushEndpointGrants.value.single.publishedGeneration, 1);
  });

  test('rejects published generation ahead of the endpoint grant', () {
    expect(
      () => BuzzPushEndpointGrant.fromMap(
        _grantMap('opaque-grant', publishedGeneration: 2),
      ),
      throwsA(isA<FormatException>()),
    );
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
}

Map<String, Object> _grantMap(
  String endpointGrant, {
  int? publishedGeneration,
}) => {
  'relayOrigin': 'wss://relay.example',
  'relayPubkey': 'a' * 64,
  'installationId': 'c' * 32,
  'endpointGrant': endpointGrant,
  'endpointHash': 'b' * 64,
  'appProfile': 'buzz-ios-sandbox',
  'endpointEpoch': 1,
  'generation': 1,
  'publishedGeneration': ?publishedGeneration,
  'expiresAt': 1752624000,
};
