import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/auth/auth.dart';
import '../../../shared/read_state/read_state_provider.dart';
import '../../activity/activity_provider.dart';
import '../../activity/feed_item.dart';
import '../../activity/inbox_unread_provider.dart';
import '../channel.dart';
import '../channels_provider.dart';

/// What the app icon should show. Pending leaves the existing icon alone.
enum UnreadBadge {
  pending,
  clear,
  unread;

  /// The platform badge value, or null while the unread state is unknown.
  int? get platformCount => switch (this) {
    UnreadBadge.pending => null,
    UnreadBadge.clear => 0,
    UnreadBadge.unread => 1,
  };
}

/// Activity unread state plus explicit channel and message mark-unread flags.
final unreadBadgeProvider = Provider<UnreadBadge>((ref) {
  // Signed out clears without starting community-scoped providers.
  final auth = ref.watch(authProvider);
  final status = auth.isLoading ? null : auth.asData?.value.status;
  if (status == AuthStatus.unauthenticated) return UnreadBadge.clear;
  if (status != AuthStatus.authenticated) return UnreadBadge.pending;

  final channelsAsync = ref.watch(channelsProvider);
  final channels = channelsAsync.isLoading ? null : channelsAsync.asData?.value;
  final feed = ref.watch(activityProvider.select(_feedState));
  final inbox = ref.watch(inboxUnreadProvider);
  final readState = ref.watch(readStateProvider);
  // The current community's channels and read state must be known first:
  // until then a value can still belong to the previous community.
  if (channels == null || inbox == InboxUnread.pending) {
    return UnreadBadge.pending;
  }
  // Known unread sets the icon even when an Activity source failed. Missing
  // data only holds back a clear.
  if (hasForcedUnreadChannel(channels, readState)) return UnreadBadge.unread;
  // A loading feed can still hold the previous community's rows, so only
  // the events the channel list received count until it loads.
  if (feed == _FeedState.unknown) {
    return ref.watch(observedInboxUnreadProvider) == InboxUnread.unread
        ? UnreadBadge.unread
        : UnreadBadge.pending;
  }
  if (inbox == InboxUnread.unread) return UnreadBadge.unread;
  return feed == _FeedState.complete ? UnreadBadge.clear : UnreadBadge.pending;
});

enum _FeedState { unknown, partial, complete }

/// Whether Activity has loaded, and whether every one of its sources did.
_FeedState _feedState(AsyncValue<HomeFeedResponse> feed) {
  final value = feed.isLoading ? null : feed.asData?.value;
  if (value == null) return _FeedState.unknown;
  return value.complete ? _FeedState.complete : _FeedState.partial;
}

/// Whether a member, non-archived channel has a channel or message unread flag.
bool hasForcedUnreadChannel(
  Iterable<Channel> channels,
  ReadStateState readState,
) {
  final forced = readState.locallyForcedChannelIds;
  if (forced.isEmpty) return false;
  return channels.any(
    (channel) =>
        channel.isMember && !channel.isArchived && forced.contains(channel.id),
  );
}
