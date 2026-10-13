import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

/// A standalone system emoji whose visual centre matches its surrounding UI.
///
/// Inline glyphs retain their platform baseline adjustment. Fixed reaction
/// boxes can opt into emoji-font metrics and optical centering independently
/// of the surrounding text font.
class NativeEmojiGlyph extends StatelessWidget {
  /// Creates a system emoji with optional optical bounds.
  const NativeEmojiGlyph({
    super.key,
    required this.emoji,
    required this.size,
    this.opticalBoxSize,
    this.centerInOpticalBox = false,
  });

  /// Emoji text rendered by the platform font.
  final String emoji;

  /// Font size used to paint the emoji.
  final double size;

  /// Optional square that normalizes the apparent bounds of wide emoji.
  final double? opticalBoxSize;

  /// Centers reaction artwork in its fixed box instead of a surrounding line.
  final bool centerInOpticalBox;

  @override
  Widget build(BuildContext context) {
    Widget glyph = Text(
      emoji,
      maxLines: 1,
      softWrap: false,
      textScaler: TextScaler.noScaling,
      style: TextStyle(
        fontSize: size,
        height: 1,
        leadingDistribution: centerInOpticalBox
            ? TextLeadingDistribution.even
            : null,
        fontFamily:
            centerInOpticalBox && defaultTargetPlatform == TargetPlatform.iOS
            ? 'Apple Color Emoji'
            : null,
      ),
    );
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      // Apple Color Emoji's painted center sits above its typographic center.
      // Reaction boxes use the emoji font directly and compensate proportionally;
      // inline glyphs retain their existing baseline adjustment.
      glyph = Transform.translate(
        offset: Offset(0, centerInOpticalBox ? size * 0.15 : -1),
        child: glyph,
      );
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
