import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/theme/theme.dart';

Widget _testable(Widget child, {List<Override> overrides = const []}) {
  return ProviderScope(
    overrides: overrides,
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Scaffold(body: child),
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
    test('buildImageViewerRoute uses modal-style page route builder', () {
      final route = buildImageViewerRoute(
        imageUrl: 'https://example.com/media/image.png',
        heroTag: Object(),
      );

      expect(route, isA<PageRouteBuilder<void>>());
      expect(route.transitionDuration, const Duration(milliseconds: 280));
      expect(
        route.reverseTransitionDuration,
        const Duration(milliseconds: 220),
      );
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

        // Mention should be rendered as @Alice in a highlighted container.
        expect(find.text('@Alice'), findsOneWidget);
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

        expect(find.text('@Kenny Lopez'), findsOneWidget);
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

        expect(find.text('@unknown'), findsOneWidget);
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
        expect(find.text('@Alice'), findsOneWidget);
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

        expect(find.text('@Alice'), findsOneWidget);
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
