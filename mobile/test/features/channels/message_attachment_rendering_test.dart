import 'dart:convert';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/media_loading_placeholder.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/media_codec_fixtures.dart';

const _base = 'https://relay.example';

Widget _app(
  String content, {
  List<List<String>> tags = const [],
  List<Override> overrides = const [],
}) => ProviderScope(
  overrides: overrides,
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: MessageContent(
        content: content,
        tags: tags,
        channelNames: const {'general': 'channel-id'},
      ),
    ),
  ),
);

// Image codecs complete outside the fake test clock. Wait for actual pixels,
// rather than waiting for an indefinitely animated loading shimmer to settle.
Future<void> _waitForImages(WidgetTester tester) async {
  for (var attempt = 0; attempt < 200; attempt++) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 10)),
    );
    await tester.pump(const Duration(milliseconds: 16));
    final images = tester.widgetList<RawImage>(find.byType(RawImage));
    if (images.any((image) => image.image != null) &&
        find.byType(MediaLoadingPlaceholder).evaluate().isEmpty) {
      return;
    }
  }
  fail('Media did not produce a decoded image frame');
}

void main() {
  setUp(() {
    MediaImageProvider.debugResetCooldowns();
    PaintingBinding.instance.imageCache.clear();
    PaintingBinding.instance.imageCache.clearLiveImages();
  });

  for (final fixture in mediaCodecFixtures.entries) {
    for (final angled in [false, true]) {
      testWidgets(
        '${fixture.key} ${angled ? 'angle' : 'plain'} image downloads and opens',
        (tester) async {
          final url = '$_base/media/photo.${fixture.key}?download=1&v=2';
          final requests = <http.Request>[];
          final client = MockClient((request) async {
            requests.add(request);
            return http.Response.bytes(base64Decode(fixture.value), 200);
          });
          final auth = MediaGetAuthService(
            baseUrl: _base,
            nsec: nostr.Keys.generate().nsec,
          );
          await tester.pumpWidget(
            _app(
              '${angled ? 'oops [ text ' : 'Look\n'}'
              '![photo \\[Q4\\]](${angled ? '<$url>' : url})',
              tags: [
                [
                  'imeta',
                  'url $url',
                  'm image/${fixture.key}',
                  'dim 2x2',
                  'alt Photo',
                ],
              ],
              overrides: [
                mediaHttpClientProvider.overrideWithValue(client),
                mediaGetAuthServiceProvider.overrideWithValue(auth),
              ],
            ),
          );
          await _waitForImages(tester);
          await tester.pumpAndSettle();
          expect(requests, hasLength(1));
          expect(requests.single.url.toString(), url);
          expect(
            requests.single.headers['Authorization'],
            startsWith('Nostr '),
          );
          expect(find.text('Image unavailable'), findsNothing);
          expect(tester.takeException(), isNull);
          await tester.tap(
            find.byKey(ValueKey('message-media-image-preview:$url')),
          );
          await _waitForImages(tester);
          await tester.pumpAndSettle();
          expect(
            tester
                .widget<MediaImageViewerPage>(find.byType(MediaImageViewerPage))
                .imageUrl,
            url,
          );
          expect(find.text('Failed to load image'), findsNothing);
        },
      );
    }
  }

  for (final title in ['(Video title)', r'"A \"preview\""']) {
    for (final item in [
      ('mp4', 'video/mp4'),
      ('mov', 'video/quicktime'),
      ('webm', 'video/webm'),
      ('bin', 'video/mp4'),
    ]) {
      testWidgets('${item.$1} uses video metadata with title $title', (
        tester,
      ) async {
        final url = '$_base/media/clip (1).${item.$1}';
        final expected = '$_base/media/clip%20%281%29.${item.$1}';
        String? requested;
        await tester.pumpWidget(
          _app(
            'oops [ text ![clip \\[Q4\\]](<$url> $title)',
            tags: [
              ['imeta', 'url $url', 'm ${item.$2}'],
            ],
            overrides: [
              videoPreviewFrameLoaderProvider.overrideWithValue((url) async {
                requested = url;
                return null;
              }),
            ],
          ),
        );
        await tester.pumpAndSettle();
        expect(requested, expected);
        expect(
          find.byKey(ValueKey('message-media-video-preview:$expected')),
          findsOneWidget,
        );
        expect(find.text('Image unavailable'), findsNothing);
      });
    }

    for (final (stem, angled) in [
      ('report [Q4]', false),
      ('report [Q4]', true),
      ('report [Q4', true),
      ('report Q4]', true),
      (r'report \ [Q4]', true),
    ]) {
      for (final extension in ['pdf', 'txt', 'zip', 'mp3', 'svg']) {
        testWidgets(
          '$stem.$extension angled=$angled title $title opens with auth and filename',
          (tester) async {
            final url = '$_base/media/report.$extension';
            final blob = BlobDescriptor(
              url: url,
              sha256: 'a' * 64,
              size: 1,
              type: 'application/octet-stream',
              uploaded: 0,
              filename: '$stem.$extension',
            );
            final markdown = blob.toMarkdownImage().replaceFirst(
              '($url)',
              angled ? '(<$url> $title)' : '($url)',
            );
            String? opened;
            String? filename;
            Map<String, String>? headers;
            await tester.pumpWidget(
              _app(
                '${angled ? 'oops [ then ' : ''}$markdown',
                overrides: [
                  mediaGetAuthServiceProvider.overrideWithValue(
                    MediaGetAuthService(
                      baseUrl: _base,
                      nsec: nostr.Keys.generate().nsec,
                    ),
                  ),
                  openDownloadedFileProvider.overrideWithValue((
                    url,
                    auth,
                    name,
                  ) async {
                    opened = url;
                    headers = auth;
                    filename = name;
                  }),
                ],
              ),
            );
            await tester.tap(find.text('$stem.$extension'));
            await tester.pump();
            expect(opened, url);
            expect(filename, '$stem.$extension');
            expect(headers?['Authorization'], startsWith('Nostr '));
            expect(find.byType(MediaImage), findsNothing);
          },
        );
      }
    }
  }

  testWidgets(
    'mixed angle/plain gallery retains URLs, metadata and viewer items',
    (tester) async {
      const first = '$_base/media/photo(1).png';
      const second = '$_base/media/photo2.png';
      const normalizedFirst = '$_base/media/photo%281%29.png';
      final client = MockClient(
        (_) async =>
            http.Response.bytes(base64Decode(mediaCodecFixtures['png']!), 200),
      );
      await tester.pumpWidget(
        _app(
          'oops [ text ![inline](<$second>)\n![first](<$first>)\n![second \\[Q4\\]]($second)',
          tags: [
            [
              'imeta',
              'url $first',
              'm image/png',
              'dim 2x2',
              'alt First photo',
            ],
            ['imeta', 'url $second', 'm image/png', 'dim 2x2'],
          ],
          overrides: [mediaHttpClientProvider.overrideWithValue(client)],
        ),
      );
      await _waitForImages(tester);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('message-media-image-preview:$second')),
        findsOneWidget,
      );
      expect(find.text('2 images'), findsOneWidget);
      expect(find.bySemanticsLabel('Open second [Q4]'), findsOneWidget);
      expect(find.bySemanticsLabel('Open First photo'), findsOneWidget);
      expect(find.text('Image unavailable'), findsNothing);
      await tester.tap(
        find.byKey(
          const ValueKey('message-media-carousel-item:$normalizedFirst'),
        ),
      );
      await _waitForImages(tester);
      await tester.pumpAndSettle();
      final viewer = tester.widget<MediaImageViewerPage>(
        find.byType(MediaImageViewerPage),
      );
      expect(viewer.galleryItems!.map((item) => item.url), [
        normalizedFirst,
        second,
      ]);
    },
  );
}
