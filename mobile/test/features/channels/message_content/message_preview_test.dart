import 'package:buzz/features/channels/channel_ids_by_name_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_mention_pill.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:buzz/shared/mentions/mention_identity_links.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_agent_icon.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

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

  testWidgets('signed names that fold alike leave a plain mention as text', (
    tester,
  ) async {
    const sam =
        'd000000000000000000000000000000000000000000000000000000000000000';
    for (final names in [
      {sam: 'Sam', _alice: 'ſam'},
      {_alice: 'ſam', sam: 'Sam'},
    ]) {
      await tester.pumpWidget(
        _testable(
          MessagePreview(
            content: 'hi @Sam',
            mentionNames: names,
            tags: [
              for (final key in names.keys) ['p', key],
            ],
          ),
        ),
      );
      expect(find.byType(MessageMentionPill), findsNothing, reason: '$names');
      expect(_plainText(tester), 'hi @Sam');
    }
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

  testWidgets('a mention cut off by the ellipsis fetches no picture', (
    tester,
  ) async {
    const bob =
        'b0b0000000000000000000000000000000000000000000000000000000000000';
    final requests = <String>[];
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          channelIdsByNameProvider.overrideWith(_ChannelNames.new),
          customEmojiListProvider.overrideWithValue(const []),
          myPubkeyProvider.overrideWithValue(_alice),
          userCacheProvider.overrideWith(_BobProfile.new),
          mediaGetAuthServiceProvider.overrideWithValue(
            MediaGetAuthService(
              baseUrl: 'https://relay.example.org',
              nsec: null,
            ),
          ),
          mediaDiskCacheProvider.overrideWithValue(null),
          mediaHttpClientProvider.overrideWithValue(
            MockClient((request) async {
              requests.add(request.url.toString());
              return http.Response('', 404);
            }),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: SizedBox(
              width: 180,
              child: MessagePreview(
                content:
                    'A long preview that wraps before the mention at the very '
                    'end of this one source line ${mentionIdentityLink('@Bob', bob)}',
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(MessageMentionPill), findsOneWidget);
    expect(requests, isEmpty);
  });

  testWidgets('an agent chip is scaled once by its span', (tester) async {
    await tester.pumpWidget(
      _testable(
        const MediaQuery(
          data: MediaQueryData(textScaler: TextScaler.linear(2)),
          child: MessagePreview(
            content: 'ask @Bot',
            mentionNames: {_alice: 'Bot'},
            agentMentionPubkeys: {_alice},
            tags: [
              ['p', _alice],
            ],
          ),
        ),
      ),
    );

    final style = tester.widget<MessageMentionPill>(
      find.byType(MessageMentionPill),
    );
    final icon = tester.widget<BuzzAgentIcon>(find.byType(BuzzAgentIcon));
    expect(icon.size, style.textStyle!.fontSize);
  });
}

class _BobProfile extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    'b0b0000000000000000000000000000000000000000000000000000000000000':
        UserProfile(
          pubkey:
              'b0b0000000000000000000000000000000000000000000000000000000000000',
          avatarUrl: 'https://example.org/bob.png',
        ),
  };
}
