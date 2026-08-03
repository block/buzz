import 'package:buzz/shared/notifications/notifications.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel('xyz.block.buzz.mobile/notifications');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late List<MethodCall> calls;
  late AndroidNotificationBridge bridge;

  setUp(() {
    calls = [];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          calls.add(call);
          return switch (call.method) {
            'getStatus' => <String, Object>{
              'permission': 'granted',
              'priorityChannelEnabled': true,
              'activityChannelEnabled': false,
            },
            'getInitialRoute' => 'buzz://message?channel=channel-1&id=initial',
            _ => null,
          };
        });
    bridge = AndroidNotificationBridge(channel: _channel);
  });

  tearDown(() {
    bridge.dispose();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, null);
  });

  test('maps status and uses the frozen method names', () async {
    final status = await bridge.getStatus();
    final requested = await bridge.requestPermission();
    final ensured = await bridge.ensureChannels();

    expect(status.permission, AndroidNotificationPermission.granted);
    expect(status.priorityChannelEnabled, isTrue);
    expect(status.activityChannelEnabled, isFalse);
    expect(requested.permission, AndroidNotificationPermission.granted);
    expect(ensured.permission, AndroidNotificationPermission.granted);
    expect(calls.map((call) => call.method), [
      'getStatus',
      'requestPermission',
      'getStatus',
      'ensureChannels',
      'getStatus',
    ]);
  });

  test('treats pre-Android-13 permission as granted', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          calls.add(call);
          if (call.method != 'getStatus') return null;
          return <String, Object>{
            'permission': 'notRequired',
            'priorityChannelEnabled': true,
            'activityChannelEnabled': true,
          };
        });

    final status = await bridge.getStatus();

    expect(status.permission, AndroidNotificationPermission.granted);
  });

  test('show sends exactly the frozen argument keys', () async {
    await bridge.show(
      id: 21,
      channel: 'priority',
      title: 'Sats',
      body: 'Ping',
      route: 'buzz://message?channel=channel-1&id=abc',
    );

    expect(calls.single.method, 'show');
    expect(calls.single.arguments, <String, Object>{
      'id': 21,
      'channel': 'priority',
      'title': 'Sats',
      'body': 'Ping',
      'route': 'buzz://message?channel=channel-1&id=abc',
    });
  });

  test('exposes initial and incoming tap routes', () async {
    expect(
      await bridge.getInitialRoute(),
      'buzz://message?channel=channel-1&id=initial',
    );

    final tapped = bridge.notificationTaps.first;
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall(
              'notificationTapped',
              'buzz://message?channel=channel-1&id=tapped',
            ),
          ),
          (_) {},
        );

    expect(
      await tapped.timeout(const Duration(seconds: 1)),
      'buzz://message?channel=channel-1&id=tapped',
    );
  });

  test('rejects non-canonical outgoing notification routes', () {
    expect(
      () => bridge.show(
        id: 21,
        channel: 'priority',
        title: 'Sats',
        body: 'Ping',
        route: 'https://example.com/message?id=abc',
      ),
      throwsArgumentError,
    );
    expect(calls, isEmpty);
  });

  test('drops malformed initial and incoming notification routes', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_channel, (call) async {
          calls.add(call);
          return call.method == 'getInitialRoute'
              ? 'buzz://message?id=missing-channel'
              : null;
        });

    expect(await bridge.getInitialRoute(), isNull);
    var delivered = false;
    final subscription = bridge.notificationTaps.listen(
      (_) => delivered = true,
    );
    addTearDown(subscription.cancel);
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall(
              'notificationTapped',
              'buzz://message?channel=one&id=two&extra=three',
            ),
          ),
          (_) {},
        );
    await Future<void>.delayed(Duration.zero);

    expect(delivered, isFalse);
  });

  test('exposes refreshed native notification status', () async {
    final changed = bridge.statusChanges.first;
    await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .handlePlatformMessage(
          _channel.name,
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('notificationStatusChanged', <String, Object>{
              'permission': 'denied',
              'priorityChannelEnabled': false,
              'activityChannelEnabled': true,
            }),
          ),
          (_) {},
        );

    final status = await changed.timeout(const Duration(seconds: 1));
    expect(status.permission, AndroidNotificationPermission.denied);
    expect(status.priorityChannelEnabled, isFalse);
    expect(status.activityChannelEnabled, isTrue);
  });

  test('opens Android settings through the frozen method', () async {
    await bridge.openSettings();
    expect(calls.single.method, 'openSettings');
    expect(calls.single.arguments, isNull);
  });
}
