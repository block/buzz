import 'dart:async';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'deep_link.dart';

/// Holds the most recent supported deep link that has not been
/// dispatched yet.
///
/// Subscribes to two sources:
/// - [AppLinks.getInitialLink], which resolves the URI the OS delivered to
///   the app at launch (cold start). app_links' [AppLinks.uriLinkStream] does
///   **not** replay the cold-start URI to late subscribers, so without this
///   call a fresh app launched from an invite link would silently drop it.
/// - [AppLinks.uriLinkStream], which delivers URIs received while the app is
///   already running.
///
/// Navigation cannot always happen the moment a link arrives — the user may
/// not be authenticated yet, or channels may still be loading — so the parsed
/// link is parked here and consumed by the dispatcher once the app is ready.
class PendingDeepLinkNotifier extends Notifier<BuzzDeepLink?> {
  @visibleForTesting
  static Stream<Uri>? debugUriStreamOverride;

  /// Optional override used in tests to inject a cold-start URI without
  /// going through the platform plugin.
  @visibleForTesting
  static Future<Uri?> Function()? debugGetInitialLinkOverride;

  StreamSubscription<Uri>? _subscription;

  @override
  BuzzDeepLink? build() {
    final appLinks = AppLinks();
    // Cold-start URI: resolved once, before any hot-link events. This must
    // not be in a Future kept by the provider because Notifier.build is
    // synchronous — we fire it in a microtask and park the result via
    // handleUri so the rest of the pipeline (listen, consume, dispatch) is
    // identical to runtime-delivered links.
    final initialLinkFuture =
        debugGetInitialLinkOverride?.call() ?? appLinks.getInitialLink();
    unawaited(
      initialLinkFuture
          .then((uri) {
            if (uri != null) handleUri(uri);
          })
          .catchError((Object error) {
            debugPrint('deep-link: failed to read initial link: $error');
          }),
    );
    final stream = debugUriStreamOverride ?? appLinks.uriLinkStream;
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
    final link = parseBuzzDeepLink(uri);
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
    NotifierProvider<PendingDeepLinkNotifier, BuzzDeepLink?>(
      PendingDeepLinkNotifier.new,
    );
