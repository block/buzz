import 'dart:async';
import 'dart:io';

import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';
import 'package:video_player/video_player.dart';

class _TempDirectory extends Fake
    with MockPlatformInterfaceMixin
    implements PathProviderPlatform {
  @override
  Future<String?> getTemporaryPath() async => Directory.systemTemp.path;
}

class _Player extends VideoPlayerPlatform {
  _Player({this.size = const Size(1920, 1080)});
  final Size size;
  final started = Completer<void>();
  final events = StreamController<VideoEvent>();
  Duration position = Duration.zero;
  double volume = 1;
  double speed = 1;
  int seeks = 0;
  bool failNextSeek = false;
  bool deferSeeks = false;
  final pendingSeeks = <Completer<void>>[];
  int plays = 0;
  int pauses = 0;

  @override
  Future<void> init() async {}
  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async => 1;
  @override
  Stream<VideoEvent> videoEventsFor(int playerId) {
    events.onListen = () => events.add(
      VideoEvent(
        eventType: VideoEventType.initialized,
        duration: const Duration(minutes: 2),
        size: size,
      ),
    );
    return events.stream;
  }

  @override
  Widget buildView(int playerId) => const ColoredBox(color: Colors.blueGrey);
  @override
  Future<void> play(int playerId) async {
    plays++;
    if (!started.isCompleted) started.complete();
  }

  @override
  Future<void> pause(int playerId) async {
    pauses++;
  }

  @override
  Future<void> setLooping(int playerId, bool looping) async {}
  @override
  Future<void> setVolume(int playerId, double volume) async {
    this.volume = volume;
  }

  @override
  Future<void> setPlaybackSpeed(int playerId, double speed) async {
    this.speed = speed;
  }

  @override
  Future<void> setMixWithOthers(bool mixWithOthers) async {}
  @override
  Future<Duration> getPosition(int playerId) async => position;
  @override
  Future<void> seekTo(int playerId, Duration position) async {
    seeks++;
    if (failNextSeek) {
      failNextSeek = false;
      throw PlatformException(code: 'seek_failed');
    }
    if (deferSeeks) {
      final pending = Completer<void>();
      pendingSeeks.add(pending);
      await pending.future;
    }
    this.position = position;
  }

  @override
  Future<void> dispose(int playerId) async {
    await events.close();
  }
}

final _surface = find.byKey(
  const ValueKey('message-media-video-viewer-gesture'),
);
final _play = find.byKey(
  const ValueKey('message-media-video-viewer-play-pause'),
);
final _timeline = find.byKey(
  const ValueKey('message-media-video-viewer-timeline'),
);
final _chrome = find.byKey(
  const ValueKey('message-media-video-viewer-controls'),
);

double _opacity(WidgetTester tester) => tester
    .widget<AnimatedOpacity>(
      find.descendant(of: _chrome, matching: find.byType(AnimatedOpacity)),
    )
    .opacity;

Future<_Player> _pumpVideo(
  WidgetTester tester, {
  bool reduceMotion = false,
  bool accessible = false,
  bool pushRoute = false,
  double textScale = 1,
  VoidCallback? onReply,
  Size videoSize = const Size(1920, 1080),
}) async {
  final navigatorKey = GlobalKey<NavigatorState>();
  final oldPlayer = VideoPlayerPlatform.instance;
  final oldPath = PathProviderPlatform.instance;
  final player = _Player(size: videoSize);
  VideoPlayerPlatform.instance = player;
  PathProviderPlatform.instance = _TempDirectory();
  addTearDown(() {
    VideoPlayerPlatform.instance = oldPlayer;
    PathProviderPlatform.instance = oldPath;
  });
  addTearDown(() => _disposeVideo(tester));
  final client = MockClient(
    (_) async => http.Response.bytes([0, 1, 2, 3], 200),
  );
  addTearDown(client.close);
  await tester.runAsync(() async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          mediaHttpClientProvider.overrideWithValue(client),
          mediaGetAuthServiceProvider.overrideWithValue(
            MediaGetAuthService(baseUrl: 'https://relay.test', nsec: null),
          ),
        ],
        child: MaterialApp(
          navigatorKey: navigatorKey,
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              disableAnimations: reduceMotion,
              accessibleNavigation: accessible,
              textScaler: TextScaler.linear(textScale),
            ),
            child: child!,
          ),
          home: pushRoute
              ? const Scaffold(body: Text('Viewer closed'))
              : MediaVideoViewerPage(
                  videoUrl: 'https://relay.test/video.mp4',
                  onReply: onReply,
                ),
        ),
      ),
    );
    if (pushRoute) {
      unawaited(
        navigatorKey.currentState!.push(
          MaterialPageRoute<void>(
            builder: (_) => MediaVideoViewerPage(
              videoUrl: 'https://relay.test/video.mp4',
              onReply: onReply,
            ),
          ),
        ),
      );
      await tester.pump();
    }
    await Future<void>.delayed(const Duration(milliseconds: 50));
  });
  for (
    var attempt = 0;
    attempt < 100 && !player.started.isCompleted;
    attempt++
  ) {
    await tester.pump();
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 10)),
    );
  }
  expect(player.started.isCompleted, isTrue, reason: 'Video initialized');
  await tester.pump();
  await tester.pump();
  return player;
}

