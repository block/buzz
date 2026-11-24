import '../../features/channels/message_content/link_normalizer.dart';
import '../../features/channels/message_content/preview_text.dart';
import '../../features/channels/message_media.dart';

/// Desktop-compatible starting title for a Solo chat's first message, as it
/// is stored: the literal start of the message, Markdown included.
///
/// Desktop names a chat with its agent harness's title only while the stored
/// name still equals the title desktop derives from that first message, so
/// this must keep matching desktop's rule. [readableMeChatTitle] is what the
/// app shows for it.
String startingMeChatTitle(String message) {
  var body = message.trim();
  // Source links are context rather than a useful chat title.
  while (body.toLowerCase().startsWith('buzz://')) {
    final end = body.indexOf(RegExp(r'\s'));
    body = end < 0 ? '' : body.substring(end).trimLeft();
  }
  // A quoted source is context too; title from the prompt after it, else
  // from the quote itself.
  final lines = body.split('\n');
  final prompt = lines.skipWhile((line) => line.startsWith('>')).join('\n');
  body = prompt.trim().isNotEmpty
      ? prompt.trim()
      : lines.map((line) => line.replaceFirst(RegExp(r'^>\s?'), '')).join('\n');
  final title = String.fromCharCodes(
    body.replaceAll(RegExp(r'\s+'), ' ').runes.take(80),
  ).trim().replaceFirst(RegExp(r'^[#\s]+'), '').trim();
  return title.isEmpty ? 'Chat' : title;
}

/// The title to show for a Solo chat whose stored [name] may still be its
/// starting title: the literal first 80 characters of its first message.
///
/// A message's attachments and links are Markdown, so a chat started with
/// only a video is stored as `![video](https://…`, cut off inside the URL.
/// This reads such a name as the Inbox reads a message preview, and words it
/// as desktop titles the same message:
///
/// - text keeps its words: a link shows its label, and media beside it is
///   dropped. A web address written among other words stays as written;
/// - a name that is only media is `Image`, `Video` or `Audio`, or a count
///   such as `2 images`, `2 videos` or `3 attachments`;
/// - a name that is only web addresses is `Link to example.com`, or a count
///   such as `2 links`.
///
/// Any other name is returned as it is.
String readableMeChatTitle(String name) {
  if (!name.contains('](') && !name.contains('![')) {
    return _linkTitle(name) ?? name;
  }
  // The 80-character cut can land inside a trailing link. An image cut before
  // its destination says nothing; a cut destination still names its kind.
  var source = name.replaceFirst(_cutImageLabel, '');
  if (_cutDestination.hasMatch(source)) {
    source = source.endsWith('(') ? '$source#)' : '$source)';
  }
  final text = StringBuffer();
  final media = <MessageMediaKind>[];
  for (final piece in messagePreviewPieces(
    normalizeBareLinks(source),
    maxLines: 1,
  )) {
    switch (piece) {
      case PreviewText(text: final shown):
        text.write(shown);
      case PreviewMention(:final semanticsLabel):
        text.write('@$semanticsLabel');
      case PreviewChannel(name: final channel):
        text.write('#$channel');
      case PreviewEmoji(:final shortcode):
        text.write(':$shortcode:');
      case PreviewMedia(:final kind):
        media.add(kind);
        text.write(' ');
    }
  }
  final title = text.toString().replaceAll(RegExp(r'\s+'), ' ').trim();
  final links = _linkTitle(title);
  if (links != null) return links;
  if (title.isNotEmpty) return title;
  if (media.isEmpty) return name;
  if (media.length == 1) return previewMediaLabel(media.single);
  final kinds = media.toSet();
  return switch (kinds.length == 1 ? kinds.single : null) {
    MessageMediaKind.image => '${media.length} images',
    MessageMediaKind.video => '${media.length} videos',
    _ => '${media.length} attachments',
  };
}

final _cutImageLabel = RegExp(r'!\[[^\]]*\]?$');
final _cutDestination = RegExp(r'\]\(<?[^)\s>]*$');
final _webLink = RegExp(r'^<?(https?://[^\s<>]+)>?$', caseSensitive: false);

/// A title for [text] that is only web addresses, which read as noise: the
/// site of one, or how many. Null when the text says anything else.
String? _linkTitle(String text) {
  if (!text.contains('://')) return null;
  final words = text.split(RegExp(r'\s+')).where((word) => word.isNotEmpty);
  final links = [for (final word in words) _webLink.firstMatch(word)?[1]];
  if (links.isEmpty || links.contains(null)) return null;
  if (links.length > 1) return '${links.length} links';
  final site = Uri.tryParse(links.single!)?.host ?? '';
  return site.isEmpty
      ? 'Link'
      : 'Link to ${site.replaceFirst(RegExp(r'^www\.'), '')}';
}
