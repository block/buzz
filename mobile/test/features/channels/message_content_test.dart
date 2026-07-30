import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/emoji/emoji_only.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';

Widget _testable(
  Widget child, {
  List<Override> overrides = const [],
  bool disableAnimations = false,
}) {
  return ProviderScope(
    overrides: overrides,
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(disableAnimations: disableAnimations),
          child: Scaffold(body: child),
        ),
      ),
    ),
  );
}

void _setSurfaceSize(WidgetTester tester, Size size) {
  tester.view.devicePixelRatio = 1.0;
  tester.view.physicalSize = size;
}

Finder _imagePreview(String imageUrl) {
  return find.byKey(ValueKey('message-media-image-preview:$imageUrl'));
}

Finder _imageViewerHeroMode() {
  return find.byKey(const ValueKey('message-media-image-viewer-hero-mode'));
}

Future<TransformationController> _openImageViewer(
  WidgetTester tester,
  String imageUrl,
) async {
  await tester.tap(_imagePreview(imageUrl));
  await tester.pumpAndSettle();

  final interactiveViewer = tester.widget<InteractiveViewer>(
    find.byType(InteractiveViewer),
  );
  final transformationController = interactiveViewer.transformationController;

  expect(transformationController, isNotNull);
  return transformationController!;
}

void _applyImageViewerTransform(
  TransformationController controller, {
  required double dx,
  required double dy,
  required double scale,
}) {
  controller.value = Matrix4.identity()
    ..translateByDouble(dx, dy, 0, 1)
    ..scaleByDouble(scale, scale, scale, 1);
}

bool _isImageViewerHeroEnabled(WidgetTester tester) {
  return tester.widget<HeroMode>(_imageViewerHeroMode()).enabled;
}

/// Extracts all plain text from all RichText widgets in the tree.
String _allRichText(WidgetTester tester) {
  final richTexts = tester.widgetList<RichText>(find.byType(RichText));
  return richTexts.map((rt) => rt.text.toPlainText()).join('\n');
}

/// Finds a RichText widget whose plain text contains [text].
Finder _findRich(String text) {
  return find.byWidgetPredicate(
    (widget) => widget is RichText && widget.text.toPlainText().contains(text),
    description: 'RichText containing "$text"',
  );
}

/// Checks that the given text appears as bold (fontWeight >= w600) in some
/// TextSpan within any RichText widget.
bool _hasBoldSpan(WidgetTester tester, String text) {
  for (final rt in tester.widgetList<RichText>(find.byType(RichText))) {
    if (_spanHasStyle(
      rt.text,
      text,
      (s) =>
          s.fontWeight != null && s.fontWeight!.value >= FontWeight.w600.value,
    )) {
      return true;
    }
  }
  return false;
}

bool _hasItalicSpan(WidgetTester tester, String text) {
  for (final rt in tester.widgetList<RichText>(find.byType(RichText))) {
    if (_spanHasStyle(rt.text, text, (s) => s.fontStyle == FontStyle.italic)) {
      return true;
    }
  }
  return false;
}

bool _hasStrikethroughSpan(WidgetTester tester, String text) {
  for (final rt in tester.widgetList<RichText>(find.byType(RichText))) {
    if (_spanHasStyle(
      rt.text,
      text,
      (s) => s.decoration == TextDecoration.lineThrough,
    )) {
      return true;
    }
  }
  return false;
}

bool _spanHasStyle(
  InlineSpan root,
  String text,
  bool Function(TextStyle) check,
) {
  var found = false;
  root.visitChildren((span) {
    if (span is TextSpan &&
        span.text != null &&
        span.text!.contains(text) &&
        span.style != null &&
        check(span.style!)) {
      found = true;
      return false; // stop visiting
    }
    return true;
  });
  return found;
}

