import 'package:buzz/shared/emoji/native_emoji_glyph.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('uses one centered optical box for narrow and wide emoji', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: Row(
            children: [
              NativeEmojiGlyph(
                key: ValueKey('narrow-emoji'),
                emoji: '🙂',
                size: 60,
                opticalBoxSize: 60,
              ),
              NativeEmojiGlyph(
                key: ValueKey('wide-emoji'),
                emoji: '👩‍👩‍👧‍👦',
                size: 60,
                opticalBoxSize: 60,
              ),
            ],
          ),
        ),
      ),
    );

    expect(
      tester.getSize(find.byKey(const ValueKey('narrow-emoji'))),
      const Size.square(60),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('wide-emoji'))),
      const Size.square(60),
    );
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('wide-emoji')),
        matching: find.byType(FittedBox),
      ),
      findsOneWidget,
    );
    for (final text in tester.widgetList<Text>(find.byType(Text))) {
      expect(text.textScaler, TextScaler.noScaling);
    }
  });
}
