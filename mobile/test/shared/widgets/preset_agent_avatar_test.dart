import 'package:buzz/shared/widgets/preset_agent_avatar.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';

class _Player extends VideoPlayerPlatform {
  bool playing = false;
  double volume = 1;
  @override
  Future<void> init() async {}
  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async => 1;
  @override
  Stream<VideoEvent> videoEventsFor(int id) {
    return Stream.value(
      VideoEvent(
        eventType: VideoEventType.initialized,
        duration: const Duration(seconds: 2),
        size: const Size(100, 100),
      ),
    );
  }

  @override
  Widget buildView(int id) => const SizedBox(key: Key('video'));
  @override
  Future<void> play(int id) async {
    playing = true;
  }

  @override
  Future<void> pause(int id) async {
    playing = false;
  }

  @override
  Future<void> setLooping(int id, bool looping) async {}
  @override
  Future<void> setVolume(int id, double value) async {
    volume = value;
  }

  @override
  Future<void> setPlaybackSpeed(int id, double speed) async {}
  @override
  Future<void> setMixWithOthers(bool value) async {}
  @override
  Future<Duration> getPosition(int id) async => Duration.zero;
  @override
  Future<void> dispose(int id) async {}
}

void main() {
  testWidgets(
    'preset plays silently, pauses when hidden, and respects reduced motion',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final original = VideoPlayerPlatform.instance;
      final player = _Player();
      VideoPlayerPlatform.instance = player;
      addTearDown(() => VideoPlayerPlatform.instance = original);
      Widget surface({
        bool visible = true,
        bool reduced = false,
      }) => ProviderScope(
        child: MaterialApp(
          home: MediaQuery(
            data: MediaQueryData(disableAnimations: reduced),
            child: TickerMode(
              enabled: visible,
              child: const PresetAgentAvatar(
                imageUrl:
                    'https://dwwgwmfqqjotj.cloudfront.net/avatars/20260925T154625166Z/poster/gloopies/gloopies-13.png',
                size: 50.4,
                fallback: Text('Swiss'),
              ),
            ),
          ),
        ),
      );
      await tester.pumpWidget(surface());
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('video')), findsOneWidget);
      expect(player.playing, isTrue);
      expect(player.volume, 0);
      await tester.pumpWidget(surface(visible: false));
      await tester.pump();
      expect(player.playing, isFalse);
      await tester.pumpWidget(surface());
      await tester.pump();
      expect(player.playing, isTrue);
      await tester.pumpWidget(surface(reduced: true));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('video')), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    },
  );
  testWidgets('Android uses transparent renderer and pauses offscreen', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final messenger = tester.binding.defaultBinaryMessenger;
    final playback = <bool>[];
    var rejectPlayback = false;
    final channels = <MethodChannel>[];
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final args = call.arguments as Map;
        expect(args['viewType'], 'buzz/transparent-avatar');
        final channel = MethodChannel('buzz/transparent-avatar/${args['id']}');
        channels.add(channel);
        messenger.setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'setPlaying') {
            if (rejectPlayback) throw PlatformException(code: 'unavailable');
            playback.add(call.arguments as bool);
          }
          return null;
        });
        return 1;
      }
      if (call.method == 'resize') {
        final args = call.arguments as Map;
        return {'width': args['width'], 'height': args['height']};
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
      for (final channel in channels) {
        messenger.setMockMethodCallHandler(channel, null);
      }
    });
    Widget surface({
      bool visible = true,
      bool reduced = false,
    }) => ProviderScope(
      child: MaterialApp(
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: reduced),
          child: TickerMode(
            enabled: visible,
            child: const PresetAgentAvatar(
              imageUrl:
                  'https://dwwgwmfqqjotj.cloudfront.net/avatars/20260925T154625166Z/poster/gloopies/gloopies-13.png',
              size: 50.4,
              fallback: Text('Swiss'),
            ),
          ),
        ),
      ),
    );
    await tester.pumpWidget(surface());
    await tester.pumpAndSettle();
    expect(find.byType(AndroidView), findsOneWidget);
    expect(playback.last, true);
    await tester.pumpWidget(surface(visible: false));
    await tester.pumpAndSettle();
    expect(playback.last, false);
    await tester.pumpWidget(surface());
    await tester.pumpAndSettle();
    expect(playback.last, true);
    await tester.pumpWidget(surface(reduced: true));
    await tester.pumpAndSettle();
    expect(find.byType(AndroidView), findsNothing);
    rejectPlayback = true;
    await tester.pumpWidget(surface());
    await tester.pumpAndSettle();
    expect(find.byType(AndroidView), findsNothing);
    expect(find.byType(AvatarImageContent), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });
}
