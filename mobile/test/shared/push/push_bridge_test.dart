import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel('buzz/push');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    apnsDeviceToken.value = null;
    apnsRegistrationError.value = null;
    installBuzzPushMethodHandler();
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
