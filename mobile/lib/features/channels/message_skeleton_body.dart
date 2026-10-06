import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/skeleton.dart';
import 'message_media.dart';
import 'message_gallery.dart';
import 'message_gallery_frame.dart';
import 'message_media_geometry.dart';

/// Loading shapes derived from a known message, without fetching its media.
class MessageSkeletonBody extends StatelessWidget {
  final String content;
  final List<List<String>> tags;

  const MessageSkeletonBody({
    required this.content,
    required this.tags,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    // A skeleton is a bounded approximation, not a second full renderer.
    // Limit both inspected input and emitted widgets, even for relay-sized posts.
    const maxCharacters = 8192;
    // Never tokenize a truncated Markdown prefix: its missing closing delimiter
    // could turn code into an embed. Oversized messages fail closed in O(1).
    if (content.length > maxCharacters) {
      return const SkeletonBar(
        key: ValueKey('message-skeleton-overflow'),
        width: 72,
        height: 16,
      );
    }
    final metadata = parseImetaTags(
      tags
          .take(64)
          .map(
            (tag) => tag
                .take(16)
                .map((part) => part.substring(0, math.min(part.length, 2048)))
                .toList(),
          )
          .toList(),
    );
    final gallery = extractTrailingImageGallery(content, metadata);
    final source = gallery?.content ?? content;
    final maxAttachments = gallery == null ? 4 : 3;
    final attachments = <String>[];
    final caption = StringBuffer();
    var cursor = 0;
    var overflow = false;
    // Consume code before embeds so media-looking text in code stays text.
    // Ordinary links remain inline, except metadata-backed audio links, as in
    // MessageContent's linkBuilder. One traversal replaces per-URL rewrites.
    final tokens = RegExp(
      r'```[\s\S]*?(?:```|$)|`[^`\n]*`|(!?)\[([^\]\n]*)\]\((https?://[^\s)]+)\)|https?://[^\s)<>]+',
    );
    for (final match in tokens.allMatches(source)) {
      final token = match.group(0)!;
      if (token.startsWith('`')) continue;
      final url = match.group(3) ?? token;
      final meta = metadata[url];
      final isEmbed = match.group(1) == '!';
      final isAudioLink =
          meta != null &&
          classifyMediaUrl(url, imeta: meta) == MessageMediaKind.audio;
      if (!isEmbed && !isAudioLink) continue;
      if (attachments.length == maxAttachments) {
        overflow = true;
        break;
      }
      caption.write(source.substring(cursor, match.start));
      cursor = match.end;
      attachments.add(url);
    }
    // No need to inspect or copy the unshown remainder after overflow.
    if (!overflow) caption.write(source.substring(cursor));
    final text = caption.toString();
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.hasBoundedWidth
            ? constraints.maxWidth
            : 280.0;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (text.trim().isNotEmpty)
              for (
                var line = 0;
                line <
                    (text.length / math.max(1, width / 8)).ceil().clamp(1, 4);
                line++
              )
                Padding(
                  padding: const EdgeInsets.only(bottom: Grid.half),
                  child: SkeletonBar(
                    width: math.min(width, math.max(48, text.length * 7.0)),
                    height: 16,
                  ),
                ),
            for (final url in attachments)
              Padding(
                padding: const EdgeInsets.only(bottom: Grid.xxs),
                child: _attachment(context, url, metadata[url], width),
              ),
            if (gallery != null)
              MessageGalleryFrame(
                key: const ValueKey('message-skeleton-gallery'),
                count: gallery.items.length,
                loading: true,
                child: const SizedBox(
                  height: messageMediaCarouselHeight,
                  width: double.infinity,
                  child: SkeletonBar(
                    width: double.infinity,
                    height: double.infinity,
                  ),
                ),
              ),
            if (overflow)
              const SkeletonBar(
                key: ValueKey('message-skeleton-overflow'),
                width: 72,
                height: 16,
              ),
          ],
        );
      },
    );
  }

  Widget _attachment(
    BuildContext context,
    String url,
    ImetaEntry? meta,
    double width,
  ) {
    // Production treats every non-audio/non-video embed as an image preview,
    // including unknown extensions and PDFs (which may reach its error UI).
    final kind = classifyMediaUrl(url, imeta: meta) ?? MessageMediaKind.image;
    if (kind == MessageMediaKind.image || kind == MessageMediaKind.video) {
      final isVideo = kind == MessageMediaKind.video;
      final imageSize = messageImagePreviewSize(context, meta?.aspectRatio);
      final mediaWidth = math.min(
        width,
        isVideo ? messageMediaMaxWidth(context) : imageSize.width,
      );
      // Video's production frame includes a one-pixel border around the
      // aspect-ratio child; reserve that same outer geometry here.
      final mediaHeight = isVideo
          ? (mediaWidth - 2) / messageVideoAspectRatio(meta?.aspectRatio) + 2
          : imageSize.height;
      return SizedBox(
        key: ValueKey('message-skeleton-${kind.name}:$url'),
        width: mediaWidth,
        height: mediaHeight,
        child: Stack(
          alignment: Alignment.center,
          children: [
            Opacity(
              opacity: kind == MessageMediaKind.video ? 0.35 : 1,
              child: SkeletonBar(
                width: double.infinity,
                height: double.infinity,
                borderRadius: BorderRadius.circular(Radii.md),
              ),
            ),
            if (kind == MessageMediaKind.video)
              const Icon(
                Icons.play_circle_outline,
                size: 48,
                color: Colors.white,
              ),
          ],
        ),
      );
    }
    return SizedBox(
      key: ValueKey('message-skeleton-audio:$url'),
      width: math.min(width, 320),
      height: 64,
      child: Row(
        children: [
          SkeletonBar(
            width: 40,
            height: 40,
            borderRadius: BorderRadius.circular(Radii.full),
          ),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                SizedBox(
                  height: 24,
                  child: Row(
                    children: [
                      for (var i = 0; i < 24; i++)
                        Expanded(
                          child: Padding(
                            padding: const EdgeInsets.symmetric(horizontal: 1),
                            child: SkeletonBar(
                              width: 3,
                              height: [8.0, 16.0, 24.0, 12.0, 20.0][i % 5],
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: Grid.half),
                const SkeletonBar(width: 36, height: 10),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
