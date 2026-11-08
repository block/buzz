import '../deeplink/deep_link.dart';

/// The unsent first message for a Solo chat started from another message.
///
/// An open channel's message is linked, as desktop's Start Solo chat does: the
/// agent can open it there. A DM, private channel or Solo chat is usually
/// closed to the agent, so the message is excerpted as a block quote instead.
String soloChatSeed({
  required bool linkable,
  required String channelId,
  required String messageId,
  required String? threadRootId,
  required String content,
}) {
  if (linkable) {
    final link = buildMessageLink(
      channelId: channelId,
      messageId: messageId,
      threadRootId: threadRootId,
    );
    return '$link\n\n';
  }
  final quote = content
      .trim()
      .split('\n')
      .map((line) => line.isEmpty ? '>' : '> $line')
      .join('\n');
  return '$quote\n\n';
}

/// One draft per source message: starting again from it returns to the
/// unsent prompt instead of stranding it.
String soloChatSeedDraftKey(String messageId) => 'new-solo:$messageId';
