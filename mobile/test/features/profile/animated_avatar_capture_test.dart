import 'dart:io';
import 'dart:async';
import 'dart:ui' show SemanticsAction;

import 'package:camera_platform_interface/camera_platform_interface.dart';
import 'package:buzz/features/profile/animated_avatar_capture.dart';
import 'package:buzz/features/profile/animated_avatar_orientation.dart';
import 'package:buzz/features/profile/camera_disposal_barrier.dart';
import 'package:buzz/features/profile/image_avatar_capture.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
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

  test('encoded poster preserves avatar scales above one', () {
    final source = image.Image(width: 256, height: 256, numChannels: 4);
    image.fill(source, color: image.ColorRgba8(255, 0, 0, 255));

    final poster = image.decodePng(
      encodeAnimatedAvatarPoster(frame: image.encodePng(source), scale: 1.5),
    )!;

    expect(poster.getPixel(8, 8).r, 255);
    expect(poster.getPixel(247, 247).r, 255);
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

  test('accounts for device orientation and lens direction', () {
    expect(
      animatedAvatarFrameRotationDegrees(
        sensorOrientation: 270,
        deviceOrientation: DeviceOrientation.landscapeRight,
        lensDirection: CameraLensDirection.front,
      ),
      180,
    );
    expect(
      animatedAvatarFrameRotationDegrees(
        sensorOrientation: 270,
        deviceOrientation: DeviceOrientation.landscapeRight,
        lensDirection: CameraLensDirection.back,
      ),
      0,
    );
  });

  testWidgets('animated capture releases a failed orientation-lock candidate', (
    tester,
  ) async {
    final platform = _TestCameraPlatform(failLockForCameraIds: {1});
    final previousPlatform = CameraPlatform.instance;
    CameraPlatform.instance = platform;
    addTearDown(() {
      CameraPlatform.instance = previousPlatform;
    });

    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: SizedBox(
              height: 600,
              child: AnimatedAvatarCapture(
                height: 600,
                onPrepareChanged: (_) {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(platform.createdCameraIds, [1]);
    expect(platform.disposedCameraIds, [1]);
    expect(find.text('Could not access the camera.'), findsOneWidget);
    final error = find.byWidgetPredicate(
      (widget) => widget is Semantics && widget.properties.liveRegion == true,
    );
    expect(error, findsOneWidget);

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('animated capture waits for disposal before lifecycle resume', (
    tester,
  ) async {
    final platform = _TestCameraPlatform(blockDisposeForCameraIds: {1});
    final previousPlatform = CameraPlatform.instance;
    CameraPlatform.instance = platform;
    addTearDown(() => CameraPlatform.instance = previousPlatform);
    final lifecycle = _TestLifecycleNotifier();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [appLifecycleProvider.overrideWith(() => lifecycle)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: SizedBox(
              height: 600,
              child: AnimatedAvatarCapture(
                height: 600,
                onPrepareChanged: (_) {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(platform.createdCameraIds, [1]);

    lifecycle.setLifecycle(AppLifecycleState.paused);
    await tester.pump();
    expect(platform.disposedCameraIds, [1]);

    lifecycle.setLifecycle(AppLifecycleState.resumed);
    await tester.pump();
    await tester.pump();
    expect(platform.createdCameraIds, [1]);

    platform.failDispose(1);
    await tester.pump();
    await tester.pump();

    expect(platform.createdCameraIds, [1, 2]);
    final record = tester.widget<InkWell>(
      find.byKey(const ValueKey('animated-avatar-record')),
    );
    expect(record.onTap, isNotNull);

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('waits for an in-flight camera before lifecycle replacement', (
    tester,
  ) async {
    final platform = _TestCameraPlatform(
      blockDisposeForCameraIds: {1},
      blockInitializeForCameraIds: {1},
    );
    final previousPlatform = CameraPlatform.instance;
    CameraPlatform.instance = platform;
    addTearDown(() => CameraPlatform.instance = previousPlatform);
    final lifecycle = _TestLifecycleNotifier();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [appLifecycleProvider.overrideWith(() => lifecycle)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: SizedBox(
              height: 600,
              child: AnimatedAvatarCapture(
                height: 600,
                onPrepareChanged: (_) {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(platform.createdCameraIds, [1]);

    lifecycle.setLifecycle(AppLifecycleState.paused);
    await tester.pump();
    lifecycle.setLifecycle(AppLifecycleState.resumed);
    await tester.pump();
    await tester.pump();
    expect(platform.createdCameraIds, [1]);

    platform.completeInitialize(1);
    await tester.pump();
    await tester.pump();
    expect(platform.disposedCameraIds, [1]);
    expect(platform.createdCameraIds, [1]);

    platform.completeDispose(1);
    await tester.pump();
    await tester.pump();
    expect(platform.createdCameraIds, [1, 2]);
    final record = tester.widget<InkWell>(
      find.byKey(const ValueKey('animated-avatar-record')),
    );
    expect(record.onTap, isNotNull);

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('shared disposal serializes camera-mode switches', (
    tester,
  ) async {
    for (final startsAnimated in [false, true]) {
      final platform = _TestCameraPlatform(blockDisposeForCameraIds: {1});
      final previousPlatform = CameraPlatform.instance;
      CameraPlatform.instance = platform;
      final barrier = CameraDisposalBarrier();
      var animated = startsAnimated;
      late StateSetter setMode;

      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: StatefulBuilder(
                builder: (context, setState) {
                  setMode = setState;
                  return SizedBox(
                    height: 600,
                    child: animated
                        ? AnimatedAvatarCapture(
                            height: 600,
                            onPrepareChanged: (_) {},
                            disposalBarrier: barrier,
                          )
                        : ImageAvatarCapture(
                            height: 600,
                            onAccepted: (_) {},
                            onClosed: () {},
                            loadCameras: platform.availableCameras,
                            disposalBarrier: barrier,
                          ),
                  );
                },
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      await tester.pump();
      expect(platform.createdCameraIds, [1]);

      setMode(() => animated = !animated);
      await tester.pump();
      await tester.pump();
      expect(platform.disposedCameraIds, [1]);
      expect(platform.createdCameraIds, [1]);

      platform.completeDispose(1);
      await tester.pump();
      await tester.pump();
      expect(platform.createdCameraIds, [1, 2]);

      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      CameraPlatform.instance = previousPlatform;
    }
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

class _TestCameraPlatform extends CameraPlatform {
  _TestCameraPlatform({
    Set<int>? failLockForCameraIds,
    Set<int>? blockDisposeForCameraIds,
    Set<int>? blockInitializeForCameraIds,
  }) : _failLockForCameraIds = failLockForCameraIds ?? const {},
       _blockDisposeForCameraIds = blockDisposeForCameraIds ?? const {},
       _blockInitializeForCameraIds = blockInitializeForCameraIds ?? const {};

  final Set<int> _failLockForCameraIds;
  final Set<int> _blockDisposeForCameraIds;
  final Set<int> _blockInitializeForCameraIds;
  final _disposeCompleters = <int, Completer<void>>{};
  final _initializeCompleters = <int, Completer<void>>{};
  final _initializedControllers =
      <int, StreamController<CameraInitializedEvent>>{};
  final _errorControllers = <int, StreamController<CameraErrorEvent>>{};
  final _orientationController =
      StreamController<DeviceOrientationChangedEvent>.broadcast();
  final createdCameraIds = <int>[];
  final disposedCameraIds = <int>[];
  var _nextCameraId = 1;

  @override
  Future<List<CameraDescription>> availableCameras() async => const [
    CameraDescription(
      name: 'front',
      lensDirection: CameraLensDirection.front,
      sensorOrientation: 0,
    ),
  ];

  @override
  Future<int> createCameraWithSettings(
    CameraDescription description,
    MediaSettings mediaSettings,
  ) async {
    final cameraId = _nextCameraId++;
    createdCameraIds.add(cameraId);
    _initializedControllers[cameraId] =
        StreamController<CameraInitializedEvent>.broadcast();
    _errorControllers[cameraId] =
        StreamController<CameraErrorEvent>.broadcast();
    return cameraId;
  }

  @override
  Stream<CameraInitializedEvent> onCameraInitialized(int cameraId) =>
      _initializedControllers[cameraId]!.stream.asBroadcastStream();

  @override
  Stream<CameraErrorEvent> onCameraError(int cameraId) =>
      _errorControllers[cameraId]!.stream.asBroadcastStream();

  @override
  Stream<DeviceOrientationChangedEvent> onDeviceOrientationChanged() =>
      _orientationController.stream;

  @override
  Future<void> initializeCamera(
    int cameraId, {
    ImageFormatGroup imageFormatGroup = ImageFormatGroup.unknown,
  }) async {
    if (_blockInitializeForCameraIds.contains(cameraId)) {
      await (_initializeCompleters[cameraId] ??= Completer<void>()).future;
    }
    _initializedControllers[cameraId]!.add(
      CameraInitializedEvent(
        cameraId,
        640,
        480,
        ExposureMode.auto,
        false,
        FocusMode.auto,
        false,
      ),
    );
  }

  @override
  Future<void> lockCaptureOrientation(
    int cameraId,
    DeviceOrientation orientation,
  ) async {
    if (_failLockForCameraIds.contains(cameraId)) {
      throw PlatformException(code: 'orientation-failed');
    }
  }

  @override
  Widget buildPreview(int cameraId) => const SizedBox.expand();

  @override
  Future<void> dispose(int cameraId) async {
    disposedCameraIds.add(cameraId);
    if (_blockDisposeForCameraIds.contains(cameraId)) {
      await (_disposeCompleters[cameraId] ??= Completer<void>()).future;
    }
  }

  void completeInitialize(int cameraId) {
    _initializeCompleters[cameraId]!.complete();
  }

  void failDispose(int cameraId) {
    _disposeCompleters[cameraId]!.completeError(
      PlatformException(code: 'dispose-failed'),
    );
  }

  void completeDispose(int cameraId) {
    _disposeCompleters[cameraId]!.complete();
  }
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
