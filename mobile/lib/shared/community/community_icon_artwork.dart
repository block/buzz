import 'dart:convert';
import 'dart:ui' as ui;

import '../emoji/emoji_avatar.dart';

const _maximumSourceEdge = 4096;
const _maximumSourcePixels = 4 * 1024 * 1024;
const _maximumCachedEdge = 512;
const _maximumEncodedBytes = 256 * 1024;

// Only the product's fixed emoji template is safe to retain as SVG. Arbitrary
// SVG can reference external or embedded rasters that bypass the limits below.
// Accept current desktop artwork and its two legacy rounded backgrounds.
final _emojiSvg = RegExp(
  r'^<svg xmlns="http://www\.w3\.org/2000/svg" width="512" height="512" viewBox="0 0 512 512"><rect width="512" height="512"(?: rx="(?:112|256)")? fill="#[0-9a-fA-F]{6}"/><text x="50%" y="56%" dominant-baseline="middle" text-anchor="middle" font-size="258">[^<>]{1,64}</text></svg>$',
);

/// Rejects pathological raster dimensions before decoding and persists only a
/// small first-frame PNG or a regenerated, resource-free emoji SVG.
Future<String?> prepareCommunityIconArtwork(String artwork) async {
  try {
    final data = UriData.parse(artwork);
    final bytes = data.contentAsBytes();
    if (bytes.length > _maximumEncodedBytes) return null;
    if (data.mimeType == 'image/svg+xml') {
      final svg = utf8.decode(bytes);
      if (!_emojiSvg.hasMatch(svg)) return null;
      final emoji = parseEmojiAvatarSvg(svg);
      if (emoji == null) return null;
      return emojiAvatarDataUrl(emoji.emoji, emoji.colorValue);
    }
    final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
    try {
      final descriptor = await ui.ImageDescriptor.encoded(buffer);
      try {
        final width = descriptor.width;
        final height = descriptor.height;
        if (width <= 0 ||
            height <= 0 ||
            width > _maximumSourceEdge ||
            height > _maximumSourceEdge ||
            width * height > _maximumSourcePixels) {
          return null;
        }
        final codec = await descriptor.instantiateCodec(
          targetWidth: width >= height
              ? width.clamp(1, _maximumCachedEdge)
              : null,
          targetHeight: height > width
              ? height.clamp(1, _maximumCachedEdge)
              : null,
        );
        try {
          final frame = await codec.getNextFrame();
          try {
            final png = await frame.image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            if (png == null || png.lengthInBytes > _maximumEncodedBytes) {
              return null;
            }
            return Uri.dataFromBytes(
              png.buffer.asUint8List(),
              mimeType: 'image/png',
            ).toString();
          } finally {
            frame.image.dispose();
          }
        } finally {
          codec.dispose();
        }
      } finally {
        descriptor.dispose();
      }
    } finally {
      buffer.dispose();
    }
  } catch (_) {
    return null;
  }
}
