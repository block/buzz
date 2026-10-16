import 'package:buzz/shared/deeplink/app_clip_invite_bridge.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  // Same name as `BuzzAppClipInviteBridge` registers in AppDelegate.
  const channel = MethodChannel('buzz/app_clip_invite');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

  tearDown(() => messenger.setMockMethodCallHandler(channel, null));

  test('returns the stored invocation URL from takePendingInvite', () async {
    final calls = <String>[];
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call.method);
      return 'https://relay.example.com/invite/abc';
    });

    expect(
      await takePendingAppClipInvite(),
      Uri.parse('https://relay.example.com/invite/abc'),
    );
    expect(calls, ['takePendingInvite']);
  });

  test('returns null when nothing is parked', () async {
    messenger.setMockMethodCallHandler(channel, (call) async => null);
    expect(await takePendingAppClipInvite(), isNull);
  });

  test('returns null when the platform has no bridge', () async {
    // No handler registered: the channel raises MissingPluginException.
    expect(await takePendingAppClipInvite(), isNull);
  });

  test('swallows a native read failure instead of blocking startup', () async {
    messenger.setMockMethodCallHandler(channel, (call) async {
      throw PlatformException(code: 'read_failed', message: 'corrupt record');
    });
    expect(await takePendingAppClipInvite(), isNull);
  });
}
