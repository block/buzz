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
    if (channelId != null) _removeSavedMessageTargets(channelId, targets);
    deleted.addAll(targets);
    while (deleted.length > _unreadCatchUpLimit) {
      deleted.remove(deleted.first);
    }
    final channels = channelId == null
        ? _observedUnreadEventsByChannel.keys.toList()
        : [channelId];
    for (final id in channels) {
      final observed = _observedUnreadEventsByChannel[id];
      final before = observed?.length;
      observed?.removeWhere((key, _) => targets.contains(key));
      // The deleted event may have been the marker standing in for a full
      // window of quiet messages, which can hide older unread ones. Read the
      // channel's whole window again rather than only since the last read.
      if (observed?.length != before) {
        _unreadCaughtUpByChannel.remove(id);
        _observedUnreadRevisions.touch(id);
      }
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

  /// The posts among [events] that a deletion seen so far removed.
  Set<String> _deletedAmong(List<NostrEvent>? events) => {
    for (final event in events ?? const <NostrEvent>[])
      if (_unscopedUnreadDeletedIds.contains(event.id) ||
          _unreadDeletedIdsByChannel[event.channelId]?.contains(event.id) ==
              true)
        event.id,
  };

  void _applySavedMessageEdit(NostrEvent event, String channelId) {
    if (event.kind != EventKind.streamMessageEdit) return;
    _removeSavedMessageTargets(channelId, {
      for (final tag in event.tags)
        if (tag.length >= 2 && tag[0] == 'e') tag[1],
    }, edited: true);
  }

  void _removeSavedMessageTargets(
    String channelId,
    Set<String> targets, {
    bool edited = false,
  }) {
    if (targets.isEmpty) return;
    final scope = _lifecycleRef.read(savedMessageScopeProvider);
    if (scope == null) return;
    pruneSavedMessageTargets(
      _lifecycleRef,
      scope,
      channelId,
      targets,
      edited: edited,
    );
  }
}
