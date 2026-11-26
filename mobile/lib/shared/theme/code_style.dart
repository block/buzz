import 'package:flutter/material.dart';

import 'message_typography.dart';

/// The app's own code style — one place stating the mono face, its size and
/// the colours a code surface is drawn with.
///
/// Two surfaces render code and they have to read as one family: the fenced
/// block in `MessageContent`, and the inline chip `gpt_markdown` paints from
/// the `InlineCodeStyle` declared in `AppTheme`. Both take their values from
/// here, so neither can drift from the other by editing one copy.
abstract final class CodeStyle {
  /// The mono face, already used by the composer and by code blocks.
  static const fontFamily = 'GeistMono';

  /// Relative size of rendered code, originally 13 points on a 15-point body.
  static const fontSizeFactor = 13 / 15;

  static final fontSize = messageBodyTextStyle.fontSize! * fontSizeFactor;

  /// Line height of code text.
  static const lineHeight = 1.5;

  /// Fill behind a code surface.
  static Color background(ColorScheme scheme) => scheme.surfaceContainerHighest;

  /// Outline around a code surface.
  static Color border(ColorScheme scheme) =>
      scheme.outline.withValues(alpha: 0.7);
}
