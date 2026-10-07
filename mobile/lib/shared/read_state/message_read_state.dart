import 'read_state_format.dart';
import 'read_state_provider.dart';

/// Effective read timestamp for a single message: the newest of the channel
/// marker, the message's own `msg:` marker, for thread replies the thread's
/// `thread:` and `thread-activity:` markers, and for messages that channel
/// catch-up reads (see `readByChannelCatchUp`) the `activity:` marker — the
/// same precedence the unread badge uses via `observedUnreadEventReadAt`.
int? effectiveMessageReadAt(
  ReadStateState readState, {
  required String channelId,
  required String messageId,
  String? threadRootId,
  bool channelCatchUp = false,
}) {
  return maxReadAt([
    readState.effectiveTimestamp(channelId),
    readState.effectiveTimestamp(msgContextKey(messageId)),
    if (threadRootId != null) ...[
      readState.effectiveTimestamp(threadContextKey(threadRootId)),
      readState.effectiveTimestamp(threadActivityContextKey(threadRootId)),
    ],
    if (channelCatchUp)
      readState.effectiveTimestamp(activityContextKey(channelId)),
  ]);
}

/// Whether a message should currently show as unread in the actions menu.
///
/// A message-level forced unread (from this menu) wins; otherwise the
/// timestamp precedence decides. A channel-level forced unread (from the
/// channel tile) is deliberately not consulted: it is a channel-scoped
/// choice, and letting it leak in here would make the message-level toggle
/// unable to round-trip without clearing the channel-level choice.
bool isMessageUnread(
  ReadStateState readState, {
  required String channelId,
  required String messageId,
  required int createdAt,
  String? threadRootId,
  bool channelCatchUp = false,
}) {
  if (readState.isForcedUnread(msgContextKey(messageId))) return true;
  final readAt = effectiveMessageReadAt(
    readState,
    channelId: channelId,
    messageId: messageId,
    threadRootId: threadRootId,
    channelCatchUp: channelCatchUp,
  );
  return readAt == null || createdAt > readAt;
}
