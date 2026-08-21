import 'package:flutter/foundation.dart';

enum MessageMediaKind { image, video }

@immutable
class ImetaEntry {
  final String url;
  final String? mimeType;
  final String? dimensions;
  final String? thumb;
  final String? image;
  final String? alt;

  const ImetaEntry({
    required this.url,
    this.mimeType,
    this.dimensions,
    this.thumb,
    this.image,
    this.alt,
  });

  bool get isVideo => mimeType?.startsWith('video/') == true;

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
    );
  }
  return byUrl;
}

MessageMediaKind? classifyMediaUrl(String url, {ImetaEntry? imeta}) {
  final mimeType = imeta?.mimeType;
  if (mimeType != null) {
    // An imeta MIME type is authoritative. The native video player chooses
    // whether the device can decode the specific codec/container; rejecting
    // every non-MP4 video here prevents it from even trying.
    if (mimeType.startsWith('video/')) return MessageMediaKind.video;
    if (mimeType.startsWith('image/')) return MessageMediaKind.image;
  }

  final path = (Uri.tryParse(url)?.path ?? url).toLowerCase();
  if (path.endsWith(_mp4Extension)) {
    return MessageMediaKind.video;
  }
  if (_imageExtensions.any(path.endsWith)) {
    return MessageMediaKind.image;
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
