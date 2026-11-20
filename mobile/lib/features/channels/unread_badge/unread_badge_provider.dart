import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/auth/auth.dart';
import '../../../shared/read_state/read_state_provider.dart';
import '../../activity/activity_provider.dart';
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
  // A feed with a failed source may lack unread items, so it does not count.
  final feedLoaded = ref.watch(
    activityProvider.select(
      (feed) => !feed.isLoading && (feed.asData?.value.complete ?? false),
    ),
  );
  final inbox = ref.watch(inboxUnreadProvider);
  final readState = ref.watch(readStateProvider);
  if (channels == null || !feedLoaded || inbox == InboxUnread.pending) {
    return UnreadBadge.pending;
  }
  if (inbox == InboxUnread.unread) return UnreadBadge.unread;
  return hasForcedUnreadChannel(channels, readState)
      ? UnreadBadge.unread
      : UnreadBadge.clear;
});

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
