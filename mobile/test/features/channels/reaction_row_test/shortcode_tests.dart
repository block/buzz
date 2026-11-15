part of '../reaction_row_test.dart';

/// A reaction slot never renders an emoji shortcode as text. A tagless
/// `:shortcode:` (an agent's reaction) resolves against the community palette;
/// one with no image, or whose image fails to load, is the neutral placeholder.
void shortcodeReactionTests() {
  const approveUrl = 'https://relay.example/media/approve.png';
  const brokenUrl = 'https://relay.example/media/broken.png';
  final palette = [
    const CustomEmoji(shortcode: 'approve', url: approveUrl),
    const CustomEmoji(shortcode: 'broken', url: brokenUrl),
  ];
  final png = base64Decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
  );
  // Tagless, as an agent sends them: no NIP-30 `emoji` tag on the event.
  final reactions = [
    _reaction(emoji: ':approve:'),
    _reaction(emoji: ':broken:'),
    _reaction(emoji: ':ghost:'),
    _reaction(emoji: _fire),
  ];

  List<Override> overrides(MockClient client) => [
    emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
    customEmojiListProvider.overrideWithValue(palette),
    mediaHttpClientProvider.overrideWithValue(client),
  ];

  void expectNoShortcodeText(WidgetTester tester, Finder scope) {
    for (final shortcode in [':approve:', ':broken:', ':ghost:']) {
      expect(
        find.descendant(of: scope, matching: find.text(shortcode)),
        findsNothing,
      );
    }
  }

  testWidgets('a reaction slot never shows a shortcode as text', (
    tester,
  ) async {
    final client = MockClient(
      (request) async => request.url.toString() == approveUrl
          ? http.Response.bytes(png, 200)
          : http.Response('missing', 404),
    );
    addTearDown(client.close);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: overrides(client),
        child: ReactionRow(
          messageId: _messageId,
          channelId: 'channel',
          reactions: reactions,
          onToggle: (_) {},
        ),
      ),
    );
    await tester.pumpAndSettle();

    // The tagless known shortcode renders the palette's image.
    final approve = find.byType(CustomEmojiImage).first;
    expect(tester.widget<CustomEmojiImage>(approve).url, approveUrl);
    await tester.runAsync(
      () => precacheImage(
        tester
            .widget<Image>(
              find.descendant(of: approve, matching: find.byType(Image)),
            )
            .image,
        tester.element(approve),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<RawImage>(
            find.descendant(of: approve, matching: find.byType(RawImage)),
          )
          .image,
      isNotNull,
    );

    // A failed image and an unknown shortcode are both the placeholder, and
    // the unicode emoji is untouched.
    final pills = find.byType(ReactionRow);
    expectNoShortcodeText(tester, pills);
    expect(
      find.descendant(of: pills, matching: find.byIcon(BuzzIcons.sparkles)),
      findsNWidgets(2),
    );
    expect(find.byType(NativeEmojiGlyph), findsOneWidget);
    expect(find.text(_fire), findsOneWidget);

    // The detail sheet follows the same rule.
    await tester.longPress(find.byKey(const ValueKey('reaction-pill-:ghost:')));
    await tester.pumpAndSettle();
    final sheet = find.byKey(const ValueKey('reaction-details-sheet'));
    expect(sheet, findsOneWidget);
    expectNoShortcodeText(tester, sheet);
    expect(
      find.descendant(of: sheet, matching: find.byType(CustomEmojiImage)),
      findsNWidgets(4),
    );
    expect(
      find.descendant(of: sheet, matching: find.byIcon(BuzzIcons.sparkles)),
      findsNWidgets(4),
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('the native sheet is told which shortcodes have no image', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    Map<Object?, Object?>? payload;
    final completion = Completer<Map<String, Object?>>();
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        if (call.method == 'reactions') {
          payload = call.arguments as Map<Object?, Object?>;
          return completion.future;
        }
        return <String, Object?>{};
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    final client = MockClient((_) async => http.Response('missing', 404));
    addTearDown(client.close);
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: overrides(client),
        child: ReactionRow(
          messageId: _messageId,
          channelId: 'channel',
          reactions: reactions,
          onToggle: (_) {},
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.longPress(find.byKey(const ValueKey('reaction-pill-:ghost:')));
    await tester.pumpAndSettle();

    final sent = {
      for (final reaction in payload!['reactions'] as List)
        (reaction as Map)['emoji']: reaction,
    };
    expect(sent[':approve:']!['url'], approveUrl);
    expect(sent[':approve:']!['isShortcode'], isTrue);
    expect(sent[':ghost:']!.containsKey('url'), isFalse);
    expect(sent[':ghost:']!['isShortcode'], isTrue);
    expect(sent[_fire]!.containsKey('url'), isFalse);
    expect(sent[_fire]!.containsKey('isShortcode'), isFalse);
    completion.complete(<String, Object?>{});
    await tester.pumpAndSettle();
    debugDefaultTargetPlatformOverride = null;
  });
}
