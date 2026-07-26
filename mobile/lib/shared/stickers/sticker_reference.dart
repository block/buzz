/// A validated Sonar sticker reference carried by a message `sticker` tag.
class StickerReference {
  final String authorPubkey;
  final String packIdentifier;
  final String shortcode;
  final String sha256;

  const StickerReference({
    required this.authorPubkey,
    required this.packIdentifier,
    required this.shortcode,
    required this.sha256,
  });

  String get coordinate => '30031:$authorPubkey:$packIdentifier';

  /// Build the verified relay-cache URL for this sticker.
  ///
  /// Sticker source URLs never reach the client. The relay resolves the
  /// coordinate, verifies the content hash, and serves the cached asset.
  String? cacheUrl(String relayBaseUrl) {
    final base = Uri.tryParse(relayBaseUrl);
    if (base == null ||
        !base.hasAuthority ||
        (base.scheme != 'http' && base.scheme != 'https')) {
      return null;
    }

    return Uri(
      scheme: base.scheme,
      host: base.host,
      port: base.hasPort ? base.port : null,
      pathSegments: [
        'media',
        'sticker',
        authorPubkey,
        packIdentifier,
        shortcode,
        sha256,
      ],
    ).toString();
  }
}

enum StickerTagStatus { absent, valid, invalid }

/// Result of parsing the message's Sonar `sticker` tag.
///
/// A message must contain exactly zero or one sticker tag. More than one tag,
/// or one malformed tag, is invalid and must not result in a network request.
class StickerTagParseResult {
  final StickerTagStatus status;
  final StickerReference? reference;

  const StickerTagParseResult._(this.status, this.reference);

  static const absent = StickerTagParseResult._(StickerTagStatus.absent, null);

  static const invalid = StickerTagParseResult._(
    StickerTagStatus.invalid,
    null,
  );

  factory StickerTagParseResult.valid(StickerReference reference) =>
      StickerTagParseResult._(StickerTagStatus.valid, reference);
}

final RegExp _hex64 = RegExp(r'^[0-9a-f]{64}$');
final RegExp _packIdentifier = RegExp(r'^[A-Za-z0-9._-]{1,80}$');
final RegExp _shortcode = RegExp(r'^[A-Za-z0-9_]{1,64}$');

/// Parse the single Sonar message reference tag from [tags].
StickerTagParseResult parseStickerReference(List<List<String>> tags) {
  final stickerTags = tags.where(
    (tag) => tag.isNotEmpty && tag[0] == 'sticker',
  );
  final iterator = stickerTags.iterator;
  if (!iterator.moveNext()) return StickerTagParseResult.absent;
  final tag = iterator.current;
  if (iterator.moveNext() || tag.length != 4) {
    return StickerTagParseResult.invalid;
  }

  final coordinate = tag[1].split(':');
  if (coordinate.length != 3 || coordinate[0] != '30031') {
    return StickerTagParseResult.invalid;
  }

  final author = coordinate[1];
  final identifier = coordinate[2];
  final shortcode = tag[2];
  final sha256 = tag[3];
  if (!_hex64.hasMatch(author) ||
      !_packIdentifier.hasMatch(identifier) ||
      !_shortcode.hasMatch(shortcode) ||
      !_hex64.hasMatch(sha256)) {
    return StickerTagParseResult.invalid;
  }

  return StickerTagParseResult.valid(
    StickerReference(
      authorPubkey: author,
      packIdentifier: identifier,
      shortcode: shortcode,
      sha256: sha256,
    ),
  );
}