Future<void> _disposeVideo(WidgetTester tester) async {
  await tester.pumpWidget(const SizedBox.shrink());
  await tester.runAsync(() async {
    await Future<void>.delayed(const Duration(milliseconds: 10));
  });
}

void main() {
  testWidgets(
    'playing controls fade after idle and video taps hide or restore them',
    (tester) async {
      final player = await _pumpVideo(tester);
      final initialPauses = player.pauses;
      expect(_opacity(tester), 1);
      await tester.pump(const Duration(milliseconds: 2900));
      expect(_opacity(tester), 1);
      await tester.pump(const Duration(milliseconds: 101));
      expect(_opacity(tester), 0);
      await tester.pump(const Duration(milliseconds: 200));
      expect(_play.hitTestable(), findsNothing);
      expect(find.bySemanticsLabel('Pause video'), findsNothing);
      await tester.tap(_surface);
      await tester.pumpAndSettle();
      expect(_opacity(tester), 1);
      expect(_play.hitTestable(), findsOneWidget);
      await tester.tap(_surface);
      await tester.pumpAndSettle();
      expect(_opacity(tester), 0);
      expect(
        player.pauses,
        initialPauses,
        reason: 'Tapping the picture only toggles chrome',
      );
      await _disposeVideo(tester);
    },
  );

  testWidgets(
    'pausing keeps controls visible and resume starts a fresh timer',
    (tester) async {
      final player = await _pumpVideo(tester);
      final initialPauses = player.pauses;
      await tester.tap(_play);
      await tester.pump();
      await tester.pump(const Duration(seconds: 5));
      expect(_opacity(tester), 1);
      expect(player.pauses, initialPauses + 1);
      await tester.tap(_surface);
      await tester.pumpAndSettle();
      expect(
        _opacity(tester),
        0,
        reason: 'Paused controls can still be hidden manually',
      );
      await tester.tap(_surface);
      await tester.pumpAndSettle();
      await tester.tap(_play);
      await tester.pump();
      await tester.pump(const Duration(seconds: 2));
      expect(_opacity(tester), 1);
      await tester.pump(const Duration(seconds: 1));
      expect(_opacity(tester), 0);
      await _disposeVideo(tester);
    },
  );

  testWidgets(
    'scrubbing holds controls open, seeks on release, then restarts idle',
    (tester) async {
      final player = await _pumpVideo(tester);
      await tester.pump(const Duration(seconds: 2));
      final rect = tester.getRect(_timeline);
      final gesture = await tester.startGesture(rect.center);
      await gesture.moveBy(Offset(rect.width * 0.2, 0));
      await tester.pump();
      await tester.pump(const Duration(seconds: 4));
      expect(_opacity(tester), 1);
      expect(
        player.seeks,
        0,
        reason: 'Do not flood native decoder with pending seeks',
      );
      await gesture.up();
      await tester.pump();
      expect(player.position.inSeconds, greaterThan(60));
      expect(player.seeks, 1);
      await tester.pump(const Duration(seconds: 2));
      expect(_opacity(tester), 1);
      await tester.pump(const Duration(seconds: 1));
      expect(_opacity(tester), 0);
      await _disposeVideo(tester);
    },
  );

  for (final oldSeekFails in [false, true]) {
    for (final releaseNewScrub in [false, true]) {
      testWidgets(
        'stale seek cannot end newer scrub: failure=$oldSeekFails, released=$releaseNewScrub',
        (tester) async {
          final player = await _pumpVideo(tester);
          player.deferSeeks = true;
          final rect = tester.getRect(_timeline);
          await tester.tapAt(
            Offset(rect.left + rect.width * 0.25, rect.center.dy),
          );
          await tester.pump();
          expect(player.pendingSeeks, hasLength(1));
          final newer = await tester.startGesture(rect.center);
          await newer.moveTo(
            Offset(rect.left + rect.width * 0.8, rect.center.dy),
          );
          await tester.pump();
          final newPosition = tester.widget<Slider>(_timeline).value;
          expect(newPosition, greaterThan(60000));
          if (releaseNewScrub) {
            await newer.up();
            await tester.pump();
            expect(player.pendingSeeks, hasLength(2));
          }
          if (oldSeekFails) {
            player.pendingSeeks.first.completeError(
              PlatformException(code: 'old_seek_failed'),
            );
          } else {
            player.pendingSeeks.first.complete();
          }
          await tester.pump();
          expect(
            tester.widget<Slider>(_timeline).value,
            newPosition,
            reason: 'Old completion must not clear the newer thumb',
          );
          expect(find.text('Could not seek in this video.'), findsNothing);
          expect(find.text('Retry'), findsNothing);
          await tester.pump(const Duration(seconds: 4));
          expect(
            _opacity(tester),
            1,
            reason: 'New scrub still owns the idle hold',
          );
          if (!releaseNewScrub) {
            await newer.up();
            await tester.pump();
          }
          expect(player.pendingSeeks, hasLength(2));
          player.pendingSeeks.last.complete();
          await tester.pump();
          await tester.pump();
          expect(player.position.inMilliseconds, newPosition.round());
          await tester.pump(const Duration(seconds: 3));
          expect(
            _opacity(tester),
            0,
            reason: 'Only current completion resumes idle',
          );
        },
      );
    }
  }

  testWidgets('failed seeks report an error and retry the requested position', (
    tester,
  ) async {
    final player = await _pumpVideo(tester);
    player.failNextSeek = true;
    final rect = tester.getRect(_timeline);
    await tester.tapAt(Offset(rect.left + rect.width * 0.75, rect.center.dy));
    await tester.pumpAndSettle();
    expect(player.seeks, 1);
    expect(player.position, Duration.zero);
    expect(find.text('Could not seek in this video.'), findsOneWidget);
    // Feedback stays actionable even after the transport chrome fades.
    await tester.pump(const Duration(seconds: 3));
    expect(_opacity(tester), 0);
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(player.seeks, 2);
    expect(player.position.inSeconds, greaterThan(80));
    expect(find.text('Could not seek in this video.'), findsNothing);
    expect(_opacity(tester), 1);
    await _disposeVideo(tester);
  });

  testWidgets('mute and playback speed work without hiding controls', (
    tester,
  ) async {
    final player = await _pumpVideo(tester);
    await tester.pump(const Duration(seconds: 2));
    await tester.tap(
      find.byKey(const ValueKey('message-media-video-viewer-mute')),
    );
    await tester.pump();
    expect(player.volume, 0);
    await tester.pump(const Duration(seconds: 2));
    expect(_opacity(tester), 1);
    for (final expected in [1.25, 1.5, 1.75, 2.0, 0.5, 0.75, 1.0]) {
      await tester.tap(
        find.byKey(const ValueKey('message-media-video-viewer-speed')),
      );
      await tester.pump();
      expect(player.speed, expected);
      expect(find.byType(CheckedPopupMenuItem<double>), findsNothing);
      await tester.pump(const Duration(seconds: 2));
      expect(_opacity(tester), 1);
    }
    await tester.pump(const Duration(seconds: 3));
    expect(_opacity(tester), 0);
    await _disposeVideo(tester);
  });

  testWidgets(
    'accessible navigation keeps controls available and reduce motion is instant',
    (tester) async {
      final semantics = tester.ensureSemantics();
      await _pumpVideo(tester, accessible: true, reduceMotion: true);
      await tester.pump(const Duration(seconds: 10));
      expect(_opacity(tester), 1);
      await tester.tap(_surface);
      await tester.pump();
      expect(_opacity(tester), 0);
      expect(find.bySemanticsLabel('Pause video'), findsNothing);
      final restore = find.bySemanticsLabel('Show video controls');
      expect(restore, findsOneWidget);
      final restoreNode = tester.getSemantics(restore);
      expect(
        restoreNode.getSemanticsData().hasAction(SemanticsAction.tap),
        isTrue,
      );
      final semanticsOwner = tester
          .renderObject(_surface)
          .owner!
          .semanticsOwner!;
      final tapStops = <SemanticsData>[];
      void countTapStops(SemanticsNode node) {
        if (node.getSemanticsData().hasAction(SemanticsAction.tap)) {
          tapStops.add(node.getSemanticsData());
        }
        node.visitChildren((child) {
          countTapStops(child);
          return true;
        });
      }

      countTapStops(semanticsOwner.rootSemanticsNode!);
      expect(
        tapStops,
        hasLength(1),
        reason: 'One named restore stop, no anonymous duplicate',
      );
      final fade = tester.widget<AnimatedOpacity>(
        find.descendant(of: _chrome, matching: find.byType(AnimatedOpacity)),
      );
      expect(fade.duration, Duration.zero);
      semanticsOwner.performAction(restoreNode.id, SemanticsAction.tap);
      await tester.pump();
      expect(_opacity(tester), 1);
      expect(find.bySemanticsLabel('Hide video controls'), findsOneWidget);
      expect(find.bySemanticsLabel('Show video controls'), findsNothing);
      tapStops.clear();
      countTapStops(semanticsOwner.rootSemanticsNode!);
      expect(
        tapStops.map(
          (node) => node.label.isNotEmpty ? node.label : node.tooltip,
        ),
        containsAll([
          'Hide video controls',
          'Close video viewer',
          'Pause video',
        ]),
      );
      semantics.dispose();
      await _disposeVideo(tester);
    },
  );

  testWidgets('buffering and playback completion restore controls', (
    tester,
  ) async {
    final player = await _pumpVideo(tester);
    await tester.pump(const Duration(seconds: 3));
    expect(_opacity(tester), 0);
    player.events.add(VideoEvent(eventType: VideoEventType.bufferingStart));
    await tester.pump();
    await tester.pump(const Duration(seconds: 4));
    expect(_opacity(tester), 1);
    player.events.add(VideoEvent(eventType: VideoEventType.bufferingEnd));
    await tester.pump();
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    expect(_opacity(tester), 0);
    player.events.add(VideoEvent(eventType: VideoEventType.completed));
    await tester.pumpAndSettle();
    expect(_opacity(tester), 1);
    expect(find.byTooltip('Play video'), findsOneWidget);
    await _disposeVideo(tester);
  });

  testWidgets(
    'iOS timeline requests native Liquid Glass and fades with the toolbar',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final nativeChannels = <MethodChannel>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        (call) async {
          if (call.method == 'create') {
            final args = call.arguments as Map;
            final native = MethodChannel('${args['viewType']}/${args['id']}');
            nativeChannels.add(native);
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              native,
              (_) async => null,
            );
          }
          return null;
        },
      );
      addTearDown(() {
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform_views,
          null,
        );
        for (final channel in nativeChannels) {
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            null,
          );
        }
      });
      const channel = MethodChannel('buzz/concentric_sheet_surface');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        (call) async => true,
      );
      addTearDown(() {
        debugDefaultTargetPlatformOverride = null;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        );
      });
      await _pumpVideo(tester);
      await tester.pumpAndSettle();
      final native = tester.widgetList<UiKitView>(
        find.byWidgetPredicate(
          (widget) =>
              widget is UiKitView &&
              widget.viewType == 'buzz/concentric_sheet_surface',
        ),
      );
      expect(native, hasLength(3));
      for (final surface in native) {
        expect(
          (surface.creationParams! as Map<String, Object>)['usesGlass'],
          isTrue,
        );
      }
      await tester.tap(_surface);
      await tester.pumpAndSettle();
      expect(_opacity(tester), 0);
      expect(_play.hitTestable(), findsNothing);
      await _disposeVideo(tester);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  for (final width in [320.0, 420.0]) {
    testWidgets('transport and reply actions use two rows at $width pixels', (
      tester,
    ) async {
      tester.view.physicalSize = Size(width, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await _pumpVideo(tester, onReply: () {});
      final centerY = tester.getCenter(_play).dy;
      for (final control in [
        _timeline,
        find.byKey(const ValueKey('message-media-video-viewer-current-time')),
        find.byKey(const ValueKey('message-media-video-viewer-total-time')),
      ]) {
        expect(tester.getCenter(control).dy, closeTo(centerY, 0.01));
        expect(tester.getRect(control).left, greaterThanOrEqualTo(0));
        expect(tester.getRect(control).right, lessThanOrEqualTo(width));
      }
      final reply = find.byKey(
        const ValueKey('message-media-video-viewer-reply-thread'),
      );
      final lowerY = tester.getCenter(reply).dy;
      expect(lowerY, greaterThan(centerY));
      for (final control in [
        find.byKey(const ValueKey('message-media-video-viewer-speed')),
        find.byKey(const ValueKey('message-media-video-viewer-mute')),
      ]) {
        expect(tester.getCenter(control).dy, closeTo(lowerY, 0.01));
        expect(tester.getRect(control).right, lessThanOrEqualTo(width));
      }
      expect(tester.getSize(_timeline).width, greaterThanOrEqualTo(64));
      expect(tester.takeException(), isNull);
      await _disposeVideo(tester);
    });
  }

  testWidgets(
    'pinch zoom and pan preserve playback and controls, then reset to fit',
    (tester) async {
      final player = await _pumpVideo(tester);
      final viewer = tester.widget<InteractiveViewer>(
        find.byKey(const ValueKey('message-media-video-viewer-zoom')),
      );
      final transform = viewer.transformationController!;
      final center = tester.getCenter(_surface);
      final pausesBefore = player.pauses;

      Future<void> pinch(double start, double end) async {
        final left = await tester.startGesture(
          center - Offset(start, 0),
          pointer: 1,
        );
        final right = await tester.startGesture(
          center + Offset(start, 0),
          pointer: 2,
        );
        for (var step = 1; step <= 8; step++) {
          final radius = start + (end - start) * step / 8;
          await left.moveTo(center - Offset(radius, 0));
          await right.moveTo(center + Offset(radius, 0));
          await tester.pump(const Duration(milliseconds: 16));
        }
        await left.up();
        await right.up();
        await tester.pumpAndSettle();
      }

      await pinch(30, 240);
      expect(transform.value.getMaxScaleOnAxis(), greaterThan(1));
      expect(transform.value.getMaxScaleOnAxis(), lessThanOrEqualTo(4));
      final beforePan = Matrix4.copy(transform.value);
      final pan = await tester.startGesture(center);
      await pan.moveBy(const Offset(0, 150));
      await tester.pump();
      await pan.moveBy(const Offset(0, 40));
      await tester.pump();
      await tester.pump(const Duration(seconds: 4));
      expect(_opacity(tester), 1);
      await pan.up();
      await tester.pumpAndSettle();
      expect(transform.value.storage, isNot(orderedEquals(beforePan.storage)));
      expect(player.pauses, pausesBefore);
      expect(
        tester.widget<Scaffold>(find.byType(Scaffold)).backgroundColor!.a,
        1,
      );
      expect(_play.hitTestable(), findsOneWidget);
      await tester.tap(_play);
      await tester.pump();
      expect(player.pauses, pausesBefore + 1);

      await pinch(240, 10);
      expect(
        transform.value.storage,
        orderedEquals(Matrix4.identity().storage),
      );
      expect(tester.takeException(), isNull);
      await _disposeVideo(tester);
    },
  );

  for (final reduceMotion in [false, true]) {
    testWidgets(
      'double tap recenters video and enables swipe to close (reduced motion: $reduceMotion)',
      (tester) async {
        final player = await _pumpVideo(
          tester,
          pushRoute: true,
          reduceMotion: reduceMotion,
        );
        await tester.pumpAndSettle();
        final viewer = tester.widget<InteractiveViewer>(
          find.byKey(const ValueKey('message-media-video-viewer-zoom')),
        );
        final transform = viewer.transformationController!;
        transform.value = Matrix4.identity()
          ..translateByDouble(-120, -100, 0, 1)
          ..scaleByDouble(2, 2, 1, 1);
        await tester.pump();
        final initialPauses = player.pauses;
        final initialPlays = player.plays;
        // Reset while a zoomed pan still has momentum.
        await tester.fling(_surface, const Offset(0, 120), 1500);
        await tester.pump(const Duration(milliseconds: 16));
        await tester.tap(_surface);
        await tester.pump(const Duration(milliseconds: 50));
        await tester.tap(_surface);
        await tester.pumpAndSettle();
        expect(
          transform.value.storage,
          orderedEquals(Matrix4.identity().storage),
        );
        await tester.pump(const Duration(milliseconds: 500));
        expect(
          transform.value.storage,
          orderedEquals(Matrix4.identity().storage),
        );
        expect(player.pauses, initialPauses);
        expect(player.plays, initialPlays);
        expect(
          _opacity(tester),
          1,
          reason: 'Double tap does not toggle the controls',
        );
        expect(
          tester
              .widget<InteractiveViewer>(find.byType(InteractiveViewer))
              .panEnabled,
          isFalse,
        );
        await tester.drag(_surface, const Offset(0, 160));
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('message-media-video-viewer')),
          findsNothing,
        );
        expect(find.text('Viewer closed'), findsOneWidget);
        await _disposeVideo(tester);
      },
    );
  }

  testWidgets('a second finger cancels a video dismiss drag', (tester) async {
    final player = await _pumpVideo(tester, reduceMotion: true);
    final center = tester.getCenter(_surface);
    final pausesBefore = player.pauses;
    final first = await tester.startGesture(
      center - const Offset(40, 0),
      pointer: 1,
    );
    await first.moveBy(const Offset(0, 50));
    await tester.pump();
    final second = await tester.startGesture(
      center + const Offset(40, 50),
      pointer: 2,
    );
    await first.moveBy(const Offset(-70, 120));
    await second.moveBy(const Offset(70, 120));
    await tester.pump();
    await first.up();
    await second.up();
    await tester.pumpAndSettle();
    expect(player.pauses, pausesBefore);
    expect(
      tester.widget<Scaffold>(find.byType(Scaffold)).backgroundColor!.a,
      1,
    );
    expect(
      find.byKey(const ValueKey('message-media-video-viewer')),
      findsOneWidget,
    );
    await _disposeVideo(tester);
  });

  testWidgets('portrait video uses the same bounded stage as photos', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    tester.view.viewPadding = const FakeViewPadding(top: 59, bottom: 34);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetViewPadding);
    await _pumpVideo(tester, videoSize: const Size(900, 3000));
    final bounds = tester.getRect(find.byType(VideoPlayer));
    expect(bounds.top, closeTo(59 + 48 + 8, 0.01));
    expect(bounds.bottom, closeTo(844 - 34 - 56 - 16, 0.01));
    expect(bounds.width / bounds.height, closeTo(900 / 3000, 0.001));
    await tester.tap(_surface);
    await tester.pumpAndSettle();
    expect(tester.getRect(find.byType(VideoPlayer)), bounds);
    await _disposeVideo(tester);
  });

  testWidgets('timeline fits a narrow phone with large text', (tester) async {
    tester.view.physicalSize = const Size(320, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await _pumpVideo(tester, textScale: 2);
    expect(_timeline, findsOneWidget);
    expect(_play.hitTestable(), findsOneWidget);
    await _disposeVideo(tester);
  });
}
