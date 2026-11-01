part of 'channels_provider.dart';

/// Apply the same authenticated relay deletion evidence as the timeline.
/// Keep bounded tombstones so an in-flight history response cannot restore a
/// deleted mention after the live deletion has removed it from unread state.
extension _UnreadDeletions on ChannelsNotifier {
  bool _applyUnreadDeletion(NostrEvent event) {
    if (event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent) {
      return false;
    }
    final channelId = event.channelId;
    if (channelId == null && event.kind != EventKind.deletion) return false;
    // NIP-09 can omit h: the relay scopes the event using its target's stored
    // channel. Event IDs are globally unique, so retain those targets across
    // this community until their messages arrive, without guessing a channel.
    final deleted = channelId == null
        ? _unscopedUnreadDeletedIds
        : _unreadDeletedIdsByChannel.putIfAbsent(channelId, () => {});
    final targets = {
      for (final tag in event.tags)
        if (tag.length >= 2 && tag[0] == 'e') tag[1],
    };
    deleted.addAll(targets);
    while (deleted.length > _unreadCatchUpLimit) {
      deleted.remove(deleted.first);
    }
    final channels = channelId == null
        ? _observedUnreadEventsByChannel.keys.toList()
        : [channelId];
    for (final id in channels) {
      final observed = _observedUnreadEventsByChannel[id];
      observed?.removeWhere((key, _) => targets.contains(key));
      if (observed == null || observed.isEmpty) {
        _observedUnreadEventsByChannel.remove(id);
        _latestObservedByChannel.remove(id);
      } else {
        _latestObservedByChannel[id] = observed.values
            .map((event) => event.createdAt)
            .reduce(max);
      }
    }
    return true;
  }
}
