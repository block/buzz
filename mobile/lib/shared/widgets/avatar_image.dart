import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

/// A circular avatar that supports both remote URLs and inline image data.
///
/// Flutter's [NetworkImage] only loads network URLs, while desktop browsers also
/// accept `data:image/*` sources directly. Agent emoji avatars are inline SVGs,
/// so mobile must decode those before rendering them.
class AvatarImage extends StatelessWidget {
  final String? imageUrl;
  final double radius;
  final Color? backgroundColor;
  final Widget fallback;

  const AvatarImage({
    super.key,
    required this.imageUrl,
    required this.radius,
    required this.fallback,
    this.backgroundColor,
  });

  @override
  Widget build(BuildContext context) {
    return CircleAvatar(
      radius: radius,
      backgroundColor: backgroundColor,
      child: ClipOval(
        child: SizedBox.square(
          dimension: radius * 2,
          child: AvatarImageContent(imageUrl: imageUrl, fallback: fallback),
        ),
      ),
    );
  }
}

/// Image content for avatar surfaces whose shape is supplied by their parent.
class AvatarImageContent extends StatelessWidget {
  final String? imageUrl;
  final Widget fallback;
  final BoxFit fit;

  const AvatarImageContent({
    super.key,
    required this.imageUrl,
    required this.fallback,
    this.fit = BoxFit.cover,
  });

  @override
  Widget build(BuildContext context) {
    final source = _AvatarSource.parse(imageUrl);
    final centeredFallback = Center(child: fallback);

    return switch (source) {
      _SvgAvatarSource(:final svg) => SvgPicture.string(
        svg,
        fit: fit,
        placeholderBuilder: (_) => centeredFallback,
        errorBuilder: (_, _, _) => centeredFallback,
      ),
      _RasterDataAvatarSource(:final bytes) => Image.memory(
        bytes,
        fit: fit,
        errorBuilder: (_, _, _) => centeredFallback,
      ),
      _NetworkAvatarSource(:final url) => Image.network(
        url,
        fit: fit,
        errorBuilder: (_, _, _) => centeredFallback,
      ),
      null => centeredFallback,
    };
  }
}

sealed class _AvatarSource {
  const _AvatarSource();

  static _AvatarSource? parse(String? value) {
    final url = value?.trim();
    if (url == null || url.isEmpty) return null;
    if (!url.startsWith('data:image/')) return _NetworkAvatarSource(url);

    try {
      final data = UriData.parse(url);
      if (data.mimeType == 'image/svg+xml') {
        final Uint8List bytes = data.contentAsBytes();
        return _SvgAvatarSource(utf8.decode(bytes));
      }
      return _RasterDataAvatarSource(data.contentAsBytes());
    } on FormatException {
      return null;
    }
  }
}

class _SvgAvatarSource extends _AvatarSource {
  final String svg;
  const _SvgAvatarSource(this.svg);
}

class _RasterDataAvatarSource extends _AvatarSource {
  final Uint8List bytes;
  const _RasterDataAvatarSource(this.bytes);
}

class _NetworkAvatarSource extends _AvatarSource {
  final String url;
  const _NetworkAvatarSource(this.url);
}
