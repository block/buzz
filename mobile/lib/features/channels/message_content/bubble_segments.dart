import '../message_media.dart';

/// Splits top-level media from text without breaking Markdown blocks.
/// Fenced code, lists and quotes retain their nested content.
List<({String text, bool attachment})> messageBubbleSegments(
  String content,
  Map<String, ImetaEntry> imeta,
) {
  final result = <({String text, bool attachment})>[];
  final lines = <String>[];
  bool? attachment;
  String? fence;
  var nestedBlock = false;
  final embed = RegExp(r'^!?\[[^\]]*\]\((https?://[^)\s]+)\)$');
  void flush() {
    final text = lines.join('\n').trimRight();
    if (text.trim().isNotEmpty) {
      result.add((text: text, attachment: attachment ?? false));
    }
    lines.clear();
  }

  for (final line in content.split('\n')) {
    final trimmed = line.trimLeft();
    final marker = RegExp(r'^(`{3,}|~{3,})').firstMatch(trimmed)?.group(1);
    final inFence = fence != null || marker != null || line.startsWith('    ');
    if (marker != null) {
      if (fence == null) {
        fence = marker;
      } else if (marker[0] == fence[0] && marker.length >= fence.length) {
        fence = null;
      }
    }
    if (trimmed.isEmpty) {
      nestedBlock = false;
    } else if (RegExp(r'^(?:>|[-+*]\s|\d+[.)]\s)').hasMatch(trimmed)) {
      nestedBlock = true;
    }
    // Indented embeds belong to a list, quote or code block. Lazy block
    // continuations also stay together until a blank line ends the block.
    final topLevel = line == trimmed && !nestedBlock;
    final match = inFence || !topLevel
        ? null
        : embed.firstMatch(line.trimRight());
    final isAttachment =
        match != null &&
        (line.trim().startsWith('!') || imeta.containsKey(match.group(1)));
    if (attachment != null &&
        isAttachment != attachment &&
        line.trim().isNotEmpty) {
      flush();
    }
    if (line.trim().isNotEmpty) attachment = isAttachment;
    lines.add(line);
  }
  flush();
  return result;
}
