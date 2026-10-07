import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

/// A standalone system emoji whose visual centre matches its surrounding UI.
///
/// Apple's emoji glyphs sit slightly low inside Flutter's text box. Keep the
/// layout box unchanged and lift only the painted glyph on iOS; Android's
/// system emoji metrics are already visually centred.
class NativeEmojiGlyph extends StatelessWidget {
  final String emoji;
  final double size;

  const NativeEmojiGlyph({super.key, required this.emoji, required this.size});

  @override
  Widget build(BuildContext context) {
    final glyph = Text(emoji, style: TextStyle(fontSize: size));
    if (defaultTargetPlatform != TargetPlatform.iOS) return glyph;

    return Transform.translate(offset: const Offset(0, -1), child: glyph);
  }
}
