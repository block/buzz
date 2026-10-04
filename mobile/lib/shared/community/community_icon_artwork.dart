import 'dart:ui' as ui;

const _maximumSourceEdge = 4096;
const _maximumSourcePixels = 4 * 1024 * 1024;
const _maximumCachedEdge = 512;
const _maximumEncodedBytes = 256 * 1024;

/// Rejects pathological raster dimensions before decoding and persists only a
/// small first-frame PNG. SVG artwork retains its vector/emoji presentation.
Future<String?> prepareCommunityIconArtwork(String artwork) async {
  try {
    final data = UriData.parse(artwork);
    final bytes = data.contentAsBytes();
    if (bytes.length > _maximumEncodedBytes) return null;
    if (data.mimeType == 'image/svg+xml') return artwork;
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
