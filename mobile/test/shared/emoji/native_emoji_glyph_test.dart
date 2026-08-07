import 'package:buzz/shared/emoji/native_emoji_glyph.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('lifts the glyph one logical pixel on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      await tester.pumpWidget(
        const MaterialApp(
          home: Center(child: NativeEmojiGlyph(emoji: '🔥', size: 24)),
        ),
      );

      final transform = tester.widget<Transform>(find.byType(Transform));
      expect(transform.transform.getTranslation().y, -1);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('keeps the glyph unshifted on Android', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await tester.pumpWidget(
        const MaterialApp(
          home: Center(child: NativeEmojiGlyph(emoji: '🔥', size: 24)),
        ),
      );

      expect(find.byType(Transform), findsNothing);
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
