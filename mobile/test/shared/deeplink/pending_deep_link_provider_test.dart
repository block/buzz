import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  setUp(() {
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream.empty();
    pendingPushNotificationLink.value = null;
  });

  tearDown(() {
    PendingDeepLinkNotifier.debugUriStreamOverride = null;
    pendingPushNotificationLink.value = null;
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
