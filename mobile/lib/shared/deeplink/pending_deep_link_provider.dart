import 'dart:async';
import 'dart:collection';

import 'package:app_links/app_links.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'deep_link.dart';
import '../community/community_provider.dart';
import '../push/push_bridge.dart';

enum DeepLinkCommunityPreparation { ready, switched, unavailable, failed }

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
  VoidCallback? _pushNotificationListener;

  @override
  BuzzDeepLink? build() {
    _waiting.clear();
    final stream = debugUriStreamOverride ?? AppLinks().uriLinkStream;
    _subscription = stream.listen(open);
    _pushNotificationListener = () {
      final link = pendingPushNotificationLink.value;
      if (link != null) _enqueue(link);
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
  void open(Uri uri) {
    final link = parseBuzzDeepLink(uri);
    if (link == null) {
      debugPrint('deep-link: ignoring unsupported link: $uri');
      return;
    }
    _enqueue(link);
  }

  /// Acknowledge the current link and expose the next queued link, if any.
  void consume() {
    if (pendingPushNotificationLink.value == state) {
      pendingPushNotificationLink.value = null;
    }
    state = _waiting.isEmpty ? null : _waiting.removeFirst();
  }

  /// Selects the device-local community carried by a structured push target.
  /// Ordinary shared deep links have no community ID and remain unchanged.
  Future<DeepLinkCommunityPreparation> prepareCommunity(
    BuzzDeepLink link,
  ) async {
    if (link is! MessageDeepLink || link.communityId == null) {
      return DeepLinkCommunityPreparation.ready;
    }
    final communityId = link.communityId!;
    try {
      final communities = await ref.read(communityListProvider.future);
      if (!communities.any((community) => community.id == communityId)) {
        return DeepLinkCommunityPreparation.unavailable;
      }
      final active = await ref.read(activeCommunityProvider.future);
      if (active?.id == communityId) return DeepLinkCommunityPreparation.ready;
      await ref
          .read(communityListProvider.notifier)
          .switchCommunity(communityId);
      return DeepLinkCommunityPreparation.switched;
    } catch (error) {
      debugPrint(
        'notification-routing: failed to switch to community '
        '$communityId: $error',
      );
      return DeepLinkCommunityPreparation.failed;
    }
  }

  void _enqueue(BuzzDeepLink link) {
    if (state == null) {
      state = link;
    } else {
      _waiting.addLast(link);
    }
  }
}

final pendingDeepLinkProvider =
    NotifierProvider<PendingDeepLinkNotifier, BuzzDeepLink?>(
      PendingDeepLinkNotifier.new,
    );
