import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../animated_avatar.dart';
import '../emoji/emoji_avatar.dart';
import '../emoji/native_emoji_glyph.dart';
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
    final animatedAvatar = parseAnimatedAvatarUrl(imageUrl);
    return CircleAvatar(
      radius: radius,
      // Animated avatar posters carry their own backdrop disc; preserve their
      // transparent surroundings on static/list surfaces, matching desktop.
      backgroundColor: animatedAvatar == null
          ? backgroundColor
          : Colors.transparent,
      child: ClipOval(
        child: SizedBox.square(
          dimension: radius * 2,
          child: AvatarImageContent(
            imageUrl: animatedAvatar?.posterUrl ?? imageUrl,
            fallback: fallback,
          ),
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
          builder: (_, constraints) {
            final glyphSize = constraints.biggest.shortestSide * 258 / 512;
            return Center(
              child: NativeEmojiGlyph(
                emoji: emoji,
                size: glyphSize,
                opticalBoxSize: glyphSize,
              ),
            );
          },
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
        final emojiAvatar = parseEmojiAvatarSvg(svg);
        return emojiAvatar == null
            ? _SvgAvatarSource(svg)
            : _EmojiAvatarSource(
                emojiAvatar.emoji,
                Color(emojiAvatar.colorValue),
              );
      }
      return _RasterDataAvatarSource(data.contentAsBytes());
    } on FormatException {
      return null;
    }
  }
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
