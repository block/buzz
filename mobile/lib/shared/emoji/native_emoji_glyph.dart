import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

/// A standalone system emoji whose visual centre matches its surrounding UI.
///
/// Apple's emoji glyphs sit slightly low inside Flutter's text box. Keep the
/// layout box unchanged and lift only the painted glyph on iOS; Android's
/// system emoji metrics are already visually centred.
class NativeEmojiGlyph extends StatelessWidget {
  /// Creates a system emoji with optional optical bounds.
  const NativeEmojiGlyph({
    super.key,
    required this.emoji,
    required this.size,
    this.opticalBoxSize,
  });

  /// Emoji text rendered by the platform font.
  final String emoji;

  /// Font size used to paint the emoji.
  final double size;

  /// Optional square that normalizes the apparent bounds of wide emoji.
  final double? opticalBoxSize;

  @override
  Widget build(BuildContext context) {
    Widget glyph = Text(
      emoji,
      maxLines: 1,
      softWrap: false,
      textScaler: TextScaler.noScaling,
      style: TextStyle(fontSize: size, height: 1),
    );
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      glyph = Transform.translate(offset: const Offset(0, -1), child: glyph);
    }
    final boxSize = opticalBoxSize;
    if (boxSize == null) return glyph;

    // Emoji sequences have very different typographic advances. Constrain
    // them to the same square optical box so wide glyphs scale down around the
    // same center instead of appearing to lean toward one edge.
    return SizedBox.square(
      dimension: boxSize,
      child: FittedBox(fit: BoxFit.scaleDown, child: glyph),
    );
  }
}
