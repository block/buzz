import 'dart:convert';

import '../deeplink/deep_link.dart';

/// Largest quoted excerpt, in UTF-8 bytes. A first Solo message may be 8 KB,
/// so this leaves room for the prompt the user adds below the quote.
const soloChatQuoteMaxBytes = 6144;

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
  return '${_capQuote(quote)}\n\n';
}

String _capQuote(String quote) {
  if (utf8.encode(quote).length <= soloChatQuoteMaxBytes) return quote;
  const ellipsis = '…';
  final budget = soloChatQuoteMaxBytes - utf8.encode(ellipsis).length;
  final kept = StringBuffer();
  var bytes = 0;
  for (final rune in quote.runes) {
    final char = String.fromCharCode(rune);
    bytes += utf8.encode(char).length;
    if (bytes > budget) break;
    kept.write(char);
  }
  return '${kept.toString().trimRight()}$ellipsis';
}

/// One draft per source message: starting again from it returns to the
/// unsent prompt instead of stranding it.
String soloChatSeedDraftKey(String messageId) => 'new-solo:$messageId';
