import 'dart:async';
import 'dart:convert';
import 'dart:isolate';

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';

class _SaturatedAuth extends MediaGetAuthService {
  _SaturatedAuth({this.alwaysReject = false, this.workerFailure = false})
    : super(baseUrl: 'https://example.com', nsec: null);
  final bool alwaysReject;
  final bool workerFailure;
  final drained = Completer<void>();
  int calls = 0;
  @override
  Future<Map<String, String>> headersForAsync(String url) async {
    calls++;
    if (calls == 1 || alwaysReject) {
      if (workerFailure) {
        await drained.future;
        throw IsolateSpawnException('worker unavailable');
      }
      throw MediaAuthQueueFull(drained.future);
    }
    return {'Authorization': 'test-proof'};
  }
}

class _PermanentFailureAuth extends MediaGetAuthService {
  _PermanentFailureAuth(this.error)
    : super(baseUrl: 'https://example.com', nsec: null);
  final Object error;
  int calls = 0;
  @override
  Future<Map<String, String>> headersForAsync(String url) async {
    calls++;
    throw error;
  }
}

class _VideoPlatform extends VideoPlayerPlatform {
  final sources = <DataSource>[];
  @override
  Future<void> init() async {}
  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async {
    sources.add(options.dataSource);
    return 1;
  }

  @override
  Stream<VideoEvent> videoEventsFor(int playerId) => Stream.value(
    VideoEvent(
      eventType: VideoEventType.initialized,
      size: const Size(100, 100),
      duration: const Duration(seconds: 1),
    ),
  );
  @override
  Widget buildView(int playerId) => const SizedBox();
  @override
  Future<void> dispose(int playerId) async {}
  @override
  Future<void> pause(int playerId) async {}
  @override
  Future<void> setLooping(int playerId, bool looping) async {}
  @override
  Future<void> setVolume(int playerId, double volume) async {}
  @override
  Future<void> seekTo(int playerId, Duration position) async {}
  @override
  Future<void> setMixWithOthers(bool mixWithOthers) async {}
}

