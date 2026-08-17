import 'dart:async';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'deep_link.dart';
import '../push/push_bridge.dart';

/// Holds the most recent supported deep link that has not been
/// dispatched yet.
///
/// Listens to [AppLinks.uriLinkStream], which delivers both the cold-start
/// link (the URL that launched the app) and links received while running.
/// Navigation cannot always happen the moment a link arrives — the user may
/// not be authenticated yet, or channels may still be loading — so the parsed
/// link is parked here and consumed by the dispatcher once the app is ready.
class PendingDeepLinkNotifier extends Notifier<BuzzDeepLink?> {
  @visibleForTesting
  static Stream<Uri>? debugUriStreamOverride;

  StreamSubscription<Uri>? _subscription;
  VoidCallback? _pushNotificationListener;

  @override
  BuzzDeepLink? build() {
    final stream = debugUriStreamOverride ?? AppLinks().uriLinkStream;
    _subscription = stream.listen(handleUri);
    _pushNotificationListener = () {
      final link = pendingPushNotificationLink.value;
      if (link != null) state = link;
    };
    pendingPushNotificationLink.addListener(_pushNotificationListener!);
    ref.onDispose(() {
      _subscription?.cancel();
      _subscription = null;
      if (_pushNotificationListener case final listener?) {
        pendingPushNotificationLink.removeListener(listener);
      }
      _pushNotificationListener = null;
    });
    return pendingPushNotificationLink.value;
  }

  /// Parse and park an incoming URI. Unsupported links are ignored loudly.
  @visibleForTesting
  void handleUri(Uri uri) {
    final link = parseBuzzDeepLink(uri);
    if (link == null) {
      debugPrint('deep-link: ignoring unsupported link: $uri');
      return;
    }
    state = link;
  }

  /// Clear the pending link after it has been dispatched (or dropped).
  void consume() {
    if (pendingPushNotificationLink.value == state) {
      pendingPushNotificationLink.value = null;
    }
    state = null;
  }
}

final pendingDeepLinkProvider =
    NotifierProvider<PendingDeepLinkNotifier, BuzzDeepLink?>(
      PendingDeepLinkNotifier.new,
    );
