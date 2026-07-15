import 'dart:async';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'deep_link.dart';

/// Holds the most recent `buzz://message` deep link that has not been
/// dispatched yet.
///
/// Listens to [AppLinks.uriLinkStream], which delivers both the cold-start
/// link (the URL that launched the app) and links received while running.
/// Navigation cannot always happen the moment a link arrives — the user may
/// not be authenticated yet, or channels may still be loading — so the parsed
/// link is parked here and consumed by the dispatcher once the app is ready.
class PendingDeepLinkNotifier extends Notifier<MessageDeepLink?> {
  @visibleForTesting
  static Stream<Uri>? debugUriStreamOverride;

  StreamSubscription<Uri>? _subscription;

  @override
  MessageDeepLink? build() {
    final stream = debugUriStreamOverride ?? AppLinks().uriLinkStream;
    _subscription = stream.listen(handleUri);
    ref.onDispose(() {
      _subscription?.cancel();
      _subscription = null;
    });
    return null;
  }

  /// Parse and park an incoming URI. Unsupported links are ignored loudly.
  @visibleForTesting
  void handleUri(Uri uri) {
    final link = parseMessageDeepLink(uri);
    if (link == null) {
      debugPrint('deep-link: ignoring unsupported link: $uri');
      return;
    }
    state = link;
  }

  /// Clear the pending link after it has been dispatched (or dropped).
  void consume() => state = null;
}

final pendingDeepLinkProvider =
    NotifierProvider<PendingDeepLinkNotifier, MessageDeepLink?>(
      PendingDeepLinkNotifier.new,
    );
