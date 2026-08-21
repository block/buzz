import 'dart:async';
import 'dart:collection';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'deep_link.dart';

/// Holds supported deep links until they can be dispatched.
///
/// Links are queued in arrival order. Navigation cannot always happen the
/// moment a link arrives — the user may not be authenticated yet, channels may
/// still be loading, or another link may already be dispatching. Keeping a FIFO
/// prevents a later app-link event from silently replacing an earlier one.
///
/// Listens to [AppLinks.uriLinkStream], which delivers both the cold-start link
/// (the URL that launched the app) and links received while running.
class PendingDeepLinkNotifier extends Notifier<BuzzDeepLink?> {
  @visibleForTesting
  static Stream<Uri>? debugUriStreamOverride;

  StreamSubscription<Uri>? _subscription;
  final Queue<BuzzDeepLink> _waiting = Queue<BuzzDeepLink>();

  @override
  BuzzDeepLink? build() {
    _waiting.clear();
    final stream = debugUriStreamOverride ?? AppLinks().uriLinkStream;
    _subscription = stream.listen(open);
    ref.onDispose(() {
      _subscription?.cancel();
      _subscription = null;
    });
    return null;
  }

  /// Parse and park an incoming URI. Unsupported links are ignored loudly.
  void open(Uri uri) {
    final link = parseBuzzDeepLink(uri);
    if (link == null) {
      debugPrint('deep-link: ignoring unsupported link: $uri');
      return;
    }
    if (state == null) {
      state = link;
    } else {
      _waiting.addLast(link);
    }
  }

  /// Acknowledge the current link and expose the next queued link, if any.
  void consume() {
    state = _waiting.isEmpty ? null : _waiting.removeFirst();
  }
}

final pendingDeepLinkProvider =
    NotifierProvider<PendingDeepLinkNotifier, BuzzDeepLink?>(
      PendingDeepLinkNotifier.new,
    );
