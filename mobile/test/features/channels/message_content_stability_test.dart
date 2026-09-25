import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_render.dart';
import 'package:buzz/shared/emoji/emoji_only.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

class _Fixture {
  final revision = ValueNotifier(0);
  String content = 'Hello @Alice, visit #sample.';
  Map<String, String> mentions = {'alice-key': 'Alice'};
  final channels = {'sample': 'channel-one'};
  Set<String> agents = {};
  List<List<String>> tags = [];
  List<CustomEmoji> palette = [];
  bool scaleEmoji = false;
  bool mentionHandler = true;
  bool mediaHandlers = false;
  int? tappedRevision;
  String? tappedId;

  Widget build() => ProviderScope(
    overrides: [customEmojiListProvider.overrideWith((ref) => palette)],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: AppMarkdownTheme(
        child: Scaffold(
          body: ValueListenableBuilder<int>(
            valueListenable: revision,
            builder: (_, generation, _) => MessageContent(
              content: content,
              mentionNames: Map.of(mentions),
              channelNames: Map.of(channels),
              agentMentionPubkeys: Set.of(agents),
              tags: tags,
              scaleEmojiOnly: scaleEmoji,
              onMediaReply: mediaHandlers
                  ? () {
                      tappedRevision = generation;
                      tappedId = 'reply';
                    }
                  : null,
              onMediaMore: mediaHandlers
                  ? (_, url) {
                      tappedRevision = generation;
                      tappedId = url;
                    }
                  : null,
              onMentionTap: mentionHandler
                  ? (id) {
                      tappedRevision = generation;
                      tappedId = id;
                    }
                  : null,
              onChannelTap: (id) {
                tappedRevision = generation;
                tappedId = id;
              },
            ),
          ),
        ),
      ),
    ),
  );

  Future<void> refresh(WidgetTester tester) async {
    revision.value++;
    await tester.pump();
  }
}

List<MarkdownComponent> _components(WidgetTester tester) => tester
    .widget<GptMarkdown>(find.byType(GptMarkdown).first)
    .inlineComponents!;

