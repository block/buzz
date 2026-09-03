import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:http/http.dart' as http;
import 'package:just_audio/just_audio.dart' as audio;
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/features/channels/voice_note_attachment.dart';
import 'package:buzz/features/channels/voice_note_waveform.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/emoji/emoji_only.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';

Widget _testable(
  Widget child, {
  List<Override> overrides = const [],
  bool disableAnimations = false,
  VideoPreviewFrameLoader? videoPreviewFrameLoader,
}) {
  return ProviderScope(
    overrides: [
      videoPreviewFrameLoaderProvider.overrideWithValue(
        videoPreviewFrameLoader ?? (_) async => null,
      ),
      ...overrides,
    ],
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

class _FakeVoiceNotePlayer extends VoiceNotePlayerController {
  VoiceNotePlaybackState _state = const VoiceNotePlaybackState();

  @override
  VoiceNotePlaybackState get state => _state;

  double speed = 1;

  @override
  Future<void> loadLocal(
    String path, {
    required Duration fallbackDuration,
  }) async {
    _state = VoiceNotePlaybackState(duration: fallbackDuration);
    notifyListeners();
  }

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> Function() headers,
    required Duration fallbackDuration,
  }) => loadLocal(url, fallbackDuration: fallbackDuration);

  @override
  Future<void> pause() async {
    _state = _state.copyWith(isPlaying: false);
    notifyListeners();
  }

  @override
  Future<void> seek(Duration position) async {
    _state = _state.copyWith(position: position);
    notifyListeners();
  }

  @override
  Future<void> setSpeed(double value) async => speed = value;

  @override
  Future<void> toggle() async {
    _state = _state.copyWith(isPlaying: !_state.isPlaying);
    notifyListeners();
  }
}

class _LoadingVoiceNotePlayer extends _FakeVoiceNotePlayer {
  @override
  VoiceNotePlaybackState get state =>
      const VoiceNotePlaybackState(isLoading: true, canCancelLoading: true);

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> Function() headers,
    required Duration fallbackDuration,
  }) async {}
}

class _ToggleTrackingLoadingVoiceNotePlayer extends _LoadingVoiceNotePlayer {
  int toggleCount = 0;

  @override
  Future<void> toggle() async {
    toggleCount += 1;
  }
}

class _BufferingVoiceNotePlayer extends _FakeVoiceNotePlayer {
  int toggleCount = 0;

  @override
  VoiceNotePlaybackState get state => const VoiceNotePlaybackState(
    duration: Duration(seconds: 3),
    isPlaying: true,
    isLoading: true,
    canCancelLoading: true,
  );

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> Function() headers,
    required Duration fallbackDuration,
  }) async {}

  @override
  Future<void> toggle() async {
    toggleCount += 1;
  }
}

class _HeldAudioPlayerBackend implements VoiceNoteAudioPlayerBackend {
  final positions = const Stream<Duration>.empty();
  final durations = const Stream<Duration?>.empty();
  final states = const Stream<audio.PlayerState>.empty();
  final pathLoad = Completer<Duration?>();

  @override
  Stream<Duration> get positionStream => positions;

  @override
  Stream<Duration?> get durationStream => durations;

  @override
  Stream<audio.PlayerState> get playerStateStream => states;

  @override
  bool get playing => false;

  @override
  Future<Duration?> setFilePath(String path) => pathLoad.future;

  @override
  Future<Duration?> setUrl(String url, {Map<String, String>? headers}) async =>
      null;

  @override
  Future<void> play() async {}

  @override
  Future<void> pause() async {}

  @override
  Future<void> cancelPendingLoad() async {}

  @override
  Future<void> seek(Duration position) async {}

  @override
  Future<void> setSpeed(double speed) async {}

  @override
  Future<void> dispose() async {}
}

class _NoopHttpClient extends http.BaseClient {
  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    throw UnsupportedError('Local playback must not issue HTTP requests');
  }
}

class _RetryableVoiceNotePlayer extends _FakeVoiceNotePlayer {
  _RetryableVoiceNotePlayer() {
    _state = const VoiceNotePlaybackState(hasError: true);
  }

