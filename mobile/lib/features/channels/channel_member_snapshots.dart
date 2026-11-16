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
