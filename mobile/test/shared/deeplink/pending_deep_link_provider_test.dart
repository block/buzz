import 'package:buzz/shared/deeplink/app_clip_invite_bridge.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  // The notifier probes the App Clip method channel at build time.
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream.empty();
    pendingPushNotificationLink.value = null;
  });

  tearDown(() {
    PendingDeepLinkNotifier.debugUriStreamOverride = null;
    pendingPushNotificationLink.value = null;
  });

  test('unsupported callback does not log its code or state', () {
    final messages = <String?>[];
    final previous = debugPrint;
    debugPrint = (message, {wrapWidth}) => messages.add(message);
    addTearDown(() => debugPrint = previous);
    final container = ProviderContainer();
    addTearDown(container.dispose);
    container
        .read(pendingDeepLinkProvider.notifier)
        .open(Uri.parse('buzz://auth?code=secret-code&state=secret-state'));
    expect(container.read(pendingDeepLinkProvider), isNull);
    expect(messages.join(), isNot(contains('secret-code')));
    expect(messages.join(), isNot(contains('secret-state')));
    expect(messages, isNotEmpty);
  });

  test('parks and consumes a native notification message link', () async {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    expect(container.read(pendingDeepLinkProvider), isNull);

    const link = MessageDeepLink(
      communityId: 'community-id',
      channelId: 'channel-id',
      messageId: 'event-id',
    );
    pendingPushNotificationLink.value = link;
    await pumpEventQueue();

    expect(container.read(pendingDeepLinkProvider), link);
    container.read(pendingDeepLinkProvider.notifier).consume();
    expect(container.read(pendingDeepLinkProvider), isNull);
    expect(pendingPushNotificationLink.value, isNull);
  });

  test(
    'parks the invite the App Clip stored, taking it exactly once',
    () async {
      var takes = 0;
      final container = ProviderContainer(
        overrides: [
          appClipInvitePickupProvider.overrideWithValue(() async {
            takes++;
            return Uri.parse('https://relay.example.com/invite/clip-code');
          }),
        ],
      );
      addTearDown(container.dispose);

      expect(container.read(pendingDeepLinkProvider), isNull);
      await pumpEventQueue();

      expect(
        container.read(pendingDeepLinkProvider),
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'clip-code',
        ),
      );
      expect(takes, 1);
      container.read(pendingDeepLinkProvider.notifier).consume();
      expect(container.read(pendingDeepLinkProvider), isNull);
    },
  );

  test('ignores an App Clip record that is not a supported link', () async {
    final container = ProviderContainer(
      overrides: [
        appClipInvitePickupProvider.overrideWithValue(
          () async => Uri.parse('https://relay.example.com/not-an-invite'),
        ),
      ],
    );
    addTearDown(container.dispose);
    await pumpEventQueue();
    expect(container.read(pendingDeepLinkProvider), isNull);
  });

  test(
    'App Clip pickup does not displace an earlier cold-start link',
    () async {
      const pushLink = MessageDeepLink(
        communityId: 'community-id',
        channelId: 'channel-id',
        messageId: 'event-id',
      );
      pendingPushNotificationLink.value = pushLink;
      final container = ProviderContainer(
        overrides: [
          appClipInvitePickupProvider.overrideWithValue(
            () async => Uri.parse('https://relay.example.com/invite/clip-code'),
          ),
        ],
      );
      addTearDown(container.dispose);
      // The first read builds the notifier, which seeds the push link as the
      // current state and starts the asynchronous App Clip pickup.
      expect(container.read(pendingDeepLinkProvider), pushLink);
      await pumpEventQueue();

      expect(container.read(pendingDeepLinkProvider), pushLink);
      container.read(pendingDeepLinkProvider.notifier).consume();
      expect(
        container.read(pendingDeepLinkProvider),
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'clip-code',
        ),
      );
    },
  );

  test('preserves a cold-start target present before provider build', () {
    const link = MessageDeepLink(
      communityId: 'community-id',
      channelId: 'channel-id',
      messageId: 'event-id',
    );
    pendingPushNotificationLink.value = link;
    final container = ProviderContainer();
    addTearDown(container.dispose);

    expect(container.read(pendingDeepLinkProvider), link);
  });
}
