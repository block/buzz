import 'package:flutter/foundation.dart';

import 'message_media.dart';
import 'message_content/link_normalizer.dart';
import 'message_content/markdown_escapes.dart';

/// Attachment metadata shared by carousel and stack rendering.
@immutable
class MessageGalleryItem {
  final String url;
  final String semanticLabel;
  final double? aspectRatio;

  /// Declared media type, used when selecting the native gallery.
  final String? mimeType;

  /// Whether this item opens video playback rather than an image decoder.
  final bool isVideo;

  /// Optional video poster; absent posters use a static video placeholder.
  final String? posterUrl;

  const MessageGalleryItem({
    required this.url,
    required this.semanticLabel,
    required this.aspectRatio,
    this.mimeType,
    this.isVideo = false,
    this.posterUrl,
  });
}

/// A trailing run of media embeds and its preceding message text.
@immutable
class TrailingImageGallery {
  final String content;
  final List<MessageGalleryItem> items;

  const TrailingImageGallery({required this.content, required this.items});
}

/// Groups trailing images, optionally including videos for message stacks.
TrailingImageGallery? extractTrailingImageGallery(
  String content,
  Map<String, ImetaEntry> imetaByUrl, {
  bool includeVideos = false,
}) {
  final lines = content.split('\n');
  var cursor = lines.length - 1;
  while (cursor >= 0 && lines[cursor].trim().isEmpty) {
    cursor -= 1;
  }

  final items = <MessageGalleryItem>[];
  final imagePattern = RegExp(r'^!\[([^\]]*)\]\((https?://[^)\s]+)\)$');
  while (cursor >= 0) {
    final match = imagePattern.firstMatch(lines[cursor].trim());
    if (match == null) break;
    // CommonMark unescapes link destinations, as it does labels below.
    final url = unshieldMarkdownEscapes(shieldMarkdownEscapes(match.group(2)!));
    final imeta = imetaByUrl[url];
    final mediaKind = classifyMediaUrl(url, imeta: imeta);
    if (mediaKind != MessageMediaKind.image &&
        !(includeVideos && mediaKind == MessageMediaKind.video)) {
      break;
    }
    final markdownLabel = decodeMarkdownLabelSyntax(
      unshieldMarkdownEscapes(shieldMarkdownEscapes(match.group(1)!)),
    ).trim();
    items.add(
      MessageGalleryItem(
        url: url,
        semanticLabel:
            imeta?.alt ??
            (markdownLabel.isNotEmpty
                ? markdownLabel
                : mediaKind == MessageMediaKind.video
                ? 'Video'
                : 'Message image'),
        aspectRatio: imeta?.aspectRatio,
        mimeType: imeta?.mimeType,
        isVideo: mediaKind == MessageMediaKind.video,
        posterUrl: imeta?.posterUrl,
      ),
    );
    cursor -= 1;
  }

  if (items.length < 2) return null;
  return TrailingImageGallery(
    content: lines.take(cursor + 1).join('\n').trimRight(),
    items: items.reversed.toList(),
  );
}
