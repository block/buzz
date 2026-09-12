import 'dart:async';

import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';

class _DeferredAuth extends MediaGetAuthService {
  final ready = Completer<Map<String, String>>();
  int calls = 0;
  _DeferredAuth({this.remote = false})
    : super(baseUrl: 'https://relay.example', nsec: null);
  final bool remote;
  @override
  bool get isRemote => remote;
  @override
  Future<Map<String, String>> headersFor(String url) {
    calls++;
    return ready.future;
  }
}

class _VideoPlatform extends VideoPlayerPlatform {
  final events = StreamController<VideoEvent>.broadcast();
  final sources = <DataSource>[];
  int plays = 0;
  int disposals = 0;
  @override
  Future<void> init() async {}
  @override
  Future<int> createWithOptions(VideoCreationOptions options) async {
    sources.add(options.dataSource);
    return 1;
  }

  @override
  Stream<VideoEvent> videoEventsFor(int playerId) => events.stream;
  @override
  Future<void> dispose(int playerId) async => disposals++;
  @override
  Future<void> play(int playerId) async => plays++;
  @override
  Future<void> pause(int playerId) async {}
  @override
  Future<void> setLooping(int playerId, bool looping) async {}
  @override
  Future<void> setVolume(int playerId, double volume) async {}
  @override
  Future<void> setPlaybackSpeed(int playerId, double speed) async {}
  @override
  Widget buildViewWithOptions(VideoViewOptions options) => const SizedBox();
  void finishInitialization() => events.add(
    VideoEvent(
      eventType: VideoEventType.initialized,
      duration: const Duration(seconds: 10),
      size: const Size(320, 180),
    ),
  );
}

void main() {
  late _DeferredAuth auth;
  late _VideoPlatform platform;
  late VideoPlayerPlatform previousPlatform;
  late int httpRequests;

  setUp(() {
    previousPlatform = VideoPlayerPlatform.instance;
    httpRequests = 0;
  });
  tearDown(() async {
    VideoPlayerPlatform.instance = previousPlatform;
    await platform.events.close();
  });

  Future<void> open(
    WidgetTester tester, {
    bool remote = false,
    http.Client? client,
  }) {
    auth = _DeferredAuth(remote: remote);
    platform = _VideoPlatform();
    VideoPlayerPlatform.instance = platform;
    return tester.pumpWidget(
      ProviderScope(
        overrides: [
          mediaVideoStreamingSupportedProvider.overrideWithValue(
            defaultTargetPlatform == TargetPlatform.android,
          ),
          mediaGetAuthServiceProvider.overrideWithValue(auth),
          mediaHttpClientProvider.overrideWithValue(
            client ??
                MockClient((_) async {
                  httpRequests++;
                  return http.Response('', 500);
                }),
          ),
        ],
        child: const MaterialApp(
          home: MediaVideoViewerPage(
            videoUrl: 'https://relay.example/media/video.mp4',
          ),
        ),
      ),
    );
  }

  for (final responseKind in ['redirect', 'oversize']) {
    testWidgets(
      'corporate Android video rejects $responseKind without native network',
      (tester) async {
        final client = MockClient.streaming((request, body) async {
          httpRequests++;
          expect(request.followRedirects, false);
          expect(request.headers['Authorization'], 'captured A auth');
          return http.StreamedResponse(
            const Stream.empty(),
            responseKind == 'redirect' ? 302 : 200,
            contentLength: responseKind == 'oversize' ? 257 * 1024 * 1024 : 0,
            headers: {'location': 'https://attacker.example/media/video.mp4'},
          );
        });
        await open(tester, remote: true, client: client);
        auth.ready.complete({'Authorization': 'captured A auth'});
        await tester.pump();
        await tester.runAsync(() => Future<void>.delayed(Duration.zero));
        await tester.pump();
        expect(httpRequests, 1);
        expect(platform.sources, isEmpty);
        expect(platform.plays, 0);
        expect(find.text('Failed to load video'), findsOneWidget);
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox());
      },
      variant: TargetPlatformVariant.only(TargetPlatform.android),
    );
  }

  testWidgets(
    'corporate auth failure never falls back to unsigned video',
    (tester) async {
      await open(tester, remote: true);
      auth.ready.completeError(StateError('Corporate login required'));
      await tester.pump();
      expect(httpRequests, 0);
      expect(platform.sources, isEmpty);
      expect(find.text('Failed to load video'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  for (final target in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets(
      '$target disposal during auth creates no player or request',
      (tester) async {
        await open(tester);
        expect(auth.calls, 1);
        await tester.pumpWidget(const SizedBox());
        auth.ready.complete({'Authorization': 'captured A auth'});
        await tester.pump();
        expect(platform.sources, isEmpty);
        expect(platform.plays, 0);
        expect(httpRequests, 0);
        expect(tester.takeException(), isNull);
      },
      variant: TargetPlatformVariant.only(target),
    );
  }

  testWidgets(
    'disposal during initialization cleans up without play or fallback',
    (tester) async {
      await open(tester);
      auth.ready.complete({'Authorization': 'captured A auth'});
      await tester.pump();
      expect(platform.sources.single.httpHeaders, {
        'Authorization': 'captured A auth',
      });
      await tester.pumpWidget(const SizedBox());
      platform.finishInitialization();
      await tester.pump();
      expect(platform.plays, 0);
      // Stream cancellation can complete outside the widget fake-async zone.
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      await tester.pump();
      expect(platform.disposals, 1);
      expect(httpRequests, 0);
      expect(auth.calls, 1);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  testWidgets(
    'initialization failure after disposal does not enter fallback',
    (tester) async {
      await open(tester);
      auth.ready.complete({});
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      platform.events.addError(
        PlatformException(code: 'init', message: 'initialization failed'),
      );
      await tester.pump();
      // Stream cancellation can complete outside the widget fake-async zone.
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      await tester.pump();
      expect(platform.disposals, 1);
      expect(platform.plays, 0);
      expect(auth.calls, 1);
      expect(httpRequests, 0);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  testWidgets(
    'live viewer still initializes, plays, and disposes normally',
    (tester) async {
      await open(tester);
      auth.ready.complete({'Authorization': 'captured A auth'});
      await tester.pump();
      platform.finishInitialization();
      await tester.pump();
      expect(platform.sources, hasLength(1));
      expect(platform.plays, 1);
      expect(platform.disposals, 0);
      expect(httpRequests, 0);
      await tester.pumpWidget(const SizedBox());
      // Stream cancellation can complete outside the widget fake-async zone.
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      await tester.pump();
      expect(platform.disposals, 1);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );
}
