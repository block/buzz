import 'dart:async';

import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  group('PendingDeepLinkNotifier cold-start (issue #5150)', () {
    tearDown(() {
      PendingDeepLinkNotifier.debugUriStreamOverride = null;
      PendingDeepLinkNotifier.debugGetInitialLinkOverride = null;
    });

    test(
      'parks the cold-start URI from AppLinks.getInitialLink() even when '
      'uriLinkStream emits nothing',
      () async {
        // Regression: app_links v6 `uriLinkStream` does NOT replay the cold-
        // start URI to late subscribers — without an explicit
        // `getInitialLink()` call, a fresh iOS install launched from an
        // invite link boots to a blank home with no claim POST (issue #5150).
        PendingDeepLinkNotifier.debugGetInitialLinkOverride = () =>
            Future.value(
              Uri.parse(
                'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=invite-code',
              ),
            );
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();

        final container = ProviderContainer();
        addTearDown(container.dispose);

        // Listen synchronously so the notifier build fires.
        container.listen(pendingDeepLinkProvider, (_, _) {});

        // The initial-link future resolves on a microtask — give it time.
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        expect(
          container.read(pendingDeepLinkProvider),
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'invite-code',
          ),
        );
      },
    );

    test(
      'handles a message deep link from cold start the same way',
      () async {
        PendingDeepLinkNotifier.debugGetInitialLinkOverride = () =>
            Future.value(
              Uri.parse(
                'buzz://message?channel=d14cd131&id=abc123&thread=root99',
              ),
            );
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();

        final container = ProviderContainer();
        addTearDown(container.dispose);
        container.listen(pendingDeepLinkProvider, (_, _) {});
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: 'd14cd131',
            messageId: 'abc123',
            threadRootId: 'root99',
          ),
        );
      },
    );

    test(
      'drops a null initial link silently (no invite launch, normal boot)',
      () async {
        PendingDeepLinkNotifier.debugGetInitialLinkOverride = () =>
            Future<Uri?>.value(null);
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();

        final container = ProviderContainer();
        addTearDown(container.dispose);
        container.listen(pendingDeepLinkProvider, (_, _) {});
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        expect(container.read(pendingDeepLinkProvider), isNull);
      },
    );

    test(
      'hot stream events still dispatch after cold start is consumed',
      () async {
        PendingDeepLinkNotifier.debugGetInitialLinkOverride = () =>
            Future<Uri?>.value(null);
        final controller = StreamController<Uri>();
        addTearDown(() => controller.close());
        PendingDeepLinkNotifier.debugUriStreamOverride = controller.stream;

        final container = ProviderContainer();
        addTearDown(container.dispose);
        container.listen(pendingDeepLinkProvider, (_, _) {});

        controller.add(
          Uri.parse(
            'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=hot-invite',
          ),
        );
        // Flush the controller's broadcast microtask queue.
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        expect(
          container.read(pendingDeepLinkProvider),
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'hot-invite',
          ),
        );
      },
    );
  });
}
