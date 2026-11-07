import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/share_inbox/share_inbox_provider.dart';
import '../../shared/share_inbox/share_targets.dart';
import 'channel.dart';
import 'channels_provider.dart';
import 'dm_channel_labels.dart';

/// Publishes the active community's channels to the iOS Share Extension so
/// its picker matches the in-app one.
///
/// Publication is debounced (live messages reorder channels often) and only
/// a successful write is remembered, so a failed write is retried on the next
/// change. The state is the latest computed fingerprint; watch this provider
/// with `ref.listen` so its changes do not rebuild the listener.
class ShareTargetsSyncNotifier extends Notifier<String?> {
  static const debounce = Duration(seconds: 2);

  Timer? _timer;
  String? _published;

  @override
  String? build() {
    ref.onDispose(() => _timer?.cancel());
    if (defaultTargetPlatform != TargetPlatform.iOS) return null;
    final community = ref.watch(activeCommunityProvider).value;
    final communities = ref.watch(communityListProvider).value;
    final channels = ref.watch(channelsProvider).value;
    final myPubkey = ref.watch(myPubkeyProvider);
    if (community == null || communities == null || channels == null) {
      return stateOrNull;
    }
    final snapshot = buildShareTargetsSnapshot(
      community: community,
      communities: communities,
      channels: channels,
      currentPubkey: myPubkey,
    );
    final fingerprint = snapshot.fingerprint;
    _timer?.cancel();
    if (fingerprint != _published) {
      _timer = Timer(debounce, () => unawaited(_publish(snapshot)));
    }
    return fingerprint;
  }

  Future<void> _publish(ShareTargetsSnapshot snapshot) async {
    final fingerprint = snapshot.fingerprint;
    try {
      await ref.read(shareTargetsSyncerProvider)(snapshot);
      _published = fingerprint;
    } catch (error) {
      debugPrint('[ShareTargetsSync] publish failed: $error');
    }
  }
}

final shareTargetsSyncProvider =
    NotifierProvider<ShareTargetsSyncNotifier, String?>(
      ShareTargetsSyncNotifier.new,
    );

/// Member, non-archived channels of [community], newest activity first.
ShareTargetsSnapshot buildShareTargetsSnapshot({
  required Community community,
  required List<Community> communities,
  required List<Channel> channels,
  required String? currentPubkey,
}) {
  final targets = [
    for (final channel in channels)
      if (channel.isMember && !channel.isArchived)
        ShareTarget(
          channelId: channel.id,
          label: resolveDmChannelDisplayLabel(
            channel,
            currentPubkey: currentPubkey,
          ),
          isDm: channel.isDm,
          lastMessageAt: channel.lastMessageAt,
        ),
  ];
  targets.sort((a, b) {
    final aTime = a.lastMessageAt?.millisecondsSinceEpoch ?? 0;
    final bTime = b.lastMessageAt?.millisecondsSinceEpoch ?? 0;
    return bTime.compareTo(aTime);
  });
  return ShareTargetsSnapshot(
    communityId: community.id,
    communities: [
      for (final entry in communities)
        ShareTargetCommunity(id: entry.id, name: entry.name),
    ],
    targets: targets,
  );
}
