part of '../message_content.dart';

// Only standalone media leaves the text bubble. Fenced examples remain text,
// and consecutive image lines stay together for the existing gallery renderer.
List<({String text, bool attachment})> _bubbleSegments(
  String content,
  Map<String, ImetaEntry> imeta,
) {
  final result = <({String text, bool attachment})>[];
  final lines = <String>[];
  bool? attachment;
  String? fence;
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
    final match = inFence ? null : embed.firstMatch(line.trim());
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
