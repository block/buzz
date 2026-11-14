// The notifier's own ref and state, used from its library-private extension.
// ignore_for_file: invalid_use_of_protected_member, invalid_use_of_visible_for_testing_member
part of 'channels_provider.dart';

/// Synced thread follow choices in unread admission and re-evaluation.
extension _ChannelThreadFollows on ChannelsNotifier {
  Set<String> _followedRootIds() =>
      ref.read(threadFollowsProvider).followedRootIds;

  Set<String> _unfollowedRootIds() =>
      ref.read(threadFollowsProvider).unfollowedRootIds;

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
  /// retained, so those channels catch up again over their whole window.
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
      events.removeWhere(
        (_, event) =>
            !event.highPriority &&
            event.rootId != null &&
            unfollowed.contains(event.rootId),
      );
    }
    if (widened.isNotEmpty) {
      _unreadCaughtUpByChannel.clear();
      unawaited(_refreshInBackground(queueIfBusy: true));
    }
    state = state.whenData((channels) => List<Channel>.of(channels));
  }
}
