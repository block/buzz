import 'package:flutter/foundation.dart';

/// Media presentation selected for a message attachment URL.
enum MessageMediaKind { image, video, audio }

/// Parsed metadata from a NIP-92 `imeta` tag.
@immutable
class ImetaEntry {
  final String url;
  final String? mimeType;
  final String? dimensions;
  final String? thumb;
  final String? image;
  final String? alt;
  final double? duration;
  final String? filename;
  final int? size;

  const ImetaEntry({
    required this.url,
    this.mimeType,
    this.dimensions,
    this.thumb,
    this.image,
    this.alt,
    this.duration,
    this.filename,
    this.size,
  });

  bool get isVideo => mimeType?.startsWith('video/') == true;

  bool get isAudio => mimeType?.startsWith('audio/') == true;

  String? get posterUrl => image ?? thumb;

  double? get aspectRatio {
    final parts = dimensions?.split('x');
    if (parts == null || parts.length != 2) return null;
    final width = double.tryParse(parts[0]);
    final height = double.tryParse(parts[1]);
    if (width == null || height == null || width <= 0 || height <= 0) {
      return null;
    }
    return width / height;
  }
}

/// Parses NIP-92 `imeta` tags into entries keyed by their attachment URL.
Map<String, ImetaEntry> parseImetaTags(List<List<String>> tags) {
  final byUrl = <String, ImetaEntry>{};
  for (final tag in tags) {
    if (tag.isEmpty || tag.first != 'imeta') continue;

    String? url;
    String? mimeType;
    String? dimensions;
    String? thumb;
    String? image;
    String? alt;
    double? duration;
    String? filename;
    int? size;

    for (final part in tag.skip(1)) {
      final separator = part.indexOf(' ');
      if (separator <= 0) continue;
      final key = part.substring(0, separator);
      final value = part.substring(separator + 1);
      switch (key) {
        case 'url':
          url = value;
        case 'm':
          mimeType = value;
        case 'dim':
          dimensions = value;
        case 'thumb':
          thumb = value;
        case 'image':
          image = value;
        case 'alt':
          alt = value;
        case 'duration':
          final parsedDuration = double.tryParse(value);
          duration =
              parsedDuration != null &&
                  parsedDuration.isFinite &&
                  parsedDuration >= 0
              ? parsedDuration
              : null;
        case 'filename':
          filename = value;
        case 'size':
          size = int.tryParse(value);
      }
    }

    if (url == null || url.isEmpty) continue;
    byUrl[url] = ImetaEntry(
      url: url,
      mimeType: mimeType,
      dimensions: dimensions,
      thumb: thumb,
      image: image,
      alt: alt,
      duration: duration,
      filename: filename,
      size: size,
    );
  }
  return byUrl;
}

/// Classifies [url] using authoritative [imeta] before extension fallback.
MessageMediaKind? classifyMediaUrl(String url, {ImetaEntry? imeta}) {
  final mimeType = imeta?.mimeType;
  if (mimeType != null) {
    final filename = imeta?.filename?.toLowerCase();
    if (mimeType == 'video/mp4' &&
        filename != null &&
        filename.startsWith('voice-note-') &&
        filename.endsWith('.mp4')) {
      return MessageMediaKind.audio;
    }
    // An imeta MIME type is authoritative. The native video player chooses
    // whether the device can decode the specific codec/container; rejecting
    // every non-MP4 video here prevents it from even trying.
    if (mimeType.startsWith('video/')) return MessageMediaKind.video;
    if (mimeType.startsWith('image/')) return MessageMediaKind.image;
    if (mimeType.startsWith('audio/')) return MessageMediaKind.audio;
  }

  final path = (Uri.tryParse(url)?.path ?? url).toLowerCase();
  if (path.endsWith(_mp4Extension)) {
    return MessageMediaKind.video;
  }
  if (_imageExtensions.any(path.endsWith)) {
    return MessageMediaKind.image;
  }
  if (path.endsWith('.m4a') || path.endsWith('.aac')) {
    return MessageMediaKind.audio;
  }
  return null;
}

const _imageExtensions = {
  '.jpg',
  '.jpeg',
  '.png',
  '.webp',
  '.bmp',
  '.heic',
  '.heif',
  '.avif',
};

const _mp4Extension = '.mp4';