void main() {
  test('permanent and ambiguous runtime failures are never retried', () async {
    for (final error in [
      ArgumentError('invalid signing key'),
      StateError('signing failed'),
      RemoteError('worker runtime error', ''),
    ]) {
      final auth = _PermanentFailureAuth(error);
      await expectLater(
        auth.headersForAsyncWithRetry('https://example.com/media/a'),
        throwsA(same(error)),
      );
      expect(auth.calls, 1);
    }
  });

  test('bounds retry waiters before signing capacity drains', () async {
    final auth = _SaturatedAuth(alwaysReject: true);
    var rejected = 0;
    final requests = List.generate(
      80,
      (_) => auth
          .headersForAsyncWithRetry('https://example.com/media/a')
          .then<Object>(
            (headers) => headers,
            onError: (Object error) {
              rejected++;
              return error;
            },
          ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(auth.calls, 80);
    expect(rejected, 16);
    auth.drained.complete();
    await Future.wait(requests);
    expect(auth.calls, 144);
    expect(rejected, 80);
    // Retry permits are released even when the second admission fails.
    await expectLater(
      auth.headersForAsyncWithRetry('https://example.com/media/a'),
      throwsA(isA<MediaAuthQueueFull>()),
    );
    expect(auth.calls, 146);
  });

  test('retries saturation only once', () async {
    final auth = _SaturatedAuth(alwaysReject: true);
    final result = auth.headersForAsyncWithRetry('https://example.com/media/a');
    final failure = expectLater(result, throwsA(isA<MediaAuthQueueFull>()));
    await Future<void>.delayed(Duration.zero);
    expect(auth.calls, 1);
    auth.drained.complete();
    await failure;
    expect(auth.calls, 2);
  });

  test('startup failures share the bounded retry permits', () async {
    final auth = _SaturatedAuth(alwaysReject: true, workerFailure: true);
    final requests = List.generate(
      80,
      (_) => auth
          .headersForAsyncWithRetry('https://example.com/media/a')
          .then<Object>((value) => value, onError: (Object error) => error),
    );
    auth.drained.complete();
    final results = await Future.wait(requests);
    expect(results.whereType<MediaAuthQueueFull>(), hasLength(16));
    expect(results.whereType<IsolateSpawnException>(), hasLength(64));
    expect(auth.calls, 144);
    await expectLater(
      auth.headersForAsyncWithRetry('https://example.com/media/a'),
      throwsA(isA<IsolateSpawnException>()),
    );
    expect(auth.calls, 146);
  });

  test('startup recovery is bounded to one retry', () async {
    final auth = _SaturatedAuth(alwaysReject: true, workerFailure: true);
    auth.drained.complete();
    await expectLater(
      auth.headersForAsyncWithRetry('https://example.com/media/a'),
      throwsA(isA<IsolateSpawnException>()),
    );
    expect(auth.calls, 2);
  });

  for (final workerFailure in [false, true]) {
    testWidgets(
      'mounted image workerFailure=$workerFailure succeeds after signing capacity drains',
      (tester) async {
        final auth = _SaturatedAuth(workerFailure: workerFailure);
        final png = base64Decode(
          'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
        );
        var fetches = 0;
        final client = MockClient((request) async {
          fetches++;
          expect(request.headers['Authorization'], 'test-proof');
          return http.Response.bytes(png, 200);
        });
        await tester.pumpWidget(
          MaterialApp(
            home: Image(
              image: MediaImageProvider(
                url: 'https://example.com/media/recovery.png',
                auth: auth,
                client: client,
              ),
              errorBuilder: (_, _, _) => const Text('failed'),
            ),
          ),
        );
        await tester.pump();
        final element = tester.element(find.byType(Image));
        expect(auth.calls, 1);
        expect(fetches, 0);
        auth.drained.complete();
        await tester.runAsync(() async {
          await Future<void>.delayed(const Duration(milliseconds: 100));
        });
        await tester.pumpAndSettle();
        expect(tester.element(find.byType(Image)), same(element));
        expect(auth.calls, 2);
        expect(fetches, 1);
        expect(tester.widget<RawImage>(find.byType(RawImage)).image, isNotNull);
        expect(find.text('failed'), findsNothing);
        await tester.pumpWidget(const SizedBox());
        PaintingBinding.instance.imageCache.clear();
        PaintingBinding.instance.imageCache.clearLiveImages();
        client.close();
      },
    );

    testWidgets(
      'mounted posterless video workerFailure=$workerFailure succeeds after signing capacity drains',
      (tester) async {
        const url = 'https://example.com/media/recovery.mp4';
        final auth = _SaturatedAuth(workerFailure: workerFailure);
        final previous = VideoPlayerPlatform.instance;
        final platform = _VideoPlatform();
        VideoPlayerPlatform.instance = platform;
        addTearDown(() => VideoPlayerPlatform.instance = previous);
        await tester.pumpWidget(
          ProviderScope(
            overrides: [mediaGetAuthServiceProvider.overrideWithValue(auth)],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: const AppMarkdownTheme(
                child: Scaffold(
                  body: MessageContent(
                    content: '![video]($url)',
                    tags: [
                      ['imeta', 'url $url', 'm video/mp4'],
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pump();
        final element = tester.element(find.byType(MessageContent));
        expect(auth.calls, 1);
        expect(platform.sources, isEmpty);
        auth.drained.complete();
        await tester.pumpAndSettle();
        expect(tester.element(find.byType(MessageContent)), same(element));
        expect(auth.calls, 2);
        expect(
          platform.sources.single.httpHeaders['Authorization'],
          'test-proof',
        );
        expect(
          find.byKey(const ValueKey('message-media-video-first-frame:$url')),
          findsOneWidget,
        );
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox());
        await tester.pump();
      },
    );
  }
}
