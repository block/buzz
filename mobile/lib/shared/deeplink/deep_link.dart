/// Parsing for `buzz://` deep links.
///
/// Mirrors the desktop handler in `desktop/src-tauri/src/deep_link.rs`:
/// `buzz://message?channel=<uuid>&id=<hex>[&thread=<hex>]` references a
/// message (optionally inside a thread) in a channel. Required params that
/// are missing or empty make the link invalid — the caller never sees a
/// half-formed target.
library;

/// A parsed `buzz://message` deep link.
class MessageDeepLink {
  /// Channel UUID from the `channel` query param.
  final String channelId;

  /// Event ID (hex) from the `id` query param.
  final String messageId;

  /// Optional thread root event ID from the `thread` query param.
  final String? threadRootId;

  const MessageDeepLink({
    required this.channelId,
    required this.messageId,
    this.threadRootId,
  });

  @override
  bool operator ==(Object other) =>
      other is MessageDeepLink &&
      other.channelId == channelId &&
      other.messageId == messageId &&
      other.threadRootId == threadRootId;

  @override
  int get hashCode => Object.hash(channelId, messageId, threadRootId);

  @override
  String toString() =>
      'MessageDeepLink(channel: $channelId, id: $messageId, '
      'thread: $threadRootId)';
}

/// Parse a `buzz://message?…` URI into a [MessageDeepLink].
///
/// Returns `null` for non-`buzz` schemes, non-`message` hosts (e.g.
/// `buzz://connect` which is desktop-only), or links missing a non-empty
/// `channel` or `id` param.
MessageDeepLink? parseMessageDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || uri.host != 'message') return null;

  final channel = uri.queryParameters['channel'];
  final id = uri.queryParameters['id'];
  if (channel == null || channel.isEmpty || id == null || id.isEmpty) {
    return null;
  }

  final thread = uri.queryParameters['thread'];
  return MessageDeepLink(
    channelId: channel,
    messageId: id,
    threadRootId: (thread == null || thread.isEmpty) ? null : thread,
  );
}
