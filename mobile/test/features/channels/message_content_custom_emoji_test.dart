import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

Widget _testable(String content, {List<List<String>> tags = const []}) {
  return ProviderScope(
    overrides: [
      customEmojiListProvider.overrideWithValue([
        const CustomEmoji(
          shortcode: 'wave',
          url: 'https://example.com/wave.png',
        ),
        for (var i = 0; i < 2500; i++)
          CustomEmoji(
            shortcode: 'unused_$i',
            url: 'https://example.com/$i.png',
          ),
      ]),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(
        body: MessageContent(
          content: content,
          tags: tags,
          channelNames: const {'test': 'test-channel'},
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('message wiring excludes unrelated emoji from the regex', (
    tester,
  ) async {
    await tester.pumpWidget(_testable('**hello** :unknown:'));
    final markdown = tester.widget<GptMarkdown>(find.byType(GptMarkdown));
    final component = markdown.inlineComponents!
        .whereType<CustomEmojiMd>()
        .single;
    expect(component.exp.hasMatch(':unused_2499:'), isFalse);
    expect(component.exp.hasMatch(':wave:'), isFalse);
    expect(find.byType(CustomEmojiImage), findsNothing);
    final text = tester
        .widgetList<RichText>(find.byType(RichText))
        .map((widget) => widget.text.toPlainText())
        .join();
    expect(text, contains('hello'));
    expect(text, contains(':unknown:'));
    expect(text, isNot(contains('**hello**')));
  });

  testWidgets('referenced event emoji stays available with tag URL priority', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        ':wave:',
        tags: const [
          ['emoji', 'wave', 'https://example.com/event-wave.png'],
        ],
      ),
    );
    final image = tester.widget<CustomEmojiImage>(
      find.byType(CustomEmojiImage),
    );
    expect(image.shortcode, 'wave');
    expect(image.url, 'https://example.com/event-wave.png');
    final markdown = tester.widget<GptMarkdown>(find.byType(GptMarkdown));
    final component = markdown.inlineComponents!
        .whereType<CustomEmojiMd>()
        .single;
    expect(component.exp.hasMatch(':unused_2499:'), isFalse);
    expect(component.exp.hasMatch(':wave:'), isTrue);
  });

  testWidgets('content edits rebuild the scoped matcher', (tester) async {
    await tester.pumpWidget(_testable('plain message'));
    expect(find.byType(CustomEmojiImage), findsNothing);

    await tester.pumpWidget(_testable('edited :wave:'));
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).shortcode,
      'wave',
    );

    await tester.pumpWidget(_testable('edited :unused_2499:'));
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).shortcode,
      'unused_2499',
    );
    final markdown = tester.widget<GptMarkdown>(find.byType(GptMarkdown));
    final component = markdown.inlineComponents!
        .whereType<CustomEmojiMd>()
        .single;
    expect(component.exp.hasMatch(':wave:'), isFalse);

    await tester.pumpWidget(_testable('edited :unknown:'));
    expect(find.byType(CustomEmojiImage), findsNothing);
  });

  testWidgets('code keeps literal emoji while adjacent known tokens render', (
    tester,
  ) async {
    await tester.pumpWidget(_testable('`:wave:` :unknown:wave:'));
    expect(find.byType(CustomEmojiImage), findsOneWidget);
    final text = tester
        .widgetList<RichText>(find.byType(RichText))
        .map((widget) => widget.text.toPlainText())
        .join();
    expect(text, contains(':wave:'));
    expect(text, contains(':unknown'));
  });
}
