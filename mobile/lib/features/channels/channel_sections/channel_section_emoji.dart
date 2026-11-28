import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/custom_emoji/custom_emoji.dart';
import '../../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../../shared/emoji/native_emoji_glyph.dart';
import '../../../shared/relay/media_image.dart';

/// A section icon in the shared Unicode or `:shortcode:` wire format.
class ChannelSectionEmoji extends ConsumerWidget {
  const ChannelSectionEmoji({
    super.key,
    required this.value,
    this.size = 16,
    this.padding = EdgeInsets.zero,
    this.fallback = const SizedBox.shrink(),
  });

  final String? value;
  final double size;
  final EdgeInsetsGeometry padding;
  final Widget fallback;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final emoji = value?.trim();
    if (emoji == null || emoji.isEmpty) return fallback;
    Widget artwork(Widget child) => ExcludeSemantics(
      child: Padding(padding: padding, child: child),
    );

    if (emoji.length >= 3 && emoji.startsWith(':') && emoji.endsWith(':')) {
      final url = ref.watch(
        customEmojiListProvider.select(
          (palette) => reactionEmojiUrl(emoji, palette),
        ),
      );
      if (url == null) return fallback;
      return MediaImage(
        url: url,
        width: size,
        height: size,
        decodeWidth: size,
        fit: BoxFit.contain,
        frameBuilder: (_, child, frame, synchronous) =>
            frame != null || synchronous ? artwork(child) : fallback,
        errorBuilder: (_, _, _) => fallback,
      );
    }

    return artwork(
      NativeEmojiGlyph(
        emoji: emoji,
        size: size,
        opticalBoxSize: size,
        centerInOpticalBox: true,
      ),
    );
  }
}
