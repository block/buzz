import 'dart:async';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import 'share_inbox_bridge.dart';
import 'shared_payload.dart';

final sharedPayloadReaderProvider = Provider<SharedPayloadReader>(
  (ref) => takePendingSharedPayload,
);

final sharedPayloadDiscarderProvider = Provider<SharedPayloadDiscarder>(
  (ref) => discardSharedPayload,
);

final shareTargetsSyncerProvider = Provider<ShareTargetsSyncer>(
  (ref) => syncShareTargets,
);

enum SharedPayloadCommunityPreparation { ready, switched, unavailable, failed }

/// The share-sheet payload waiting for the user to pick a destination.
///
/// The Share Extension stages items in the App Group inbox and asks iOS to
/// open `buzz://share?id=…`. That open is best-effort, so besides reacting to
/// the link the notifier re-checks the inbox every time the app returns to the
/// foreground: a share that could not launch the app is picked up as soon as
/// the user opens Buzz themselves.
class PendingSharedPayloadNotifier extends Notifier<SharedPayload?> {
  Future<void>? _inFlight;

  @override
  SharedPayload? build() {
    // Flutter's own listener, not appLifecycleProvider: that one also starts
    // relay connectivity monitoring, which a share check must not trigger.
    final lifecycle = AppLifecycleListener(onResume: () => unawaited(sync()));
    ref.onDispose(lifecycle.dispose);
    unawaited(sync());
    return null;
  }

  /// Pull the newest staged payload from native, if one exists.
  Future<void> sync() {
    final inFlight = _inFlight;
    if (inFlight != null) return inFlight;
    final run = _sync().whenComplete(() => _inFlight = null);
    _inFlight = run;
    return run;
  }

  Future<void> _sync() async {
    final payload = await ref.read(sharedPayloadReaderProvider)();
    if (!ref.mounted || payload == null) return;
    state = payload;
  }

  /// Release payload [id] once a destination has taken its contents.
  ///
  /// Takes the id rather than acting on [state]: a newer share can replace
  /// [state] while a picker for an older one is still open, and releasing the
  /// newer one here would lose it.
  Future<void> consume(String id) async {
    if (state?.id == id) state = null;
    await ref.read(sharedPayloadDiscarderProvider)(id);
  }

  /// Drop payload [id] without using it (user dismissed the picker).
  Future<void> dismiss(String id) => consume(id);

  /// Makes the community a targeted payload names the active one.
  Future<SharedPayloadCommunityPreparation> prepareCommunity(
    String communityId,
  ) async {
    try {
      final communities = await ref.read(communityListProvider.future);
      if (!communities.any((community) => community.id == communityId)) {
        return SharedPayloadCommunityPreparation.unavailable;
      }
      final active = await ref.read(activeCommunityProvider.future);
      if (active?.id == communityId) {
        return SharedPayloadCommunityPreparation.ready;
      }
      await ref
          .read(communityListProvider.notifier)
          .switchCommunity(communityId);
      return SharedPayloadCommunityPreparation.switched;
    } catch (error) {
      debugPrint(
        'share-inbox: failed to switch to community $communityId: $error',
      );
      return SharedPayloadCommunityPreparation.failed;
    }
  }
}

final pendingSharedPayloadProvider =
    NotifierProvider<PendingSharedPayloadNotifier, SharedPayload?>(
      PendingSharedPayloadNotifier.new,
    );
