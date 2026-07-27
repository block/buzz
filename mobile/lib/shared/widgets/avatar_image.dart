import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../relay/relay.dart';

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
class AvatarImageContent extends StatefulWidget {
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
  State<AvatarImageContent> createState() => _AvatarImageContentState();
}

class _AvatarImageContentState extends State<AvatarImageContent> {
  late _AvatarSource? _source = _AvatarSource.parse(widget.imageUrl);

  @override
  void didUpdateWidget(AvatarImageContent oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.imageUrl != oldWidget.imageUrl) {
      _source = _AvatarSource.parse(widget.imageUrl);
    }
  }

  @override
  Widget build(BuildContext context) {
    final centeredFallback = Center(child: widget.fallback);

    return switch (_source) {
      _EmojiAvatarSource(:final emoji, :final color) => ColoredBox(
        color: color,
        child: LayoutBuilder(
          builder: (_, constraints) => Center(
            child: Text(
              emoji,
              textScaler: TextScaler.noScaling,
              style: TextStyle(
                fontSize: constraints.biggest.shortestSide * 258 / 512,
                height: 1,
              ),
            ),
          ),
        ),
      ),
      _SvgAvatarSource(:final svg) => SvgPicture.string(
        svg,
        fit: widget.fit,
        placeholderBuilder: (_) => centeredFallback,
        errorBuilder: (_, _, _) => centeredFallback,
      ),
      _RasterDataAvatarSource(:final bytes) => Image.memory(
        bytes,
        fit: widget.fit,
        errorBuilder: (_, _, _) => centeredFallback,
      ),
      _NetworkAvatarSource(:final url) => MediaImage(
        url: url,
        fit: widget.fit,
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
        final svg = utf8.decode(bytes);
        return _parseEmojiAvatar(svg) ?? _SvgAvatarSource(svg);
      }
      return _RasterDataAvatarSource(data.contentAsBytes());
    } on FormatException {
      return null;
    }
  }
}

_EmojiAvatarSource? _parseEmojiAvatar(String svg) {
  final colorValue = RegExp(
    r'<rect\b[^>]*\sfill="([^"]+)"',
  ).firstMatch(svg)?[1];
  final emojiValue = RegExp(r'<text\b[^>]*>(.*?)</text>').firstMatch(svg)?[1];
  if (colorValue == null || emojiValue == null) return null;

  final color = _parseHexColor(colorValue);
  if (color == null) return null;
  final emoji = emojiValue
      .replaceAll('&gt;', '>')
      .replaceAll('&lt;', '<')
      .replaceAll('&amp;', '&');
  return _EmojiAvatarSource(emoji, color);
}

Color? _parseHexColor(String value) {
  final hex = value.startsWith('#') ? value.substring(1) : value;
  if (!RegExp(r'^[0-9a-fA-F]{6}$').hasMatch(hex)) return null;
  final rgb = int.tryParse(hex, radix: 16);
  return rgb == null ? null : Color(0xFF000000 | rgb);
}

class _EmojiAvatarSource extends _AvatarSource {
  final String emoji;
  final Color color;
  const _EmojiAvatarSource(this.emoji, this.color);
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