void main() {
  testWidgets('explicit channel links retain the current callback', (
    tester,
  ) async {
    const id = '580ca78b-9dae-46f3-8854-bd671853ba32';
    final fixture = _Fixture()..content = '[Open channel](buzz://channel/$id)';
    addTearDown(fixture.revision.dispose);
    await tester.pumpWidget(fixture.build());
    final before = _components(tester);
    await fixture.refresh(tester);
    expect(_components(tester), same(before));
    await tester.tap(find.text('Open channel', findRichText: true));
    expect(fixture.tappedId, id);
    expect(fixture.tappedRevision, fixture.revision.value);
  });

  testWidgets(
    'inline media retains current actions and invalidates metadata and action presence',
    (tester) async {
      const url = 'https://relay.example/image.png';
      final fixture = _Fixture()
        ..content = '![image]($url)\n\nAfter image'
        ..mediaHandlers = true
        ..tags = [
          ['imeta', 'url $url', 'm image/png', 'alt First label'],
        ];
      addTearDown(fixture.revision.dispose);
      await tester.pumpWidget(fixture.build());
      final before = _components(tester);
      await fixture.refresh(tester);
      expect(_components(tester), same(before));
      await tester.tap(
        find.byKey(const ValueKey('message-media-image-preview:$url')),
      );
      await tester.pumpAndSettle();
      var viewer = tester.widget<MediaImageViewerPage>(
        find.byType(MediaImageViewerPage),
      );
      viewer.onReply!();
      expect(fixture.tappedId, 'reply');
      expect(fixture.tappedRevision, fixture.revision.value);
      viewer.onMore!(tester.element(find.byType(MediaImageViewerPage)), url);
      expect(fixture.tappedId, url);
      expect(fixture.tappedRevision, fixture.revision.value);
      Navigator.of(tester.element(find.byType(MediaImageViewerPage))).pop();
      await tester.pumpAndSettle();
      fixture.tags[0][3] = 'alt Second label';
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(before)));
      await tester.tap(
        find.byKey(const ValueKey('message-media-image-preview:$url')),
      );
      await tester.pumpAndSettle();
      viewer = tester.widget<MediaImageViewerPage>(
        find.byType(MediaImageViewerPage),
      );
      expect(viewer.semanticLabel, 'Second label');
      Navigator.of(tester.element(find.byType(MediaImageViewerPage))).pop();
      await tester.pumpAndSettle();
      final beforeRemoval = _components(tester);
      fixture.mediaHandlers = false;
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(beforeRemoval)));
      await tester.tap(
        find.byKey(const ValueKey('message-media-image-preview:$url')),
      );
      await tester.pumpAndSettle();
      viewer = tester.widget<MediaImageViewerPage>(
        find.byType(MediaImageViewerPage),
      );
      expect(viewer.onReply, isNull);
      expect(viewer.onMore, isNull);
    },
  );

  testWidgets(
    'equal presentation retains parser components and current callbacks',
    (tester) async {
      final fixture = _Fixture();
      addTearDown(fixture.revision.dispose);
      await tester.pumpWidget(fixture.build());
      final before = _components(tester);
      for (var i = 0; i < 3; i++) {
        await fixture.refresh(tester);
        // This is the production dependency seam: gpt_markdown's config.isSame
        // compares these component identities before deciding whether to parse.
        expect(_components(tester), same(before));
      }
      await tester.tap(find.text('Alice'));
      expect(fixture.tappedRevision, fixture.revision.value);
      expect(fixture.tappedId, 'alice-key');
      await tester.tap(find.text('sample'));
      expect(fixture.tappedRevision, fixture.revision.value);
      expect(fixture.tappedId, 'channel-one');

      fixture.channels['sample'] = 'channel-two';
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(before)));
      await tester.tap(find.text('sample'));
      expect(fixture.tappedId, 'channel-two');
    },
  );

  testWidgets(
    'mention labels, bindings, agent status and handler presence invalidate',
    (tester) async {
      final fixture = _Fixture();
      addTearDown(fixture.revision.dispose);
      await tester.pumpWidget(fixture.build());
      var before = _components(tester);
      fixture.mentions['alice-key'] = 'ALICE';
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(before)));
      expect(find.text('ALICE'), findsOneWidget);
      before = _components(tester);
      fixture.agents.add('alice-key');
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(before)));
      expect(find.byIcon(LucideIcons.bot), findsOneWidget);
      before = _components(tester);
      fixture.mentionHandler = false;
      fixture.tappedId = null;
      await fixture.refresh(tester);
      expect(_components(tester), isNot(same(before)));
      await tester.tap(find.text('ALICE'));
      expect(fixture.tappedId, isNull);
      fixture.mentionHandler = true;
      fixture.mentions = {'second-key': 'ALICE'};
      await fixture.refresh(tester);
      await tester.tap(find.text('ALICE'));
      expect(fixture.tappedId, 'second-key');
      expect(find.byIcon(LucideIcons.bot), findsNothing);
    },
  );

  testWidgets('signed qualified mention changes cannot retain an old binding', (
    tester,
  ) async {
    final key = 'a' * 64;
    final fixture = _Fixture()
      ..content = '@Alice ($key)'
      ..mentions = {}
      ..tags = [
        ['p', key],
      ];
    addTearDown(fixture.revision.dispose);
    await tester.pumpWidget(fixture.build());
    final before = _components(tester);
    await tester.tap(find.text('Alice (aaaaaaaa…aaaa)'));
    expect(fixture.tappedId, key);
    fixture.tags = [];
    fixture.tappedId = null;
    await fixture.refresh(tester);
    expect(_components(tester), isNot(same(before)));
    expect(find.text('Alice (aaaaaaaa…aaaa)'), findsNothing);
  });

  testWidgets('community emoji palette and emoji-only size invalidate', (
    tester,
  ) async {
    final fixture = _Fixture()
      ..content = ':sample:'
      ..palette = [
        const CustomEmoji(
          shortcode: 'sample',
          url: 'https://relay.example/one.png',
        ),
      ];
    addTearDown(fixture.revision.dispose);
    await tester.pumpWidget(fixture.build());
    var before = _components(tester);
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).url,
      'https://relay.example/one.png',
    );
    fixture.palette = [
      const CustomEmoji(
        shortcode: 'sample',
        url: 'https://relay.example/two.png',
      ),
    ];
    ProviderScope.containerOf(
      tester.element(find.byType(MessageContent)),
    ).invalidate(customEmojiListProvider);
    await fixture.refresh(tester);
    expect(_components(tester), isNot(same(before)));
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).url,
      'https://relay.example/two.png',
    );
    before = _components(tester);
    fixture.scaleEmoji = true;
    await fixture.refresh(tester);
    expect(_components(tester), isNot(same(before)));
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).size,
      kEmojiOnlyCustomEmojiSize,
    );
    before = _components(tester);
    fixture.tags = [
      ['emoji', 'sample', 'https://relay.example/tag.png'],
    ];
    await fixture.refresh(tester);
    expect(_components(tester), isNot(same(before)));
    expect(
      tester.widget<CustomEmojiImage>(find.byType(CustomEmojiImage)).url,
      'https://relay.example/tag.png',
    );
  });
}