  int toggleCount = 0;

  @override
  Future<void> loadRemote(
    String url, {
    required Map<String, String> Function() headers,
    required Duration fallbackDuration,
  }) async {}

  @override
  Future<void> toggle() async {
    toggleCount += 1;
    _state = _state.copyWith(hasError: false, isPlaying: true);
    notifyListeners();
  }
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
  bool visit(InlineSpan span, TextStyle? inheritedStyle) {
    if (span is! TextSpan) return false;
    final effectiveStyle = inheritedStyle?.merge(span.style) ?? span.style;
    if (span.text != null &&
        span.text!.contains(text) &&
        effectiveStyle != null &&
        check(effectiveStyle)) {
      return true;
    }
    for (final child in span.children ?? const <InlineSpan>[]) {
      if (visit(child, effectiveStyle)) return true;
    }
    return false;
  }

  return visit(root, null);
}

class _TestChannelsNotifier extends ChannelsNotifier {
  _TestChannelsNotifier(this.channels);

  final Future<List<Channel>> channels;

  @override
  Future<List<Channel>> build() => channels;
}

void main() {
  test('wide voice-note waveforms distribute bars across their full width', () {
    const width = 320.0;
    const sampleCount = 48;
    final layout = voiceNoteWaveformBarLayout(
      width: width,
      sampleCount: sampleCount,
    );

    expect(layout.barWidth, 3);
    expect(
      (layout.barWidth * sampleCount) + (layout.gap * (sampleCount - 1)),
      closeTo(width, 0.001),
    );
  });

  testWidgets('voice-note waveform semantics seek within bounded steps', (
    tester,
  ) async {
    final progress = ValueNotifier(0.0);
    addTearDown(progress.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: ValueListenableBuilder<double>(
          valueListenable: progress,
          builder: (context, value, _) => VoiceNoteWaveform(
            samples: const [0.2, 0.8],
            progress: value,
            onSeek: (next) => progress.value = next,
          ),
        ),
      ),
    );

    final semantics = tester.getSemantics(
      find.bySemanticsLabel('Voice note waveform'),
    );
    expect(semantics.value, '0 percent');
    semantics.owner!.performAction(semantics.id, SemanticsAction.increase);
    await tester.pump();
    expect(
      tester.getSemantics(find.bySemanticsLabel('Voice note waveform')).value,
      '10 percent',
    );

    progress.value = 0.5;
    await tester.pump();
    final middle = tester.getSemantics(
      find.bySemanticsLabel('Voice note waveform'),
    );
    middle.owner!.performAction(middle.id, SemanticsAction.decrease);
    await tester.pump();
    expect(
      tester.getSemantics(find.bySemanticsLabel('Voice note waveform')).value,
      '40 percent',
    );

    progress.value = 1;
    await tester.pump();
    final end = tester.getSemantics(
      find.bySemanticsLabel('Voice note waveform'),
    );
    end.owner!.performAction(end.id, SemanticsAction.increase);
    await tester.pump();
    expect(
      tester.getSemantics(find.bySemanticsLabel('Voice note waveform')).value,
      '100 percent',
    );
  });

  group('MessageContent', () {
    testWidgets('forwards text alignment to markdown rendering', (
      tester,
    ) async {
      await tester.pumpWidget(
        _testable(
          const MessageContent(
            content: 'Centered status',
            textAlign: TextAlign.center,
          ),
        ),
      );

      expect(
        tester.widget<GptMarkdown>(find.byType(GptMarkdown)).textAlign,
        TextAlign.center,
      );
    });

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

      testWidgets('renders and routes a buzz message link', (tester) async {
        const url =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb&thread=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '[Open message]($url)')),
        );

        expect(find.text('Open message'), findsOneWidget);
        await tester.tap(find.text('Open message'));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
            messageId:
                'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
            threadRootId:
                'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
          ),
        );
      });

      testWidgets('renders and routes bare Buzz message links', (tester) async {
        const url =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

        await tester.pumpWidget(
          _testable(const MessageContent(content: 'See $url now')),
        );

        expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsOneWidget);
        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$url')));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
            messageId:
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          ),
        );
      });

      testWidgets('keeps Markdown delimiters outside bare Buzz links', (
        tester,
      ) async {
        const url =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '**$url**. and _${url}_')),
        );

        expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsNWidgets(2));

        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$url')).first);
        await tester.pump();
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
            messageId:
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          ),
        );
      });

      testWidgets('keeps non-adjacent Markdown delimiters outside links', (
        tester,
      ) async {
        const url =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content:
                  '*join $url* and **open $url** and '
                  '~~visit $url~~ and **_${url}_**.',
            ),
          ),
        );

        expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsNWidgets(4));
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        for (final link
            in find.byKey(ValueKey('buzz-link-chip:$url')).evaluate()) {
          await tester.tap(find.byWidget(link.widget));
          await tester.pump();
          expect(
            container.read(pendingDeepLinkProvider),
            const MessageDeepLink(
              channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
              messageId:
                  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            ),
          );
          container.read(pendingDeepLinkProvider.notifier).state = null;
        }
      });

      testWidgets('excludes sentence punctuation from bare Buzz links', (
        tester,
      ) async {
        const messageUrl =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
        const joinUrl =
            'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=invite-1';

        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'See $messageUrl. Then $joinUrl!'),
          ),
        );

        expect(
          find.byKey(ValueKey('buzz-link-chip:$messageUrl')),
          findsOneWidget,
        );
        expect(find.text(joinUrl), findsOneWidget);
        expect(_allRichText(tester), contains('See \u{FFFC}. Then \u{FFFC}!'));

        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$messageUrl')));
        await tester.pump();
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
            messageId:
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          ),
        );

        container.read(pendingDeepLinkProvider.notifier).consume();
        await tester.tap(find.text(joinUrl));
        await tester.pump();
        expect(
          container.read(pendingDeepLinkProvider),
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'invite-1',
          ),
        );
      });

      testWidgets('renders and routes autolinked Buzz thread links', (
        tester,
      ) async {
        const url =
            'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc&thread=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '<$url>')),
        );

        expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsOneWidget);
        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$url')));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const MessageDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
            messageId:
                'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
            threadRootId:
                'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
          ),
        );
      });

      testWidgets('renders and routes bare Buzz join links', (tester) async {
        const url =
            'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=invite-1';

        await tester.pumpWidget(
          _testable(const MessageContent(content: 'Join with $url')),
        );

        expect(find.text(url), findsOneWidget);
        await tester.tap(find.text(url));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'invite-1',
          ),
        );
      });

      testWidgets('renders and routes bare Buzz channel links', (tester) async {
        const url = 'buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32';

        await tester.pumpWidget(
          _testable(const MessageContent(content: 'See $url now')),
        );

        expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsOneWidget);
        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$url')));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const ChannelDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          ),
        );
      });

      testWidgets('renders and routes labeled Buzz channel links', (
        tester,
      ) async {
        const url = 'buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '[Open channel]($url)')),
        );

        await tester.tap(find.text('Open channel'));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const ChannelDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          ),
        );
      });

      testWidgets('routes rendered Buzz channel links through callback', (
        tester,
      ) async {
        const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
        const url = 'buzz://channel/$channelId';
        String? tappedChannelId;

        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: '[Open channel]($url)',
              onChannelTap: (id) => tappedChannelId = id,
            ),
          ),
        );

        await tester.tap(find.text('Open channel'));
        await tester.pump();

        expect(tappedChannelId, channelId);
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(container.read(pendingDeepLinkProvider), isNull);
      });

      testWidgets('renders and routes autolinked Buzz channel links', (
        tester,
      ) async {
        const url = 'buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '<$url>')),
        );

        await tester.tap(find.byKey(ValueKey('buzz-link-chip:$url')));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const ChannelDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          ),
        );
      });

      testWidgets('leaves malformed Buzz channel forms as plain text', (
        tester,
      ) async {
        const url =
            'buzz://channel?channel=580ca78b-9dae-46f3-8854-bd671853ba32';

        await tester.pumpWidget(
          _testable(const MessageContent(content: 'See $url now')),
        );

        expect(find.text(url), findsNothing);
        expect(_allRichText(tester), contains(url));
        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(container.read(pendingDeepLinkProvider), isNull);
      });

      testWidgets('renders bare URL as link', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Visit https://example.com today'),
          ),
        );

        // The URL text should be rendered and tappable.
        expect(find.text('https://example.com'), findsOneWidget);
        final linkText = tester.widget<Text>(find.text('https://example.com'));
        expect(
          linkText.style?.decoration ?? linkText.textSpan?.style?.decoration,
          TextDecoration.underline,
        );
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
      testWidgets('uses the shared Buzz loader while a voice note loads', (
        tester,
      ) async {
        const url = 'https://example.com/media/loading-voice-note.mp4';

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![audio]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-loading.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(
                _LoadingVoiceNotePlayer.new,
              ),
            ],
          ),
        );
        await tester.pump();

        expect(find.byType(BuzzLoadingIndicator), findsOneWidget);
        expect(find.byType(CircularProgressIndicator), findsNothing);
        expect(
          find.bySemanticsLabel('Cancel voice note loading'),
          findsOneWidget,
        );
        expect(find.bySemanticsLabel('Loading voice note'), findsNothing);
      });

      testWidgets('routes repeated loading-control taps through toggle', (
        tester,
      ) async {
        const url = 'https://example.com/media/cancel-loading-voice-note.mp4';
        final player = _ToggleTrackingLoadingVoiceNotePlayer();

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![audio]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-loading.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(() => player),
            ],
          ),
        );
        await tester.pump();

        final control = find.bySemanticsLabel('Cancel voice note loading');
        final controlSemantics = tester.getSemantics(control);
        expect(
          controlSemantics.getSemanticsData().hasAction(SemanticsAction.tap),
          isTrue,
        );
        tester.binding.performSemanticsAction(
          SemanticsActionEvent(
            type: SemanticsAction.tap,
            viewId: tester.view.viewId,
            nodeId: controlSemantics.id,
          ),
        );
        await tester.pump();

        expect(player.toggleCount, 1);
      });

      testWidgets('active buffering playback keeps its pause action', (
        tester,
      ) async {
        const url = 'https://example.com/media/buffering-voice-note.mp4';
        final player = _BufferingVoiceNotePlayer();

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![audio]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-buffering.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(() => player),
            ],
          ),
        );
        await tester.pump();

        final control = find.bySemanticsLabel('Pause voice note');
        expect(control, findsOneWidget);
        expect(
          tester
              .getSemantics(control)
              .getSemanticsData()
              .hasAction(SemanticsAction.tap),
          isTrue,
        );

        await tester.tap(find.byKey(const ValueKey('voice-note-play-pause')));
        await tester.pump();

        expect(player.toggleCount, 1);
      });

      testWidgets(
        'local preview loading is non-actionable while remote loading cancels',
        (tester) async {
          final backend = _HeldAudioPlayerBackend();
          final player = DeviceVoiceNotePlayerController(
            coordinator: VoiceNotePlaybackCoordinator(),
            client: _NoopHttpClient(),
            player: backend,
          );
          addTearDown(() {
            if (!backend.pathLoad.isCompleted) backend.pathLoad.complete(null);
          });

          await tester.pumpWidget(
            _testable(
              const VoiceNoteAttachment.local(
                path: '/tmp/local-voice-note.m4a',
                duration: Duration(seconds: 3),
                waveform: [],
              ),
              overrides: [
                voiceNotePlayerFactoryProvider.overrideWithValue(() => player),
              ],
            ),
          );
          await tester.pump();

          expect(backend.pathLoad.isCompleted, isFalse);

          final control = find.bySemanticsLabel('Loading voice note');
          final controlSemantics = tester.getSemantics(control);
          expect(control, findsOneWidget);
          expect(
            controlSemantics.getSemanticsData().hasAction(SemanticsAction.tap),
            isFalse,
          );
          expect(
            find.bySemanticsLabel('Cancel voice note loading'),
            findsNothing,
          );
          await tester.tap(find.byKey(const ValueKey('voice-note-play-pause')));
          await tester.pump();
          expect(player.state.canCancelLoading, isFalse);
          expect(backend.pathLoad.isCompleted, isFalse);
        },
      );

      testWidgets('offers an accessible retry after a voice note fails', (
        tester,
      ) async {
        const url = 'https://example.com/media/retry-voice-note.mp4';
        final player = _RetryableVoiceNotePlayer();

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![audio]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-retry.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(() => player),
            ],
          ),
        );
        await tester.pump();

        expect(find.text('Voice note unavailable'), findsOneWidget);
        expect(find.byTooltip('Retry voice note'), findsOneWidget);
        expect(find.bySemanticsLabel('Retry voice note'), findsOneWidget);
        expect(
          find.byKey(const ValueKey('voice-note-retry-icon')),
          findsOneWidget,
        );

        await tester.tap(find.byTooltip('Retry voice note'));
        await tester.pump();

        expect(player.toggleCount, 1);
        expect(find.byTooltip('Pause voice note'), findsOneWidget);
        expect(find.text('Voice note unavailable'), findsNothing);
      });

      testWidgets('renders desktop packaged voice-note links as audio cards', (
        tester,
      ) async {
        const url = 'https://example.com/media/desktop-voice-note.mp4';

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '[voice-note-desktop.mp4]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-desktop.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(
                _FakeVoiceNotePlayer.new,
              ),
            ],
          ),
        );
        await tester.pump();

        expect(
          find.byKey(const ValueKey('voice-note-attachment:$url')),
          findsOneWidget,
        );
        expect(find.text('voice-note-desktop.mp4'), findsNothing);
      });

      testWidgets('keeps audio-looking links without imeta as ordinary links', (
        tester,
      ) async {
        const url = 'https://example.com/media/not-an-attachment.mp4';

        await tester.pumpWidget(
          _testable(const MessageContent(content: '[recording.mp4]($url)')),
        );
        await tester.pump();

        expect(
          find.byKey(const ValueKey('voice-note-attachment:$url')),
          findsNothing,
        );
        expect(find.text('recording.mp4'), findsOneWidget);
      });

      testWidgets('renders an audio imeta attachment as a voice note card', (
        tester,
      ) async {
        const url = 'https://example.com/media/voice-note.mp4';
        final player = _FakeVoiceNotePlayer();
        final hapticCalls = <MethodCall>[];
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          (call) async {
            if (call.method == 'HapticFeedback.vibrate') hapticCalls.add(call);
            return null;
          },
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            null,
          ),
        );
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![audio]($url)',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm video/mp4',
                  'duration 3.0',
                  'filename voice-note-test.mp4',
                ],
              ],
            ),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(() => player),
            ],
          ),
        );
        await tester.pump();

        expect(
          find.byKey(const ValueKey('voice-note-attachment:$url')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('voice-note-play-pause')),
          findsOneWidget,
        );
        final cardFinder = find.byKey(
          const ValueKey('voice-note-attachment:$url'),
        );
        final rateFinder = find.byKey(
          const ValueKey('voice-note-playback-rate'),
        );
        final card = tester.widget<Container>(cardFinder);
        expect(card.padding, const EdgeInsets.all(Grid.twelve));
        expect(
          tester.getTopLeft(find.byType(VoiceNoteWaveform)).dx,
          tester
              .getTopLeft(find.byKey(const ValueKey('voice-note-duration')))
              .dx,
        );
        final leadingInset =
            tester
                .getTopLeft(find.byKey(const ValueKey('voice-note-play-pause')))
                .dx -
            tester.getTopLeft(cardFinder).dx;
        final trailingInset =
            tester.getTopRight(cardFinder).dx -
            tester.getTopRight(rateFinder).dx;
        expect(leadingInset, Grid.twelve + 1);
        expect(trailingInset, leadingInset);
        expect(rateFinder, findsOneWidget);
        final rateSize = tester.getSize(rateFinder);
        final ratePadding = tester
            .widgetList<Padding>(
              find.descendant(of: rateFinder, matching: find.byType(Padding)),
            )
            .singleWhere(
              (widget) =>
                  widget.padding ==
                  const EdgeInsets.symmetric(
                    horizontal: Grid.xxs,
                    vertical: Grid.half + Grid.quarter,
                  ),
            );
        expect(
          ratePadding.padding,
          const EdgeInsets.symmetric(
            horizontal: Grid.xxs,
            vertical: Grid.half + Grid.quarter,
          ),
        );
        final rateValueFinder = find.byKey(
          const ValueKey('voice-note-playback-rate-value'),
        );
        hapticCalls.clear();
        await tester.tap(find.byKey(const ValueKey('voice-note-play-pause')));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 500));
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(
          tester
              .widget<VoiceNoteWaveform>(find.byType(VoiceNoteWaveform))
              .progress,
          greaterThan(0),
        );

        final waveformRect = tester.getRect(find.byType(VoiceNoteWaveform));
        await tester.dragFrom(
          Offset(
            waveformRect.left + waveformRect.width * 0.25,
            waveformRect.center.dy,
          ),
          Offset(waveformRect.width * 0.5, 0),
        );
        await tester.pump();
        expect(player.state.position.inMilliseconds, closeTo(2250, 80));

        expect(tester.widget<Text>(rateValueFinder).data, '1×');
        hapticCalls.clear();
        await tester.tap(rateFinder);
        await tester.pump();
        expect(tester.widget<Text>(rateValueFinder).data, '1.5×');
        expect(player.speed, 1.5);
        expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');
        expect(tester.getSize(rateFinder), rateSize);
        await tester.tap(rateFinder);
        await tester.pump();
        expect(tester.widget<Text>(rateValueFinder).data, '2×');
        expect(tester.getSize(rateFinder), rateSize);
        await tester.tap(rateFinder);
        await tester.pump();
        expect(tester.widget<Text>(rateValueFinder).data, '.5×');
        expect(tester.getSize(rateFinder), rateSize);
      });

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
        'keeps voice notes out of image carousels for audio-only and mixed media',
        (tester) async {
          const firstAudio = 'https://example.com/media/voice-note-first.mp4';
          const secondAudio = 'https://example.com/media/voice-note-second.mp4';
          const image = 'https://example.com/media/photo.png';

          Widget message(String content, List<List<String>> tags) => _testable(
            MessageContent(content: content, tags: tags),
            overrides: [
              voiceNotePlayerFactoryProvider.overrideWithValue(
                _FakeVoiceNotePlayer.new,
              ),
            ],
          );

          await tester.pumpWidget(
            message('![audio]($firstAudio)\n![audio]($secondAudio)', const [
              [
                'imeta',
                'url $firstAudio',
                'm video/mp4',
                'filename voice-note-first.mp4',
              ],
              [
                'imeta',
                'url $secondAudio',
                'm video/mp4',
                'filename voice-note-second.mp4',
              ],
            ]),
          );
          await tester.pumpAndSettle();
          expect(
            find.byKey(const ValueKey('message-media-carousel')),
            findsNothing,
          );
          expect(find.byType(VoiceNoteAttachment), findsNWidgets(2));

          await tester.pumpWidget(
            message('![image]($image)\n![audio]($firstAudio)', const [
              ['imeta', 'url $image', 'm image/png'],
              [
                'imeta',
                'url $firstAudio',
                'm video/mp4',
                'filename voice-note-first.mp4',
              ],
            ]),
          );
          await tester.pumpAndSettle();
          expect(
            find.byKey(const ValueKey('message-media-carousel')),
            findsNothing,
          );
          expect(find.byType(VoiceNoteAttachment), findsOneWidget);
          expect(_imagePreview(image), findsOneWidget);
        },
      );

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

      testWidgets('derives a first frame for a posterless video event', (
        tester,
      ) async {
        const videoUrl = 'https://example.com/media/legacy-video.mp4';
        var disposed = false;

        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![video]($videoUrl)',
              tags: [
                ['imeta', 'url $videoUrl', 'm video/mp4', 'dim 1080x1920'],
              ],
            ),
            videoPreviewFrameLoader: (url) async {
              expect(url, videoUrl);
              return LoadedVideoPreviewFrame(
                aspectRatio: 9 / 16,
                child: const ColoredBox(
                  key: ValueKey('derived-video-frame'),
                  color: Colors.blue,
                ),
                dispose: () async => disposed = true,
              );
            },
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(
            const ValueKey('message-media-video-first-frame:$videoUrl'),
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('derived-video-frame')),
          findsOneWidget,
        );
        expect(find.text('Video attachment'), findsNothing);

        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump();
        expect(disposed, isTrue);
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
          expect(
            find.descendant(
              of: find.byKey(
                const ValueKey('message-media-video-viewer-reply-thread'),
              ),
              matching: find.byType(IconButton),
            ),
            findsNothing,
          );

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

      testWidgets('swiping down on the video dismisses its viewer', (
        tester,
      ) async {
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

        await tester.tap(
          find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mp4',
            ),
          ),
        );
        await tester.pumpAndSettle();

        await tester.drag(
          find.byKey(const ValueKey('message-media-video-viewer-gesture')),
          const Offset(0, 140),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(const ValueKey('message-media-video-viewer')),
          findsNothing,
        );
      });

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

      testWidgets('renders an explicitly tagged non-mp4 video preview', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '![video](https://example.com/media/clip.mov)',
              tags: [
                [
                  'imeta',
                  'url https://example.com/media/clip.mov',
                  'm video/quicktime',
                ],
              ],
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.byKey(
            const ValueKey(
              'message-media-video-preview:https://example.com/media/clip.mov',
            ),
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(
            const ValueKey(
              'message-media-image-preview:https://example.com/media/clip.mov',
            ),
          ),
          findsNothing,
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

    group('Buzz permalink chips', () {
      testWidgets('keeps authored Buzz labels as ordinary links', (
        tester,
      ) async {
        final owner = 'ab' * 32;
        final id = 'cd' * 32;
        const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
        final links = {
          'Open message': 'buzz://message?channel=$channelId&id=$id',
          'Open channel': 'buzz://channel/$channelId',
          'Release candidate': 'buzz://pr?id=$id&owner=$owner&d=buzz',
        };

        await tester.pumpWidget(
          _testable(
            MessageContent(
              content: links.entries
                  .map((entry) => '[${entry.key}](${entry.value})')
                  .join(' '),
            ),
          ),
        );
        await tester.pump();

        for (final entry in links.entries) {
          expect(
            find.byKey(ValueKey('buzz-link-chip:${entry.value}')),
            findsNothing,
          );
          expect(find.text(entry.key), findsOneWidget);
        }
      });

      testWidgets('preserves formatting in authored Buzz labels', (
        tester,
      ) async {
        const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: '[**design discussion**](buzz://channel/$channelId)',
            ),
          ),
        );
        await tester.pump();

        expect(_hasBoldSpan(tester, 'design discussion'), isTrue);
      });

      testWidgets(
        'renders message, channel, repo, PR, and issue links as chips',
        (tester) async {
          final owner = 'ab' * 32;
          final id = 'cd' * 32;
          const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
          final urls = [
            'buzz://message?channel=$channelId&id=$id',
            'buzz://channel/$channelId',
            'buzz://repo?owner=$owner&d=buzz',
            'buzz://pr?id=$id&owner=$owner&d=buzz',
            'buzz://issue?id=$id&owner=$owner&d=buzz',
          ];
          await tester.pumpWidget(
            _testable(
              MessageContent(
                content: urls.join(' '),
                channelNames: const {'engineering': channelId},
              ),
            ),
          );
          await tester.pump();

          for (final url in urls) {
            expect(find.byKey(ValueKey('buzz-link-chip:$url')), findsOneWidget);
          }
          expect(find.text('engineering · cdcdcdcd'), findsOneWidget);
          expect(find.text('engineering'), findsOneWidget);
          expect(find.text('buzz'), findsOneWidget);
          expect(find.text('buzz · cdcdcdcd'), findsNWidgets(2));
          expect(find.byIcon(LucideIcons.messageSquare), findsOneWidget);
          expect(find.byIcon(LucideIcons.hash), findsOneWidget);
          expect(find.byIcon(LucideIcons.folderGit2), findsOneWidget);
          expect(find.byIcon(LucideIcons.gitPullRequest), findsOneWidget);
          expect(find.byIcon(LucideIcons.circleDot), findsOneWidget);
          expect(
            find.bySemanticsLabel(
              'Open message cdcdcdcd in channel engineering',
            ),
            findsOneWidget,
          );
          expect(
            find.bySemanticsLabel('Pull request cdcdcdcd in repository buzz'),
            findsOneWidget,
          );
          for (final url in urls.skip(2)) {
            final chipKey = ValueKey('buzz-link-chip:$url');
            final ignoredChip = find.ancestor(
              of: find.byKey(chipKey),
              matching: find.byWidgetPredicate(
                (widget) => widget is IgnorePointer && widget.ignoring,
              ),
            );
            expect(ignoredChip, findsOneWidget, reason: url);
            expect(
              tester.widget<IgnorePointer>(ignoredChip).ignoring,
              isTrue,
              reason: url,
            );
          }
        },
      );

      testWidgets('uses shortened channel identifiers when names are missing', (
        tester,
      ) async {
        final id = 'cd' * 32;
        const channelId = '580ca78b-9dae-46f3-8854-bd671853ba32';
        final messageUrl = 'buzz://message?channel=$channelId&id=$id';
        const channelUrl = 'buzz://channel/$channelId';

        await tester.pumpWidget(
          _testable(MessageContent(content: '$messageUrl $channelUrl')),
        );
        await tester.pump();

        expect(find.text('580ca78b · cdcdcdcd'), findsOneWidget);
        expect(find.text('580ca78b'), findsOneWidget);
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

      testWidgets('renders a known agent mention with the bot chip', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Ask @Helper Bot to investigate',
              mentionNames: {'agent-pubkey': 'Helper Bot'},
              agentMentionPubkeys: {'agent-pubkey'},
              maxLines: 2,
            ),
          ),
        );

        expect(find.byIcon(LucideIcons.bot), findsOneWidget);
        expect(find.text('@'), findsNothing);
        expect(find.text('Helper Bot'), findsOneWidget);
      });

      testWidgets('normalizes passed multi-word agent mentions', (
        tester,
      ) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(
              content: 'Ask @Helper Bot to investigate',
              mentionNames: {'agent-pubkey': 'Helper Bot'},
              agentMentionPubkeys: {'agent-pubkey'},
            ),
          ),
        );

        expect(find.byIcon(LucideIcons.bot), findsOneWidget);
        expect(find.text('Helper Bot'), findsOneWidget);
        expect(_allRichText(tester), isNot(contains('Bot Bot')));
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

        expect(find.byIcon(LucideIcons.hash), findsOneWidget);
        expect(find.text('general'), findsOneWidget);
        expect(find.text('#general'), findsNothing);
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

        await tester.tap(find.text('general'));
        expect(tappedId, 'ch-id-1');
      });

      testWidgets('resolved #channel defaults to in-app navigation', (
        tester,
      ) async {
        final channels = Future.value([
          Channel(
            id: '580ca78b-9dae-46f3-8854-bd671853ba32',
            name: 'general',
            channelType: 'stream',
            visibility: 'open',
            description: '',
            createdBy: 'creator',
            createdAt: DateTime(2026),
            memberCount: 1,
            isMember: true,
          ),
        ]);
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'See #general'),
            overrides: [
              channelsProvider.overrideWith(
                () => _TestChannelsNotifier(channels),
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();

        await tester.tap(find.text('general'));
        await tester.pump();

        final container = ProviderScope.containerOf(
          tester.element(find.byType(MessageContent)),
        );
        expect(
          container.read(pendingDeepLinkProvider),
          const ChannelDeepLink(
            channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          ),
        );
      });

      testWidgets('unknown channel renders without tap', (tester) async {
        await tester.pumpWidget(
          _testable(
            const MessageContent(content: 'Check #unknown', channelNames: {}),
          ),
        );

        expect(find.byIcon(LucideIcons.hash), findsOneWidget);
        expect(find.text('unknown'), findsOneWidget);
        expect(find.text('#unknown'), findsNothing);
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
