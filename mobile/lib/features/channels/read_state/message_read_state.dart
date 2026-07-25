import 'read_state_format.dart';
import 'read_state_provider.dart';

/// Effective read timestamp for a single message: the newest of the channel
/// marker, the message's own `msg:` marker, and (for thread replies) the
/// thread's `thread:` marker — the same precedence the unread badge uses via
/// `observedUnreadEventReadAt`.
int? effectiveMessageReadAt(
  ReadStateState readState, {
  required String channelId,
  required String messageId,
  String? threadRootId,
}) {
  return maxReadAt([
    readState.effectiveTimestamp(channelId),
    readState.effectiveTimestamp(msgContextKey(messageId)),
    if (threadRootId != null)
      readState.effectiveTimestamp(threadContextKey(threadRootId)),
  ]);
}

/// Whether a message should currently show as unread in the actions menu.
bool isMessageUnread(
  ReadStateState readState, {
  required String channelId,
  required String messageId,
  required int createdAt,
  String? threadRootId,
}) {
  if (readState.locallyForcedChannelIds.contains(channelId)) return true;
  final readAt = effectiveMessageReadAt(
    readState,
    channelId: channelId,
    messageId: messageId,
    threadRootId: threadRootId,
  );
  return readAt == null || createdAt > readAt;
}
