import 'package:buzz/features/channels/channel_ids_by_name_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_mention_pill.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _ChannelNames extends ChannelIdsByName {
  @override
  Map<String, String> build() => const {'general': 'ch-general'};
}

const _alice =
    'a11ce00000000000000000000000000000000000000000000000000000000000';

Widget _testable(Widget child) => ProviderScope(
  overrides: [
    channelIdsByNameProvider.overrideWith(_ChannelNames.new),
    customEmojiListProvider.overrideWithValue([
      const CustomEmoji(shortcode: 'wave', url: 'https://example.com/w.png'),
    ]),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(body: child),
  ),
);

String _plainText(WidgetTester tester) =>
    tester.widget<RichText>(find.byType(RichText).first).text.toPlainText();

void main() {
  testWidgets('renders chips and emoji in one capped paragraph', (
    tester,
  ) async {
    await tester.pumpWidget(
      _testable(
        const MessagePreview(
          content: '**Hi** @Alice :wave: see #general\nline two\nline three',
          mentionNames: {_alice: 'Alice'},
        ),
      ),
    );

    expect(find.byType(GptMarkdown), findsNothing);
    expect(find.byType(MessageMentionPill), findsOneWidget);
    expect(find.byType(CustomEmojiImage), findsOneWidget);
    expect(find.text('general'), findsOneWidget);
    final text = tester.widget<RichText>(find.byType(RichText).first);
    expect(text.maxLines, 2);
    expect(text.overflow, TextOverflow.ellipsis);
    expect(_plainText(tester), startsWith('Hi '));
    expect(_plainText(tester), isNot(contains('**')));
  });

  testWidgets('labels media by kind', (tester) async {
    await tester.pumpWidget(
      _testable(
        const MessagePreview(
          content: 'https://example.com/photo.jpg',
          tags: [
            ['imeta', 'url https://example.com/photo.jpg', 'm image/jpeg'],
          ],
        ),
      ),
    );

    expect(_plainText(tester), 'Image');
  });

  testWidgets('renders nothing for empty content', (tester) async {
    await tester.pumpWidget(_testable(const MessagePreview(content: '')));

    expect(find.byType(RichText), findsNothing);
  });
}
