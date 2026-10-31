import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// Renders [text] the way gpt_markdown does: the builder only ever sees a
/// match of the pattern.
InlineSpan _render(
  CustomEmojiPattern pattern,
  BuildContext context,
  String text,
) => pattern.builder(
  context,
  pattern.pattern.firstMatch(text)!,
  const TextStyle(),
);

const _palette = [
  CustomEmoji(shortcode: 'wave', url: 'https://example.com/wave.png'),
  CustomEmoji(shortcode: 'wave_long', url: 'https://example.com/long.png'),
  CustomEmoji(shortcode: 'party-parrot', url: 'https://example.com/parrot.png'),
];

void main() {
  test('pattern is bounded by referenced emoji, not the community palette', () {
    final largePalette = [
      ..._palette,
      for (var i = 0; i < 2500; i++)
        CustomEmoji(shortcode: 'unused_$i', url: 'https://example.com/$i.png'),
    ];
    final small = CustomEmojiPattern(_palette, content: 'hello :wave:');
    final large = CustomEmojiPattern(largePalette, content: 'hello :wave:');
    expect(large.pattern.pattern, small.pattern.pattern);
    expect(large.pattern.hasMatch(':wave:'), isTrue);
    expect(large.pattern.hasMatch(':wave_long:'), isFalse);
    expect(large.pattern.hasMatch(':unused_2499:'), isFalse);
  });

  test(
    'no references and unknown references produce a nonmatching pattern',
    () {
      for (final content in [
        'hello world',
        ':unknown:',
        'https://example.com',
      ]) {
        final matcher = CustomEmojiPattern(_palette, content: content);
        expect(matcher.pattern.allMatches(content), isEmpty);
        expect(matcher.pattern.hasMatch(':wave:'), isFalse);
      }
      expect(
        CustomEmojiPattern(
          const [],
          content: ':wave:',
        ).pattern.hasMatch(':wave:'),
        isFalse,
      );
    },
  );

  test('selection preserves the original matcher across token boundaries', () {
    final original = RegExp(
      ':(?:${_palette.map((e) => RegExp.escape(e.shortcode)).join('|')}):',
      caseSensitive: false,
    );
    for (final content in [
      ':wave:',
      ':WAVE: :Wave_Long: :PARTY-PARROT:',
      ':unknown:wave:',
      ':wave:unknown:wave_long:',
      ':wave::wave_long:',
      ':::wave::: :wave_long:! (:party-parrot:)',
      ':wave_longer: :unknown: no match',
      '`code :wave:` **bold :wave_long:**',
    ]) {
      final selected = CustomEmojiPattern(_palette, content: content);
      expect(
        selected.pattern.allMatches(content).map((m) => m.group(0)).toList(),
        original.allMatches(content).map((m) => m.group(0)).toList(),
        reason: content,
      );
    }
  });

  testWidgets('known tokens keep their URL and size; unknowns remain text', (
    tester,
  ) async {
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    final context = tester.element(find.byType(SizedBox));
    final matcher = CustomEmojiPattern(
      _palette,
      content: ':WAVE: :unknown:',
      size: 32,
    );
    final known = _render(matcher, context, ':WAVE:');
    expect(known, isA<WidgetSpan>());
    final image = (known as WidgetSpan).child as CustomEmojiImage;
    expect(image.shortcode, 'wave');
    expect(image.url, 'https://example.com/wave.png');
    expect(image.size, 32);
    expect(matcher.pattern.hasMatch(':unknown:'), isFalse);
  });

  testWidgets('selection uses the current content and current palette URL', (
    tester,
  ) async {
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    final context = tester.element(find.byType(SizedBox));
    final initial = CustomEmojiPattern(_palette, content: ':wave:');
    final updated = CustomEmojiPattern(const [
      CustomEmoji(shortcode: 'wave_long', url: 'https://example.com/new.png'),
    ], content: ':wave_long:');
    expect(initial.pattern.hasMatch(':wave_long:'), isFalse);
    expect(updated.pattern.hasMatch(':wave:'), isFalse);
    final span = _render(updated, context, ':wave_long:');
    expect(
      ((span as WidgetSpan).child as CustomEmojiImage).url,
      'https://example.com/new.png',
    );
  });
}
