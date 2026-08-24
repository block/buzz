import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' show SemanticsAction;

import 'package:buzz/features/profile/animated_avatar_orientation.dart';
import 'package:buzz/features/profile/animated_avatar_capture.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
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

  test('normalizes the fixed portrait camera sensor correction', () {
    expect(
      animatedAvatarPortraitFrameRotationDegrees(sensorOrientation: 270),
      270,
    );
    expect(
      animatedAvatarPortraitFrameRotationDegrees(sensorOrientation: 450),
      90,
    );
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

  testWidgets('poster scrubber supports semantic adjustment actions', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final frames = [
      for (var index = 0; index < 3; index++)
        image.encodePng(image.Image(width: 2, height: 2)),
    ];
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AnimatedAvatarCapture(
              height: 600,
              initialFrames: frames,
              onPrepareChanged: (_) {},
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.text('Frame'));
    await tester.pump();

    final scrubber = find.bySemanticsLabel('Choose still frame');
    expect(scrubber, findsOneWidget);
    final initialSemantics = tester.getSemantics(scrubber);
    expect(initialSemantics.value, '1 of 3');
    final initialData = initialSemantics.getSemanticsData();
    expect(initialData.hasAction(SemanticsAction.increase), isTrue);
    expect(initialData.hasAction(SemanticsAction.decrease), isFalse);

    final semanticsWidget = tester.widget<Semantics>(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics &&
            widget.properties.label == 'Choose still frame',
      ),
    );
    semanticsWidget.properties.onIncrease!();
    await tester.pump();
    expect(tester.getSemantics(scrubber).value, '2 of 3');
    semantics.dispose();
  });

  testWidgets('review preview exposes accessible repositioning actions', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final frame = image.encodePng(image.Image(width: 2, height: 2));
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AnimatedAvatarCapture(
              height: 600,
              initialFrames: [frame],
              onPrepareChanged: (_) {},
            ),
          ),
        ),
      ),
    );
    await tester.pump();

    final position = find.bySemanticsLabel('Avatar position');
    expect(position, findsOneWidget);
    final positionWidget = tester.widget<Semantics>(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics && widget.properties.label == 'Avatar position',
      ),
    );
    final actions = positionWidget.properties.customSemanticsActions!;
    expect(
      actions.keys.map((action) => action.label),
      containsAll(['Move left', 'Move right', 'Move up', 'Move down']),
    );
    actions.entries
        .firstWhere((entry) => entry.key.label == 'Move right')
        .value();
    await tester.pump();
    expect(tester.getSemantics(position).value, '10 horizontal, 0 vertical');
    semantics.dispose();
  });

  testWidgets('an existing Save callback uses the latest cutout position', (
    tester,
  ) async {
    final source = image.Image(width: 256, height: 256, numChannels: 4);
    image.fillRect(
      source,
      x1: 108,
      y1: 108,
      x2: 147,
      y2: 147,
      color: image.ColorRgba8(0, 255, 0, 255),
    );
    final frame = image.encodePng(source);
    Future<ProfileAvatarDraft?> Function()? prepare;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          appLifecycleProvider.overrideWith(_TestLifecycleNotifier.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AnimatedAvatarCapture(
              height: 600,
              initialFrames: [frame],
              onPrepareChanged: (value) => prepare = value,
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    final originalPrepare = prepare!;
    final centered = await tester.runAsync(originalPrepare);

    final positionWidget = tester.widget<Semantics>(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics && widget.properties.label == 'Avatar position',
      ),
    );
    positionWidget.properties.customSemanticsActions!.entries
        .firstWhere((entry) => entry.key.label == 'Move right')
        .value();
    await tester.pump();
    final moved = await tester.runAsync(originalPrepare);

    expect(
      _greenCenterX((moved! as ProfileAnimatedAvatarDraft).poster),
      greaterThan(
        _greenCenterX((centered! as ProfileAnimatedAvatarDraft).poster) + 3,
      ),
    );
  });
}

double _greenCenterX(Uint8List bytes) {
  final decoded = image.decodePng(bytes)!;
  final matchingX = <int>[];
  for (final pixel in decoded) {
    if (pixel.g > 200 && pixel.r < 30 && pixel.b < 30) {
      matchingX.add(pixel.x);
    }
  }
  return matchingX.reduce((left, right) => left + right) / matchingX.length;
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
