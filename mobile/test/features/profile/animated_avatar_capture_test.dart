import 'dart:io';

import 'package:buzz/features/profile/animated_avatar_orientation.dart';
import 'package:buzz/features/profile/animated_avatar_capture.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:camera/camera.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image/image.dart' as image;

void main() {
  test('encoded poster preserves avatar scales below one', () {
    final source = image.Image(width: 256, height: 256, numChannels: 4);
    image.fill(source, color: image.ColorRgba8(255, 0, 0, 255));

    final poster = image.decodePng(
      encodeAnimatedAvatarPoster(frame: image.encodePng(source), scale: 0.75),
    )!;

    final edge = poster.getPixel(8, 128);
    final center = poster.getPixel(128, 128);
    expect(edge.r, isNot(255));
    expect(center.r, 255);
  });

  test('capture frame workspaces are isolated', () async {
    final first = await createAnimatedAvatarFrameDirectory(
      parent: Directory.systemTemp,
    );
    final second = await createAnimatedAvatarFrameDirectory(
      parent: Directory.systemTemp,
    );
    addTearDown(() async {
      if (await first.exists()) await first.delete(recursive: true);
      if (await second.exists()) await second.delete(recursive: true);
    });

    expect(first.path, isNot(second.path));
  });

  group('animatedAvatarFrameRotationDegrees', () {
    test('compensates front camera frames for every device orientation', () {
      const expected = {
        DeviceOrientation.portraitUp: 270,
        DeviceOrientation.landscapeRight: 180,
        DeviceOrientation.portraitDown: 90,
        DeviceOrientation.landscapeLeft: 0,
      };

      for (final entry in expected.entries) {
        expect(
          animatedAvatarFrameRotationDegrees(
            sensorOrientation: 270,
            deviceOrientation: entry.key,
            lensDirection: CameraLensDirection.front,
          ),
          entry.value,
        );
      }
    });

    test('compensates back camera frames for every device orientation', () {
      const expected = {
        DeviceOrientation.portraitUp: 90,
        DeviceOrientation.landscapeRight: 180,
        DeviceOrientation.portraitDown: 270,
        DeviceOrientation.landscapeLeft: 0,
      };

      for (final entry in expected.entries) {
        expect(
          animatedAvatarFrameRotationDegrees(
            sensorOrientation: 90,
            deviceOrientation: entry.key,
            lensDirection: CameraLensDirection.back,
          ),
          entry.value,
        );
      }
    });
  });

  testWidgets('completed review frames survive lifecycle changes', (
    tester,
  ) async {
    final lifecycle = _TestLifecycleNotifier();
    Future<ProfileAvatarDraft?> Function()? prepare;
    final frame = image.encodePng(image.Image(width: 2, height: 2));
    await tester.pumpWidget(
      ProviderScope(
        overrides: [appLifecycleProvider.overrideWith(() => lifecycle)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: MediaQuery(
              data: const MediaQueryData(disableAnimations: true),
              child: ExcludeSemantics(
                child: AnimatedAvatarCapture(
                  height: 600,
                  initialFrames: [frame, frame],
                  onPrepareChanged: (value) => prepare = value,
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(
      find.byKey(const ValueKey('animated-avatar-review-preview')),
      findsOneWidget,
    );
    expect(prepare, isNotNull);

    lifecycle.setLifecycle(AppLifecycleState.paused);
    await tester.pump();
    lifecycle.setLifecycle(AppLifecycleState.resumed);
    await tester.pump();

    expect(
      find.byKey(const ValueKey('animated-avatar-review-preview')),
      findsOneWidget,
    );
    expect(prepare, isNotNull);
  });
}

class _TestLifecycleNotifier extends AppLifecycleNotifier {
  AppLifecycleState _lifecycle = AppLifecycleState.resumed;

  @override
  AppLifecycleState build() => _lifecycle;

  void setLifecycle(AppLifecycleState value) {
    _lifecycle = value;
    state = value;
  }
}
