import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_gallery_frame.dart';
import 'package:buzz/features/channels/message_skeleton_body.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/media_loading_placeholder.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  for (final count in [2, 4]) {
    for (final width in [208.0, 320.0]) {
      for (final scale in [1.0, 2.0]) {
        testWidgets(
          '$count trailing images match carousel at $width, scale $scale',
          (tester) async {
            final urls = List.generate(
              count,
              (i) => 'https://example.com/$count-$width-$scale-$i.png',
            );
            final content = urls.map((url) => '![photo]($url)').join('\n');
            final tags = urls
                .map(
                  (url) => [
                    'imeta',
                    'url $url',
                    'm image/png',
                    'dim 1200x2400',
                  ],
                )
                .toList();
            final response = Completer<http.Response>();
            await tester.pumpWidget(
              ProviderScope(
                overrides: [
                  mediaGetAuthServiceProvider.overrideWithValue(
                    MediaGetAuthService(
                      baseUrl: 'https://example.com',
                      nsec: null,
                    ),
                  ),
                  mediaHttpClientProvider.overrideWithValue(
                    MockClient((_) => response.future),
                  ),
                ],
                child: MaterialApp(
                  theme: AppTheme.light(),
                  builder: (context, child) => MediaQuery(
                    data: MediaQuery.of(context).copyWith(
                      disableAnimations: true,
                      textScaler: TextScaler.linear(scale),
                    ),
                    child: AppMarkdownTheme(child: child!),
                  ),
                  home: Scaffold(
                    body: SingleChildScrollView(
                      child: SizedBox(
                        width: width,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            MessageSkeletonBody(content: content, tags: tags),
                            MessageContent(
                              content: content,
                              tags: tags,
                              channelNames: const {'general': 'general-id'},
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            );
            final skeleton = find.byKey(
              const ValueKey('message-skeleton-gallery'),
            );
            final frame = find.descendant(
              of: find.byType(MessageContent),
              matching: find.byType(MessageGalleryFrame),
            );
            expect(skeleton, findsOneWidget);
            expect(
              find.byKey(ValueKey('message-skeleton-image:${urls.first}')),
              findsNothing,
            );
            final expected = tester.getSize(frame);
            expect(tester.getSize(skeleton), expected);
            expect(tester.getSize(find.byType(MessageSkeletonBody)), expected);
            response.complete(
              http.Response.bytes(
                base64Decode(
                  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=',
                ),
                200,
              ),
            );
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 100)),
            );
            await tester.pumpAndSettle();
            expect(tester.getSize(frame), expected);
            expect(find.byType(MediaLoadingPlaceholder), findsNothing);
            expect(tester.takeException(), isNull);
          },
        );
      }
    }
  }
}
