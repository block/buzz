// The notifier's own ref and state, used from its library-private extension.
// ignore_for_file: invalid_use_of_protected_member, invalid_use_of_visible_for_testing_member
part of 'channels_provider.dart';

/// Synced thread follow choices in unread admission and re-evaluation.
extension _ChannelThreadFollows on ChannelsNotifier {
  Set<String> _followedRootIds() =>
      ref.read(threadFollowsProvider).followedRootIds;

  // An unfollow never mutes a DM, whose replies are read like messages.
  Set<String> _unfollowedRootIds(Channel channel) => channel.isDm
      ? const {}
      : ref.read(threadFollowsProvider).unfollowedRootIds;

  /// Records the user's own thread interest and the thread's channel.
  void _noteThreadEvent(NostrEvent event, String channelId, String? myPk) {
    if (myPk != null &&
        event.pubkey.toLowerCase() == myPk.toLowerCase() &&
        _addSelfThreadInterest(event)) {
      _writeThreadInterestStores(myPk);
    }
    _noteThreadChannel(event, channelId);
  }

  void _noteThreadChannel(NostrEvent event, String channelId) {
    if (event.threadReference.rootId case final rootId?) {
      ref
          .read(threadFollowsProvider.notifier)
          .noteThreadChannel(rootId, channelId);
    }
  }

  /// Re-evaluates retained unread state when follow choices change. It
  /// starts once the channel list has loaded, so it never builds read state
  /// ahead of the community it belongs to.
  void _listenThreadFollows() {
    ref.listen(threadFollowsProvider, _onThreadFollowsChanged);
  }

  /// Re-evaluates retained unread evidence after merged follow choices
  /// change, from this device or another. A newly unfollowed thread drops its
  /// ordinary replies; mentions, broadcasts and DMs still count. A thread
  /// newly followed, or back to automatic, can count replies that were never
  /// retained, so those threads' channels catch up again over their whole
  /// window.
  /// No read frontier moves either way.
  void _onThreadFollowsChanged(
    ThreadFollowsState? previous,
    ThreadFollowsState next,
  ) {
    final unfollowed = next.unfollowedRootIds.difference(
      previous?.unfollowedRootIds ?? const {},
    );
    final widened = {
      ...next.followedRootIds.difference(previous?.followedRootIds ?? const {}),
      ...(previous?.unfollowedRootIds ?? const <String>{}).difference(
        next.unfollowedRootIds,
      ),
    };
    if (unfollowed.isEmpty && widened.isEmpty) return;
    final channels = state.value ?? const <Channel>[];
    final dmIds = {
      for (final channel in channels)
        if (channel.isDm) channel.id,
    };
    for (final MapEntry(key: channelId, value: events)
        in _observedUnreadEventsByChannel.entries) {
      if (dmIds.contains(channelId)) continue;
      final before = events.length;
      events.removeWhere(
        (_, event) =>
            !event.highPriority &&
            event.rootId != null &&
            unfollowed.contains(event.rootId),
      );
      if (events.length != before) _observedUnreadRevisions.touch(channelId);
    }
    if (widened.isNotEmpty) {
      // Only the widened threads' channels catch up again; a root whose
      // channel is unknown could be in any of them.
      final widenedChannels = {
        for (final rootId in widened)
          next.channelByRoot[rootId] ?? previous?.channelByRoot[rootId],
      };
      if (widenedChannels.contains(null)) {
        _unreadCaughtUpByChannel.clear();
      } else {
        widenedChannels.forEach(_unreadCaughtUpByChannel.remove);
      }
      unawaited(_refreshInBackground(queueIfBusy: true));
    }
    state = state.whenData((channels) => List<Channel>.of(channels));
  }
}
