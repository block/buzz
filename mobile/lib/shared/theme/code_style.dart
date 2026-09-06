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

  /// Code size at message body scale.
  static const fontSize = 13.0;

  /// Line height of code text.
  static const lineHeight = 1.5;

  /// [fontSize] stated against the message body size, for APIs that scale code
  /// relative to the text around it rather than fixing a point size. Keeping
  /// it a ratio means code follows the body if that size ever changes.
  static final fontSizeFactor = fontSize / messageBodyTextStyle.fontSize!;

  /// Fill behind a code surface.
  static Color background(ColorScheme scheme) =>
      scheme.surfaceContainerHighest.withValues(alpha: 0.6);

  /// Outline around a code surface.
  static Color border(ColorScheme scheme) =>
      scheme.outline.withValues(alpha: 0.7);
}
