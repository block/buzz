import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../features/channels/channel.dart';
import '../../features/channels/channel_mutes/channel_mutes_provider.dart';
import '../../features/channels/channels_provider.dart';
import '../auth/auth.dart';
import '../me/me_sync.dart';
import '../relay/relay_provider.dart';
import 'dev_push_lease.dart';
import 'push_subscription.dart';
import 'push_relay_capability_provider.dart';
import 'push_setup_recovery.dart';

/// Keeps the persisted desired lease and the App Group snapshot aligned with
/// the active identity, joined channels, and mute state. This is desired client
/// policy only. Relay-accepted authority is introduced by lease publication.
///
/// Yields how many Solo chats the lease had no room for.
final pushSubscriptionSyncProvider = FutureProvider.autoDispose<int>((
  ref,
) async {
  try {
    if (ref.watch(authProvider).value?.status != AuthStatus.authenticated) {
      return 0;
    }
    final active = ref.watch(activeCommunityProvider).value;
    final descriptor = ref.watch(currentRelayPushDescriptorProvider).value;
    if (active == null ||
        !active.pushNotificationsEnabled ||
        descriptor == null) {
      return 0;
    }
    final channels = ref.watch(channelsProvider).value;
    final mutes = ref.watch(channelMutesProvider);
    if (channels == null || !mutes.isReady) {
      return 0;
    }

    // Legacy Solo chats are listed only in the Solo record. Wait while it
    // loads so an unresolved record never drops them. If it fails without a
    // cached copy, keep DMs and mentions current; the Solo tab cannot list
    // those chats either, and recovery republishes them.
    final soloRecord = ref.watch(meGroupsProvider);
    if (soloRecord.isLoading && !soloRecord.hasValue) return 0;

    final desired = desiredBuzzPushSubscriptions(
      community: active,
      descriptor: descriptor,
      channels: channels,
      legacySoloChannelIds: soloRecord.value?.channels ?? const [],
      mutedChannelIds: [
        for (final entry in mutes.store.channels.entries)
          if (entry.value.muted) entry.key,
      ],
    );
    if (desired == null) return 0;
    await ref
        .read(communityListProvider.notifier)
        .updateDesiredPushSubscriptions(active.id, desired.subscriptions);
    return desired.omittedSoloChats;
  } catch (error, stack) {
    reportPushSetupError('subscription synchronization', error, stack);
    rethrow;
  }
}, retry: pushSetupRetry);

/// The desired lease, and how many Solo chats it had no room for.
({List<BuzzPushSubscription> subscriptions, int omittedSoloChats})?
desiredBuzzPushSubscriptions({
  required Community community,
  required BuzzPushLeaseDescriptor descriptor,
  required Iterable<Channel> channels,
  required Iterable<String> mutedChannelIds,
  Iterable<String> legacySoloChannelIds = const [],
}) {
  final pubkey = community.pubkey ?? pubkeyFromNsec(community.nsec);
  if (pubkey == null || pubkey.isEmpty) return null;
  final muted = {for (final id in mutedChannelIds) id.toLowerCase()};
  final candidates = _channelWideCandidates(
    pubkey: pubkey,
    channels: channels,
    muted: muted,
    legacySoloChannelIds: legacySoloChannelIds,
  );
  List<BuzzPushSubscription> lease(int soloCount) =>
      buildDesiredBuzzPushSubscriptions(
        myPubkey: pubkey,
        channelIds: [...candidates.dms, ...candidates.solos.take(soloCount)],
        mutedChannelIds: mutedChannelIds,
      );

  // Each Solo chat only adds bytes, so keep the most recent ones that fit.
  var fitting = candidates.solos.length;
  if (!buzzPushSubscriptionsFitLease(descriptor, lease(fitting))) {
    // Without Solo chats only DMs and mentions remain. A relay too small for
    // those cannot carry push, so fail setup rather than save a lease that
    // publication would reject.
    if (!buzzPushSubscriptionsFitLease(descriptor, lease(0))) {
      throw const FormatException(
        'The relay push limits are too small for a lease.',
      );
    }
    var low = 0;
    var high = fitting - 1;
    while (low < high) {
      final mid = (low + high + 1) ~/ 2;
      if (buzzPushSubscriptionsFitLease(descriptor, lease(mid))) {
        low = mid;
      } else {
        high = mid - 1;
      }
    }
    fitting = low;
  }
  return (
    subscriptions: lease(fitting),
    omittedSoloChats: candidates.omitted + candidates.solos.length - fitting,
  );
}

/// Channel-wide traffic pushes only for joined DMs and the viewer's own Solo
/// chats, which are conversations with them. Other channels push through an
/// exact #p mention, so subscribing to every joined channel would over-notify.
///
/// DMs always stay, as before. Solo chats follow most recent first, within the
/// lease's channel counts; the rest push only on mention.
({List<String> dms, List<String> solos, int omitted}) _channelWideCandidates({
  required String pubkey,
  required Iterable<Channel> channels,
  required Set<String> muted,
  required Iterable<String> legacySoloChannelIds,
}) {
  bool isMuted(String id) => muted.contains(id.toLowerCase());
  final joined = [
    for (final channel in channels)
      if (channel.isMember && !channel.isArchived) channel,
  ];
  final dms = [
    for (final channel in joined)
      if (channel.isDm) channel.id,
  ];
  // Muted Solo chats stay out, like muted channels, so they push only on
  // mention.
  final ordered = [
    for (final channel in joined)
      if (!channel.isDm &&
          !isMuted(channel.id) &&
          channel.isPersonalChat(pubkey, legacySoloChannelIds))
        channel,
  ]..sort(_mostRecentFirst);
  final room =
      buzzPushMaxChannelWideChannels - dms.where((id) => !isMuted(id)).length;
  final solos = [for (final solo in ordered.take(room < 0 ? 0 : room)) solo.id];
  return (dms: dms, solos: solos, omitted: ordered.length - solos.length);
}

int _mostRecentFirst(Channel a, Channel b) {
  final byRecency = (b.lastMessageAt ?? b.createdAt).compareTo(
    a.lastMessageAt ?? a.createdAt,
  );
  return byRecency != 0 ? byRecency : a.id.compareTo(b.id);
}