void main() {
  group('MessageContent', () {
    testWidgets('opens local file links through an authenticated download', (
      tester,
    ) async {
      const url = 'https://relay.example/media/report.pdf';
      String? openedUrl;
      Map<String, String>? openedHeaders;
      String? openedFilename;
      final auth = MediaGetAuthService(
        baseUrl: 'https://relay.example',
        nsec: nostr.Keys.generate().nsec,
      );

      await tester.pumpWidget(
        _testable(
          const MessageContent(content: '[report.pdf]($url)'),
          overrides: [
            mediaGetAuthServiceProvider.overrideWithValue(auth),
            openDownloadedFileProvider.overrideWithValue((
              url,
              headers,
              filename,
            ) async {
              openedUrl = url;
              openedHeaders = headers;
              openedFilename = filename;
            }),
          ],
        ),
      );

      await tester.tap(find.text('report.pdf'));
      await tester.pump();

      expect(openedUrl, url);
      expect(openedFilename, 'report.pdf');
      expect(openedHeaders?['Authorization'], startsWith('Nostr '));
    });

    test('buildImageViewerRoute uses modal-style page route builder', () {
      final route = buildImageViewerRoute(
        imageUrl: 'https://example.com/media/image.png',
        heroTag: Object(),
      );

      expect(route, isA<PageRouteBuilder<void>>());
      expect(route.transitionDuration, const Duration(milliseconds: 260));
      expect(
        route.reverseTransitionDuration,
        const Duration(milliseconds: 170),
      );
    });

    test('buildImageViewerRoute disables motion when requested', () {
      final route = buildImageViewerRoute(
        imageUrl: 'https://example.com/media/image.png',
        heroTag: Object(),
        disableAnimations: true,
      );

      expect(route.transitionDuration, Duration.zero);
      expect(route.reverseTransitionDuration, Duration.zero);
    });

    group('plain text', () {
      testWidgets('renders simple text', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'Hello world')),
        );

        expect(_findRich('Hello world'), findsOneWidget);
      });

      testWidgets('renders empty content', (tester) async {
        await tester.pumpWidget(_testable(const MessageContent(content: '')));

        // Should not crash.
        expect(find.byType(MessageContent), findsOneWidget);
      });
    });

    group('custom emoji', () {
      testWidgets('renders tagged custom emoji as inline image', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Narf :shipit:',
              tags: [
                ['emoji', 'shipit', 'https://relay.example/shipit.png'],
              ],
            ),
          ),
        );

        expect(find.byType(Image), findsOneWidget);
        final image = tester.widget<Image>(find.byType(Image));
        expect(image.semanticLabel, ':shipit:');
        expect(_allRichText(tester), contains('Narf'));
        expect(_allRichText(tester), isNot(contains(':shipit:')));
      });

      testWidgets('leaves untagged custom emoji shortcode as text', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'Missing :shipit:')),
        );

        expect(find.byType(Image), findsNothing);
        expect(_allRichText(tester), contains(':shipit:'));
      });
    });

    group('emoji-only bodies', () {
      /// Font size the body text actually rendered at.
      ///
      /// gpt_markdown resolves the style onto the spans rather than the root
      /// RichText, which keeps the ambient 14px — so read the span carrying the
      /// text, not the root.
      double bodyFontSize(WidgetTester tester, String text) {
        final richText = tester.widget<RichText>(_findRich(text).first);
        double? size;
        richText.text.visitChildren((span) {
          if (span is TextSpan && (span.text ?? '').contains(text)) {
            size ??= span.style?.fontSize;
          }
          return size == null;
        });
        return size ?? richText.text.style!.fontSize!;
      }

      testWidgets('an all-emoji body renders larger, like desktop', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: '\u{1F389}', scaleEmojiOnly: true),
          ),
        );

        expect(bodyFontSize(tester, '\u{1F389}'), kEmojiOnlyFontSize);
      });

      testWidgets('one word alongside the emoji keeps body size', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'ship it \u{1F389}',
              scaleEmojiOnly: true,
            ),
          ),
        );

        expect(bodyFontSize(tester, 'ship it'), lessThan(kEmojiOnlyFontSize));
      });

      testWidgets('the scale is opt-in, so previews are unaffected', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: '\u{1F389}')),
        );

        expect(bodyFontSize(tester, '\u{1F389}'), lessThan(kEmojiOnlyFontSize));
      });

      testWidgets('a tagged custom emoji alone scales its image too', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: ':shipit:',
              scaleEmojiOnly: true,
              tags: [
                ['emoji', 'shipit', 'https://relay.example/shipit.png'],
              ],
            ),
          ),
        );

        final image = tester.widget<Image>(find.byType(Image));
        expect(image.height, kEmojiOnlyCustomEmojiSize);
      });

      testWidgets('an unresolvable shortcode stays body size', (tester) async {
        // Without a matching emoji tag it renders as literal text, so scaling it
        // would blow up a `:word:` that was never emoji at all.
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: ':shipit:', scaleEmojiOnly: true),
          ),
        );

        expect(bodyFontSize(tester, ':shipit:'), lessThan(kEmojiOnlyFontSize));
      });
    });

    group('inline formatting', () {
      testWidgets('renders bold text', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'This is **bold** text')),
        );

        final allText = _allRichText(tester);
        expect(allText, contains('bold'));
        expect(allText, isNot(contains('**')));
        expect(_hasBoldSpan(tester, 'bold'), isTrue);
      });

      testWidgets('renders italic text', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'This is *italic* text')),
        );

        final allText = _allRichText(tester);
        expect(allText, contains('italic'));
        expect(_hasItalicSpan(tester, 'italic'), isTrue);
      });

      testWidgets('renders strikethrough text', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'This is ~~struck~~ text')),
        );

        final allText = _allRichText(tester);
        expect(allText, contains('struck'));
        expect(allText, isNot(contains('~~')));
        expect(_hasStrikethroughSpan(tester, 'struck'), isTrue);
      });

      testWidgets('renders inline code', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: 'Use `flutter test` to run')),
        );

        // Inline code is rendered inside a styled span.
        expect(_findRich('flutter test'), findsWidgets);
      });

      testWidgets('renders markdown link', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Check [Buzz](https://example.com)'),
          ),
        );

        final allText = _allRichText(tester);
        expect(allText, contains('Buzz'));
        // Should not show raw markdown syntax.
        expect(allText, isNot(contains('[Buzz]')));
        expect(allText, isNot(contains('(https://example.com)')));
      });

      testWidgets('renders bare URL as link', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Visit https://example.com today'),
          ),
        );

        // The URL text should be rendered and tappable.
        expect(find.text('https://example.com'), findsOneWidget);
        final urlWidget = tester.widget<Text>(find.text('https://example.com'));
        expect(urlWidget.style?.decoration, TextDecoration.underline);
      });
    });

    group('code blocks', () {
      testWidgets('renders fenced code block', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Before\n```\ncode here\n```\nAfter'),
          ),
        );

        expect(_findRich('code here'), findsWidgets);
        expect(_findRich('Before'), findsWidgets);
        expect(_findRich('After'), findsWidgets);
      });

      testWidgets('renders code block with language tag', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: '```dart\nvoid main() {}\n```'),
          ),
        );

        expect(_findRich('void main() {}'), findsWidgets);
        expect(find.text('dart'), findsOneWidget);
      });
    });

    group('media attachments', () {
      testWidgets(
        'renders image markdown as a media preview and opens viewer',
        (tester) async {
          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content: 'Look\n![image](https://example.com/media/image.png)',
                tags: [
                  [
                    'imeta',
                    'url https://example.com/media/image.png',
                    'm image/png',
                  ],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final preview = find.byKey(
            const ValueKey(
              'message-media-image-preview:https://example.com/media/image.png',
            ),
          );
          expect(preview, findsOneWidget);

          await tester.tap(preview);
          await tester.pumpAndSettle();

          final viewer = tester.widget<Scaffold>(
            find.byKey(const ValueKey('message-media-image-viewer')),
          );

          expect(
            find.byKey(const ValueKey('message-media-image-viewer')),
            findsOneWidget,
          );
          expect(viewer.backgroundColor, Colors.black);
          expect(find.byType(AppBar), findsNothing);
          expect(
            find.byKey(const ValueKey('message-media-image-viewer-close')),
            findsOneWidget,
          );

          await tester.tap(
            find.byKey(const ValueKey('message-media-image-viewer-close')),
          );
          await tester.pumpAndSettle();

          expect(
            find.byKey(const ValueKey('message-media-image-viewer')),
            findsNothing,
          );
        },
      );

      testWidgets('uses unique hero tags for repeated identical image urls', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '''
![image](https://example.com/media/repeated.png)
![image](https://example.com/media/repeated.png)
''',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/repeated.png',
                  'm image/png',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();

        final heroes = tester.widgetList<Hero>(find.byType(Hero)).toList();
        final heroTags = heroes.map((hero) => hero.tag).toSet();

        expect(heroes, hasLength(2));
        expect(heroTags, hasLength(2));

        await tester.tap(find.byType(Image).first);
        await tester.pumpAndSettle();

        expect(tester.takeException(), isNull);
        expect(
          find.byKey(const ValueKey('message-media-image-viewer')),
          findsOneWidget,
        );
      });

      testWidgets(
        'groups uploaded photos into a carousel and opens the full gallery',
        (tester) async {
          const first = 'https://example.com/media/one.png';
          const second = 'https://example.com/media/two.png';
          const third = 'https://example.com/media/three.png';
          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content:
                    '''
Photos
![image]($first)
![image]($second)
![image]($third)
''',
                tags: [
                  ['imeta', 'url $first', 'm image/png', 'alt First photo'],
                  ['imeta', 'url $second', 'm image/png', 'alt Second photo'],
                  ['imeta', 'url $third', 'm image/png', 'alt Third photo'],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final carousel = find.byKey(const ValueKey('message-media-carousel'));
          expect(carousel, findsOneWidget);
          expect(find.text('3 images'), findsOneWidget);

          await tester.drag(carousel, const Offset(-600, 0));
          await tester.pumpAndSettle();

          await tester.tap(
            find.byKey(const ValueKey('message-media-carousel-item:$second')),
          );
          await tester.pumpAndSettle();

          expect(
            find.byKey(const ValueKey('message-media-image-viewer')),
            findsOneWidget,
          );
          expect(
            find.byKey(const ValueKey('message-media-image-viewer-filmstrip')),
            findsOneWidget,
          );
          expect(
            find.byKey(
              const ValueKey('message-media-image-viewer-thumbnail:1'),
            ),
            findsOneWidget,
          );
          final displayedImage = tester.widget<MediaImage>(
            find.byKey(const ValueKey('message-media-image-viewer-image:1')),
          );
          expect(displayedImage.decodeWidth, isNotNull);
          final selectedThumbnailClip = tester.widget<ClipRRect>(
            find.byKey(
              const ValueKey('message-media-image-viewer-thumbnail-clip:1'),
            ),
          );
          final selectedThumbnailRadius =
              selectedThumbnailClip.borderRadius as BorderRadius;
          expect(
            selectedThumbnailRadius.topLeft.x,
            closeTo(Radii.sm - 2.5, 0.01),
          );

          await tester.fling(
            find.byKey(const ValueKey('message-media-image-viewer-pages')),
            const Offset(-700, 0),
            1200,
          );
          await tester.pumpAndSettle();

          final thirdThumbnail = find.byKey(
            const ValueKey('message-media-image-viewer-thumbnail:2'),
          );
          final thirdSemantics = tester.widget<Semantics>(
            find
                .ancestor(of: thirdThumbnail, matching: find.byType(Semantics))
                .first,
          );
          expect(thirdSemantics.properties.selected, isTrue);
        },
      );

      testWidgets(
        'keeps adjacent carousel images active and ends with a gutter',
        (tester) async {
          const first = 'https://example.com/media/gutter-one.png';
          const second = 'https://example.com/media/gutter-two.png';
          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content:
                    '''
![image]($first)
![image]($second)
''',
                tags: [
                  ['imeta', 'url $first', 'm image/png'],
                  ['imeta', 'url $second', 'm image/png'],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final carousel = find.byKey(const ValueKey('message-media-carousel'));
          final pageViewFinder = find.descendant(
            of: carousel,
            matching: find.byType(PageView),
          );
          final pageView = tester.widget<PageView>(pageViewFinder);

          expect(pageView.allowImplicitScrolling, isTrue);
          expect(pageView.clipBehavior, Clip.none);

          pageView.controller!.jumpToPage(1);
          await tester.pumpAndSettle();

          final lastCard = find.byKey(
            const ValueKey('message-media-carousel-item:$second'),
          );
          expect(
            tester.getRect(carousel).right - tester.getRect(lastCard).right,
            Grid.gutter,
          );
        },
      );

      testWidgets(
        'jumps to a selected gallery thumbnail when motion is disabled',
        (tester) async {
          const first = 'https://example.com/media/reduced-motion-one.png';
          const second = 'https://example.com/media/reduced-motion-two.png';
          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content:
                    '''
![image]($first)
![image]($second)
''',
                tags: [
                  ['imeta', 'url $first', 'm image/png'],
                  ['imeta', 'url $second', 'm image/png'],
                ],
              ),
              disableAnimations: true,
            ),
          );
          await tester.pumpAndSettle();

          await tester.tap(
            find.byKey(const ValueKey('message-media-carousel-item:$first')),
          );
          await tester.pumpAndSettle();
          await tester.tap(
            find.byKey(
              const ValueKey('message-media-image-viewer-thumbnail:1'),
            ),
          );
          await tester.pumpAndSettle();

          final selectedThumbnail = tester.widget<Semantics>(
            find
                .ancestor(
                  of: find.byKey(
                    const ValueKey('message-media-image-viewer-thumbnail:1'),
                  ),
                  matching: find.byType(Semantics),
                )
                .first,
          );
          expect(selectedThumbnail.properties.selected, isTrue);
          expect(tester.takeException(), isNull);
        },
      );

      testWidgets('resets carousel paging when gallery images change', (
        tester,
      ) async {
        const firstGallery = [
          'https://example.com/media/first-a.png',
          'https://example.com/media/first-b.png',
          'https://example.com/media/first-c.png',
        ];
        const secondGallery = [
          'https://example.com/media/second-a.png',
          'https://example.com/media/second-b.png',
        ];

        Widget gallery(List<String> urls) => _testable(
          MessageContent(
            content: urls.map((url) => '![image]($url)').join('\n'),
            tags: [
              for (final url in urls) ['imeta', 'url $url', 'm image/png'],
            ],
          ),
        );

        await tester.pumpWidget(gallery(firstGallery));
        await tester.pumpAndSettle();
        final firstCarousel = tester.widget<PageView>(
          find.descendant(
            of: find.byKey(const ValueKey('message-media-carousel')),
            matching: find.byType(PageView),
          ),
        );

        await tester.fling(
          find.byKey(const ValueKey('message-media-carousel')),
          const Offset(-700, 0),
          1200,
        );
        await tester.pumpAndSettle();
        expect(firstCarousel.controller!.page, greaterThan(0));

        await tester.pumpWidget(gallery(secondGallery));
        await tester.pumpAndSettle();
        final secondCarousel = tester.widget<PageView>(
          find.descendant(
            of: find.byKey(const ValueKey('message-media-carousel')),
            matching: find.byType(PageView),
          ),
        );

        expect(
          secondCarousel.controller,
          isNot(same(firstCarousel.controller)),
        );
        expect(secondCarousel.controller!.page, 0);
      });

      testWidgets(
        'disables hero on close after the fullscreen image is transformed',
        (tester) async {
          const imageUrl = 'https://example.com/media/transformed.png';

          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content:
                    'Look\n![image](https://example.com/media/transformed.png)',
                tags: [
                  [
                    'imeta',
                    'url https://example.com/media/transformed.png',
                    'm image/png',
                  ],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final transformationController = await _openImageViewer(
            tester,
            imageUrl,
          );

          expect(_isImageViewerHeroEnabled(tester), isTrue);

          _applyImageViewerTransform(
            transformationController,
            dx: 24.0,
            dy: 18.0,
            scale: 1.5,
          );
          await tester.pump();

          await tester.tap(
            find.byKey(const ValueKey('message-media-image-viewer-close')),
          );
          await tester.pump();

          expect(_isImageViewerHeroEnabled(tester), isFalse);

          await tester.pumpAndSettle();

          expect(tester.takeException(), isNull);
          expect(
            find.byKey(const ValueKey('message-media-image-viewer')),
            findsNothing,
          );
        },
      );

      testWidgets('double tap resets the fullscreen image transform', (
        tester,
      ) async {
        const imageUrl = 'https://example.com/media/double-tap-reset.png';

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content:
                  'Look\n![image](https://example.com/media/double-tap-reset.png)',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/double-tap-reset.png',
                  'm image/png',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();

        final transformationController = await _openImageViewer(
          tester,
          imageUrl,
        );
        _applyImageViewerTransform(
          transformationController,
          dx: 32,
          dy: 24,
          scale: 2,
        );
        await tester.pump();

        final gestureSurface = find.byKey(
          const ValueKey('message-media-image-viewer-gesture:0'),
        );
        await tester.tap(gestureSurface);
        await tester.pump(const Duration(milliseconds: 50));
        await tester.tap(gestureSurface);
        await tester.pumpAndSettle();

        expect(
          transformationController.value.storage,
          orderedEquals(Matrix4.identity().storage),
        );
        expect(_isImageViewerHeroEnabled(tester), isTrue);
      });

      testWidgets('swiping down dismisses the fullscreen gallery', (
        tester,
      ) async {
        const imageUrl = 'https://example.com/media/swipe-dismiss.png';

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content:
                  'Look\n![image](https://example.com/media/swipe-dismiss.png)',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/swipe-dismiss.png',
                  'm image/png',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();
        await _openImageViewer(tester, imageUrl);

        await tester.drag(
          find.byKey(const ValueKey('message-media-image-viewer-gesture:0')),
          const Offset(0, 180),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('message-media-image-viewer')),
          findsNothing,
        );
      });

      testWidgets(
        'disables hero on back navigation after the fullscreen image is transformed',
        (tester) async {
          const imageUrl = 'https://example.com/media/transformed-back.png';

          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content:
                    'Look\n![image](https://example.com/media/transformed-back.png)',
                tags: [
                  [
                    'imeta',
                    'url https://example.com/media/transformed-back.png',
                    'm image/png',
                  ],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final transformationController = await _openImageViewer(
            tester,
            imageUrl,
          );

          _applyImageViewerTransform(
            transformationController,
            dx: 32.0,
            dy: 20.0,
            scale: 1.4,
          );
          await tester.pump();

          final popRouteFuture = tester.binding.handlePopRoute();
          await tester.pump();

          await popRouteFuture;
          await tester.pumpAndSettle();

          expect(tester.takeException(), isNull);
          expect(
            find.byKey(const ValueKey('message-media-image-viewer')),
            findsNothing,
          );
        },
      );

      testWidgets('caps tall image previews to a bounded inline size', (
        tester,
      ) async {
        _setSurfaceSize(tester, const Size(400, 800));
        addTearDown(() {
          tester.view.resetPhysicalSize();
          tester.view.resetDevicePixelRatio();
        });

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![image](https://example.com/media/tall.png)',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/tall.png',
                  'm image/png',
                  'dim 1200x2400',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();

        final preview = find.byKey(
          const ValueKey(
            'message-media-image-preview:https://example.com/media/tall.png',
          ),
        );
        final size = tester.getSize(preview);

        expect(size.height, closeTo(240, 0.1));
        expect(size.width, closeTo(120, 0.1));
      });

      testWidgets(
        'keeps no-dim image previews max-bounded without fixed crop',
        (tester) async {
          _setSurfaceSize(tester, const Size(400, 800));
          addTearDown(() {
            tester.view.resetPhysicalSize();
            tester.view.resetDevicePixelRatio();
          });

          const previewKey = ValueKey(
            'message-media-image-preview:https://example.com/media/no-dim.png',
          );

          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content: '![image](https://example.com/media/no-dim.png)',
                tags: [
                  [
                    'imeta',
                    'url https://example.com/media/no-dim.png',
                    'm image/png',
                  ],
                ],
              ),
            ),
          );
          await tester.pump();

          final preview = tester.widget<Container>(find.byKey(previewKey));
          final image = tester.widget<Image>(
            find.descendant(
              of: find.byKey(previewKey),
              matching: find.byType(Image),
            ),
          );

          expect(preview.constraints, isNotNull);
          expect(preview.constraints!.minWidth, 0);
          expect(preview.constraints!.minHeight, 0);
          expect(preview.constraints!.maxWidth, closeTo(288, 0.1));
          expect(preview.constraints!.maxHeight, closeTo(240, 0.1));
          expect(image.fit, BoxFit.contain);
        },
      );

      testWidgets('renders video markdown as a video preview', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![video](https://example.com/media/clip.mp4)',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/clip.mp4',
                  'm video/mp4',
                  'image https://example.com/media/poster.jpg',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mp4',
            ),
          ),
          findsOneWidget,
        );
        expect(find.byIcon(LucideIcons.play), findsOneWidget);
      });

      testWidgets(
        'tapping video preview opens overlay viewer with close button',
        (tester) async {
          await tester.pumpWidget(
            _testable(
              const MessageContent(
                content: '![video](https://example.com/media/clip.mp4)',
                tags: [
                  [
                    'imeta',
                    'url https://example.com/media/clip.mp4',
                    'm video/mp4',
                  ],
                ],
              ),
            ),
          );
          await tester.pumpAndSettle();

          final preview = find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mp4',
            ),
          );
          expect(preview, findsOneWidget);

          await tester.tap(preview);
          await tester.pumpAndSettle();

          // Video viewer opens as a modal overlay (no AppBar)
          final viewer = tester.widget<Scaffold>(
            find.byKey(const ValueKey('message-media-video-viewer')),
          );
          expect(
            find.byKey(const ValueKey('message-media-video-viewer')),
            findsOneWidget,
          );
          expect(viewer.backgroundColor, Colors.black);
          expect(viewer.appBar, isNull);

          // Close button is present
          expect(
            find.byKey(const ValueKey('message-media-video-viewer-close')),
            findsOneWidget,
          );

          // Tapping close dismisses the viewer
          await tester.tap(
            find.byKey(const ValueKey('message-media-video-viewer-close')),
          );
          await tester.pumpAndSettle();

          expect(
            find.byKey(const ValueKey('message-media-video-viewer')),
            findsNothing,
          );
        },
      );

      testWidgets('treats only mp4 fallback URLs as videos', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '''
![mp4](https://example.com/media/clip.mp4)
![mov](https://example.com/media/clip.mov)
''',
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mp4',
            ),
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mov',
            ),
          ),
          findsNothing,
        );
        expect(
          find.byKey(
            const ValueKey(
              'message-media-image-preview:https://example.com/media/clip.mov',
            ),
          ),
          findsOneWidget,
        );
      });
    });

    group('blockquotes', () {
      testWidgets('renders blockquote with left border', (tester) async {
        await tester.pumpWidget(
          _testable(const MessageContent(content: '> This is a quote')),
        );

        final allText = _allRichText(tester);
        expect(allText, contains('This is a quote'));
        // Should strip the > prefix.
        expect(allText, isNot(contains('> This')));
      });
    });

    group('@mentions', () {
      testWidgets('renders @mention with highlight', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Hey @Alice check this out',
              mentionNames: {'pk1': 'Alice'},
            ),
          ),
        );

        // The desktop-style mention chip renders the prefix and label
        // separately so they can be aligned independently.
        expect(find.text('@'), findsOneWidget);
        expect(find.text('Alice'), findsOneWidget);
      });

      testWidgets('highlights an entire multi-word display name', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Hey @Kenny Lopez can you review this?',
              mentionNames: {'pk1': 'Kenny Lopez'},
            ),
          ),
        );

        expect(find.text('@'), findsOneWidget);
        expect(find.text('Kenny Lopez'), findsOneWidget);
        expect(find.text('@Kenny'), findsNothing);
        expect(_allRichText(tester), isNot(contains('Lopez Lopez')));
      });

      testWidgets('renders unknown @mention as-is', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Hey @unknown check this',
              mentionNames: {},
            ),
          ),
        );

        expect(find.text('@'), findsOneWidget);
        expect(find.text('unknown'), findsOneWidget);
      });

      testWidgets('does not treat email addresses as mentions', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Email alice@example.com for access',
              mentionNames: {'pk1': 'Alice'},
            ),
          ),
        );

        expect(_allRichText(tester), contains('alice@example.com'));
        expect(find.text('@example.com'), findsNothing);
      });

      testWidgets('mention tap callback fires with pubkey', (tester) async {
        String? tappedPubkey;
        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: 'Hey @Alice check this out',
              mentionNames: const {'pk1': 'Alice'},
              onMentionTap: (pubkey) => tappedPubkey = pubkey,
            ),
          ),
        );

        await tester.tap(find.text('Alice'));
        expect(tappedPubkey, 'pk1');
      });

      testWidgets('multi-word mention tap callback fires with pubkey', (
        tester,
      ) async {
        String? tappedPubkey;
        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: 'Hey @Kenny Lopez can you review this?',
              mentionNames: const {'pk1': 'Kenny Lopez'},
              onMentionTap: (pubkey) => tappedPubkey = pubkey,
            ),
          ),
        );

        await tester.tap(find.text('Kenny Lopez'));
        expect(tappedPubkey, 'pk1');
      });

      testWidgets('unknown mention renders without tap', (tester) async {
        var tapped = false;
        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: 'Hey @unknown check this',
              mentionNames: const {},
              onMentionTap: (_) => tapped = true,
            ),
          ),
        );

        await tester.tap(find.text('unknown'), warnIfMissed: false);
        expect(tapped, isFalse);
      });
    });

    group('#channel links', () {
      testWidgets('renders #channel with highlight', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Check out #general',
              channelNames: {'general': 'ch-id-1'},
            ),
          ),
        );

        expect(find.text('#general'), findsOneWidget);
      });

      testWidgets('channel tap callback fires', (tester) async {
        String? tappedId;
        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: 'See #general',
              channelNames: const {'general': 'ch-id-1'},
              onChannelTap: (id) => tappedId = id,
            ),
          ),
        );

        await tester.tap(find.text('#general'));
        expect(tappedId, 'ch-id-1');
      });

      testWidgets('unknown channel renders without tap', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Check #unknown', channelNames: {}),
          ),
        );

        expect(find.text('#unknown'), findsOneWidget);
      });

      testWidgets('does not treat URL fragments as channel links', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'See https://example.com/docs#frag',
              channelNames: {'frag': 'ch-id-1'},
            ),
          ),
        );

        expect(_allRichText(tester), contains('https://example.com/docs#frag'));
        expect(find.text('#frag'), findsNothing);
      });
    });

    group('mixed content', () {
      testWidgets('renders bold with mentions', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '**Important** @Alice please review',
              mentionNames: {'pk1': 'Alice'},
            ),
          ),
        );

        expect(_hasBoldSpan(tester, 'Important'), isTrue);
        expect(find.text('@'), findsOneWidget);
        expect(find.text('Alice'), findsOneWidget);
      });

      testWidgets('preserves markdown around mentions', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '**@Alice** please review',
              mentionNames: {'pk1': 'Alice'},
            ),
          ),
        );

        expect(find.text('@'), findsOneWidget);
        expect(find.text('Alice'), findsOneWidget);
        expect(_allRichText(tester), isNot(contains('**')));
      });

      testWidgets('renders code block between paragraphs', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Try this:\n```\nflutter test\n```\nDid it work?',
            ),
          ),
        );

        expect(_findRich('flutter test'), findsWidgets);
        expect(_findRich('Try this:'), findsWidgets);
        expect(_findRich('Did it work?'), findsWidgets);
      });
    });
  });
}
