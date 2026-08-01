import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../features/channels/channel.dart';
import '../../features/channels/channel_mutes/channel_mutes_provider.dart';
import '../../features/channels/channels_provider.dart';
import '../community/community.dart';
import '../community/community_provider.dart';
import '../relay/relay_provider.dart';
import 'push_subscription.dart';

/// Keeps the persisted desired lease and the App Group snapshot aligned with
/// the active identity, joined channels, and mute state. This is desired client
/// policy only. Relay-accepted authority is introduced by lease publication.
final pushSubscriptionSyncProvider = Provider<void>((ref) {
  final active = ref.watch(activeCommunityProvider).value;
  final channels = ref.watch(channelsProvider).value;
  final mutes = ref.watch(channelMutesProvider);
  if (active == null || channels == null || !mutes.isReady) return;

  final subscriptions = desiredBuzzPushSubscriptions(
    community: active,
    channels: channels,
    mutedChannelIds: [
      for (final entry in mutes.store.channels.entries)
        if (entry.value.muted) entry.key,
    ],
  );
  if (subscriptions == null) return;
  unawaited(
    ref
        .read(communityListProvider.notifier)
        .updateDesiredPushSubscriptions(active.id, subscriptions),
  );
});

List<BuzzPushSubscription>? desiredBuzzPushSubscriptions({
  required Community community,
  required Iterable<Channel> channels,
  required Iterable<String> mutedChannelIds,
}) {
  final pubkey = community.pubkey ?? pubkeyFromNsec(community.nsec);
  if (pubkey == null || pubkey.isEmpty) return null;
  return buildDesiredBuzzPushSubscriptions(
    myPubkey: pubkey,
    channelIds: [
      for (final channel in channels)
        if (channel.isMember && !channel.isArchived) channel.id,
    ],
    mutedChannelIds: mutedChannelIds,
  );
}
