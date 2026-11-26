part of 'channels_provider.dart';

extension on ChannelsNotifier {
  void _cacheMemberSnapshots(
    Iterable<NostrEvent> events, {
    bool replaceAll = false,
  }) {
    final latestByChannelId = <String, NostrEvent>{};
    for (final event in events) {
      final channelId = event.getTagValue('d');
      if (channelId == null) continue;
      final current = latestByChannelId[channelId];
      if (current == null || event.createdAt > current.createdAt) {
        latestByChannelId[channelId] = event;
      }
    }

    final snapshots = replaceAll
        ? <String, List<ChannelMember>>{}
        : Map<String, List<ChannelMember>>.of(_memberSnapshotsByChannelId);
    snapshots.addAll({
      for (final entry in latestByChannelId.entries)
        entry.key: List.unmodifiable([
          for (final member in membersFromEvent(entry.value))
            ChannelMember(
              pubkey: member.pubkey,
              role: member.role,
              joinedAt: DateTime.fromMillisecondsSinceEpoch(
                entry.value.createdAt * 1000,
                isUtc: true,
              ),
            ),
        ]),
    });
    _memberSnapshotsByChannelId = Map.unmodifiable(snapshots);
  }
}

/// Membership in private backing rooms, including rooms hidden from the list.
final channelViewerIsMemberProvider = Provider.autoDispose.family<bool, String>(
  (ref, channelId) {
    ref.watch(channelsProvider);
    final pubkey = ref.watch(myPubkeyProvider);
    return pubkey != null &&
        ref
            .read(channelsProvider.notifier)
            .cachedMembersForChannel(channelId)
            .any((member) => member.pubkey == pubkey);
  },
);

/// Null means the room membership has not been learned yet.
final huddleHistoryMembershipProvider = Provider.autoDispose
    .family<bool?, String>((ref, channelId) {
      ref.watch(channelsProvider);
      final pubkey = ref.watch(myPubkeyProvider);
      final notifier = ref.read(channelsProvider.notifier);
      final members = notifier._memberSnapshotsByChannelId[channelId];
      if (pubkey == null || !notifier._hasAuthoritativeMemberships) return null;
      if (members == null) return false;
      return members.any((member) => member.pubkey == pubkey);
    });

/// Portraits for inventory shortcuts reuse the directory's existing roster.
final pinMemberSnapshotProvider = Provider.autoDispose
    .family<List<ChannelMember>, String>((ref, channelId) {
      ref.watch(channelsProvider);
      return ref
          .read(channelsProvider.notifier)
          .cachedMembersForChannel(channelId);
    });
