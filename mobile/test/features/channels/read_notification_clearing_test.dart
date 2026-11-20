import 'package:buzz/features/channels/read_notification_clearing.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _push = MethodChannel('buzz/push');
final _root = 'a' * 64;

final _community = Community(
  id: 'community',
  name: 'Example',
  relayUrl: 'https://relay.example.org',
  addedAt: DateTime.utc(2026),
);

Map<String, String> _notification(
  String id, {
  String communityId = 'community',
  String channelId = 'dm',
  String? replyRootId,
  required int createdAt,
}) => {
  'id': id,
  'eventId': id.padLeft(64, '0'),
  'communityId': communityId,
  'channelId': channelId,
  'replyRootId': ?replyRootId,
  'messageCreatedAt': '$createdAt',
};

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('delivered notifications clear once read state covers them', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    final delivered = [
      _notification('1', createdAt: 1000),
      _notification('2', createdAt: 2000),
      _notification(
        '3',
        channelId: 'general',
        replyRootId: _root,
        createdAt: 1500,
      ),
      // Another community's read state isn't loaded.
      _notification('4', communityId: 'elsewhere', createdAt: 500),
    ];
    messenger.setMockMethodCallHandler(_push, (call) async {
      switch (call.method) {
        case 'deliveredMessageNotifications':
          return [...delivered];
        case 'removeDeliveredNotifications':
          final ids = (call.arguments as List).cast<String>();
          delivered.removeWhere((n) => ids.contains(n['id']));
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(_push, null);
      debugDefaultTargetPlatformOverride = null;
    });

    final container = ProviderContainer(
      overrides: [
        activeCommunityProvider.overrideWith((ref) async => _community),
        readStateProvider.overrideWith(_ReadState.new),
        appForegroundProvider.overrideWith(_Foreground.new),
      ],
    );
    addTearDown(container.dispose);
    await container.read(activeCommunityProvider.future);
    container.listen(readNotificationClearerProvider, (_, _) {});
    await pumpEventQueue();
    List<String?> ids() => [for (final n in delivered) n['id']];
    expect(ids(), ['1', '2', '3', '4']);

    // Read on another device up to the first message: a newer one stays.
    final readState = container.read(readStateProvider.notifier) as _ReadState;
    readState.sync('dm', 1000);
    await pumpEventQueue();
    expect(ids(), ['2', '3', '4']);

    readState.sync(threadContextKey(_root), 1500);
    await pumpEventQueue();
    expect(ids(), ['2', '4']);
  });
}

class _ReadState extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState(
    isReady: true,
    pubkey: 'me',
    contexts: {},
    version: 1,
  );

  void sync(String contextId, int timestamp) =>
      state = state.copyWithContext(contextId, timestamp);
}

class _Foreground extends AppForegroundNotifier {
  @override
  bool build() => true;
}
