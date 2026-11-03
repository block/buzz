import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_skeleton_body.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/widgets/media_loading_placeholder.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  for (final viewport in [320.0, 430.0, 800.0]) {
    for (final dimensions in [
      null,
      'malformed',
      '0x2400',
      '1200x2400',
      '1920x1080',
      '8000x1000',
    ]) {
      for (final kind in [
        'image',
        'video',
        'pdf',
        'extensionless',
        'unknown',
      ]) {
        testWidgets('$kind $dimensions matches preview at $viewport', (
          tester,
        ) async {
          tester.view.devicePixelRatio = 1;
          tester.view.physicalSize = Size(viewport, 1000);
          addTearDown(tester.view.resetPhysicalSize);
          addTearDown(tester.view.resetDevicePixelRatio);
          final url =
              'https://example.com/$viewport-$dimensions-$kind${kind == 'extensionless'
                  ? ''
                  : kind == 'video'
                  ? '.mp4'
                  : kind == 'image'
                  ? '.png'
                  : kind == 'pdf'
                  ? '.pdf'
                  : '.bin'}';
          final tags = [
            [
              'imeta',
              'url $url',
              'm ${kind == 'video'
                  ? 'video/mp4'
                  : kind == 'image'
                  ? 'image/png'
                  : 'application/octet-stream'}',
              if (dimensions != null) 'dim $dimensions',
            ],
          ];
          final response = Completer<http.Response>();
          final frame = Completer<LoadedVideoPreviewFrame?>();
          await tester.pumpWidget(
            ProviderScope(
              overrides: [
                videoPreviewFrameLoaderProvider.overrideWithValue(
                  (_) => frame.future,
                ),
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
                  data: MediaQuery.of(
                    context,
                  ).copyWith(disableAnimations: true),
                  child: AppMarkdownTheme(child: child!),
                ),
                home: Scaffold(
                  body: SingleChildScrollView(
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 56),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          MessageSkeletonBody(
                            content: '![media]($url)',
                            tags: tags,
                          ),
                          MessageContent(
                            content: '![media]($url)',
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
            ValueKey(
              'message-skeleton-${kind == 'video' ? 'video' : 'image'}:$url',
            ),
          );
          final preview = find.byKey(
            ValueKey(
              'message-media-${kind == 'video' ? 'video' : 'image'}-preview:$url',
            ),
          );
          final expected = tester.getSize(preview);
          expect(
            tester.getSize(skeleton).width,
            closeTo(expected.width, 0.001),
          );
          expect(
            tester.getSize(skeleton).height,
            closeTo(expected.height, 0.001),
          );
          if (kind == 'video') {
            frame.complete(
              LoadedVideoPreviewFrame(
                child: const ColoredBox(color: Colors.black),
                aspectRatio: 0.5,
                dispose: () async {},
              ),
            );
          } else {
            response.complete(
              http.Response.bytes(
                kind == 'unknown'
                    ? utf8.encode('not an image')
                    : base64Decode(
                        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=',
                      ),
                200,
              ),
            );
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 100)),
            );
          }
          await tester.pumpAndSettle();
          expect(tester.getSize(preview), expected);
          expect(find.byType(MediaLoadingPlaceholder), findsNothing);
          expect(tester.takeException(), isNull);
        });
      }
    }
  }
}
