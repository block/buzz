import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/custom_widgets/markdown_config.dart';

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
    final small = CustomEmojiMd(_palette, content: 'hello :wave:');
    final large = CustomEmojiMd(largePalette, content: 'hello :wave:');
    expect(large.exp.pattern, small.exp.pattern);
    expect(large.exp.hasMatch(':wave:'), isTrue);
    expect(large.exp.hasMatch(':wave_long:'), isFalse);
    expect(large.exp.hasMatch(':unused_2499:'), isFalse);
  });

  test(
    'no references and unknown references produce a nonmatching pattern',
    () {
      for (final content in [
        'hello world',
        ':unknown:',
        'https://example.com',
      ]) {
        final matcher = CustomEmojiMd(_palette, content: content);
        expect(matcher.exp.allMatches(content), isEmpty);
        expect(matcher.exp.hasMatch(':wave:'), isFalse);
      }
      expect(
        CustomEmojiMd(const [], content: ':wave:').exp.hasMatch(':wave:'),
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
      final selected = CustomEmojiMd(_palette, content: content);
      expect(
        selected.exp.allMatches(content).map((m) => m.group(0)).toList(),
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
    final matcher = CustomEmojiMd(
      _palette,
      content: ':WAVE: :unknown:',
      size: 32,
    );
    final known = matcher.span(context, ':WAVE:', GptMarkdownConfig());
    expect(known, isA<WidgetSpan>());
    final image = (known as WidgetSpan).child as CustomEmojiImage;
    expect(image.shortcode, 'wave');
    expect(image.url, 'https://example.com/wave.png');
    expect(image.size, 32);
    final unknown = matcher.span(context, ':unknown:', GptMarkdownConfig());
    expect(unknown, isA<TextSpan>());
    expect((unknown as TextSpan).text, ':unknown:');
  });

  testWidgets('selection uses the current content and current palette URL', (
    tester,
  ) async {
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    final context = tester.element(find.byType(SizedBox));
    final initial = CustomEmojiMd(_palette, content: ':wave:');
    final updated = CustomEmojiMd(const [
      CustomEmoji(shortcode: 'wave_long', url: 'https://example.com/new.png'),
    ], content: ':wave_long:');
    expect(initial.exp.hasMatch(':wave_long:'), isFalse);
    expect(updated.exp.hasMatch(':wave:'), isFalse);
    final span = updated.span(context, ':wave_long:', GptMarkdownConfig());
    expect(
      ((span as WidgetSpan).child as CustomEmojiImage).url,
      'https://example.com/new.png',
    );
  });
}
