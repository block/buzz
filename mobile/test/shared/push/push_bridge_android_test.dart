import 'package:buzz/shared/push/android_push_authority.dart';
import 'package:buzz/shared/push/android_push_keystore.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/android_push_fakes.dart';

const _channel = MethodChannel('buzz/push_registration');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    fcmDeviceToken.value = null;
    fcmRegistrationError.value = null;
    pushUnavailableReason.value = null;
    pushEndpointGrants.value = const [];
    pushEndpointGrantError.value = null;
    installBuzzPushMethodHandler();
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, null);
    debugDefaultTargetPlatformOverride = null;
    debugAndroidPushAuthority = null;
  });

  Future<void> native(String method, Map<String, Object?> arguments) =>
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
            _channel.name,
            _channel.codec.encodeMethodCall(MethodCall(method, arguments)),
            (_) {},
          );

  test('asks the native side to register and to refresh the token', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final methods = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          methods.add(call.method);
          return null;
        });

    await startBuzzPushRegistration();
    await refreshBuzzPushToken();

    expect(methods, ['startRegistration', 'refreshToken']);
  });

  test('a token clears an earlier registration failure', () async {
    await native('fcmRegistrationFailed', {'message': 'no network'});
    expect(fcmRegistrationError.value, 'no network');
    expect(pushUnavailableReason.value, 'no network');

    await native('fcmTokenChanged', {'token': 'fcm-token'});

    expect(fcmDeviceToken.value, 'fcm-token');
    expect(fcmRegistrationError.value, isNull);
    expect(pushUnavailableReason.value, isNull);
  });

  test('a token does not clear an attestation failure', () async {
    pushUnavailableReason.value = 'The phone cannot attest its key.';

    await native('fcmTokenChanged', {'token': 'fcm-token'});

    expect(pushUnavailableReason.value, 'The phone cannot attest its key.');
  });

  test('enrolls through the Dart authority and exposes its grants', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var now = 1_750_000_000;
    final gateway = FakePushGateway(nowSeconds: () => now);
    debugAndroidPushAuthority = AndroidPushAuthority(
      keystore: FakeAndroidKeystore(),
      gateway: fakeGatewayClient(gateway),
      store: _MemoryStore(),
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) async {},
    );
    fcmDeviceToken.value = 'fcm-token';
    pushUnavailableReason.value = 'stale';

    final grant = (await enrollBuzzPush(
      'wss://relay.example/',
      'https://gateway.example/',
      relayOrigin: 'wss://relay.example',
      relayPubkey: 'a' * 64,
    ))!;

    expect(grant.relayOrigin, 'wss://relay.example');
    expect(grant.relayPubkey, 'a' * 64);
    expect(pushUnavailableReason.value, isNull);
    expect(pushEndpointGrants.value.single.endpointGrant, grant.endpointGrant);
    expect(
      (await readBuzzPushEndpointGrants()).single.installationId,
      grant.installationId,
    );
  });

  test('a phone that cannot attest reports push as unavailable', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    var now = 1_750_000_000;
    debugAndroidPushAuthority = AndroidPushAuthority(
      keystore: FakeAndroidKeystore()..attests = false,
      gateway: fakeGatewayClient(FakePushGateway(nowSeconds: () => now)),
      store: _MemoryStore(),
      now: () => DateTime.fromMillisecondsSinceEpoch(now * 1000),
      sleep: (_) async {},
    );
    fcmDeviceToken.value = 'fcm-token';

    await expectLater(
      enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway.example/',
        relayOrigin: 'wss://relay.example',
        relayPubkey: 'a' * 64,
      ),
      throwsA(isA<AndroidPushAttestationUnavailable>()),
    );

    expect(pushUnavailableReason.value, isNotNull);
  });

  test('enrolling without a token fails loudly', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;

    await expectLater(
      enrollBuzzPush(
        'wss://relay.example/',
        'https://gateway.example/',
        relayOrigin: 'wss://relay.example',
        relayPubkey: 'a' * 64,
      ),
      throwsStateError,
    );
  });
}

class _MemoryStore implements AndroidPushStateStore {
  String? value;

  @override
  Future<String?> read() async => value;

  @override
  Future<void> write(String? next) async => value = next;
}
