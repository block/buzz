import 'package:flutter/foundation.dart';

import 'message_media.dart';

/// Image metadata shared by carousel rendering and loading projection.
@immutable
class MessageGalleryItem {
  final String url;
  final String semanticLabel;
  final double? aspectRatio;

  const MessageGalleryItem({
    required this.url,
    required this.semanticLabel,
    required this.aspectRatio,
  });
}

/// A trailing run of image embeds and its preceding message text.
@immutable
class TrailingImageGallery {
  final String content;
  final List<MessageGalleryItem> items;

  const TrailingImageGallery({required this.content, required this.items});
}

/// Groups trailing image lines using the production carousel policy.
TrailingImageGallery? extractTrailingImageGallery(
  String content,
  Map<String, ImetaEntry> imetaByUrl,
) {
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
    final url = match.group(2)!;
    final imeta = imetaByUrl[url];
    final mediaKind = classifyMediaUrl(url, imeta: imeta);
    if (mediaKind != MessageMediaKind.image) {
      break;
    }
    final markdownLabel = match.group(1)?.trim();
    items.add(
      MessageGalleryItem(
        url: url,
        semanticLabel:
            imeta?.alt ??
            (markdownLabel?.isNotEmpty == true
                ? markdownLabel!
                : 'Message image'),
        aspectRatio: imeta?.aspectRatio,
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
