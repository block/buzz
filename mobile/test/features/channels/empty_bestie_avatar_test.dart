import 'package:flutter/foundation.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';
import 'package:buzz/features/channels/empty_bestie_avatar.dart';
import 'package:buzz/shared/agents/agent_avatar_catalog.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

class _NoVideo extends VideoPlayerPlatform {
  int creations = 0;
  @override
  Future<void> init() async {}
  @override
  Future<void> setMixWithOthers(bool value) async {}
  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async {
    creations++;
    throw StateError('The empty Bestie carousel must not request video.');
  }
}

void main() {
  Widget app({bool reducedMotion = false, bool enabled = true}) =>
      ProviderScope(
        child: MaterialApp(
          home: MediaQuery(
            data: MediaQueryData(disableAnimations: reducedMotion),
            child: TickerMode(
              enabled: enabled,
              child: const Center(child: EmptyBestieAvatar()),
            ),
          ),
        ),
      );

  String current(WidgetTester tester) => tester
      .widgetList<AvatarImageContent>(find.byType(AvatarImageContent))
      .last
      .imageUrl!;

  testWidgets('cycles still posters without creating network video players', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final previous = VideoPlayerPlatform.instance;
    final video = _NoVideo();
    VideoPlayerPlatform.instance = video;
    addTearDown(() => VideoPlayerPlatform.instance = previous);
    await tester.pumpWidget(app());
    expect(current(tester), contains('/fuzzies/'));
    await tester.pump(const Duration(milliseconds: 4999));
    expect(current(tester), contains('/fuzzies/'));
    await tester.pump(const Duration(milliseconds: 1));
    expect(find.byType(AvatarImageContent), findsNWidgets(2));
    expect(find.byType(ScaleTransition), findsWidgets);
    await tester.pump(const Duration(milliseconds: 280));
    expect(current(tester), contains('/gloopies/'));
    await tester.pump(const Duration(seconds: 5));
    await tester.pump(const Duration(milliseconds: 280));
    expect(
      current(tester),
      agentAvatarCatalog[AgentAvatarPack.figgies.indices.first].url,
    );
    expect(video.creations, 0);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 10));
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('Reduce Motion keeps a single stationary character', (
    tester,
  ) async {
    await tester.pumpWidget(app(reducedMotion: true));
    final first = current(tester);
    await tester.pump(const Duration(seconds: 15));
    expect(current(tester), first);
    expect(find.byType(AvatarImageContent), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('hidden navigation pauses the carousel', (tester) async {
    await tester.pumpWidget(app(enabled: false));
    final first = current(tester);
    await tester.pump(const Duration(seconds: 15));
    expect(current(tester), first);
    await tester.pumpWidget(app());
    await tester.pump(const Duration(seconds: 5));
    await tester.pump(const Duration(milliseconds: 280));
    expect(current(tester), isNot(first));
    await tester.pumpWidget(const SizedBox());
  });
}
