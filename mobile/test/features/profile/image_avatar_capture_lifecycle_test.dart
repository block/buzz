import 'dart:async';

import 'package:buzz/features/profile/image_avatar_capture.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:camera_platform_interface/camera_platform_interface.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('resumes with a replacement after native disposal fails', (
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
              height: 400,
              child: ImageAvatarCapture(
                height: 400,
                onAccepted: (_) {},
                onClosed: () {},
                loadCameras: platform.availableCameras,
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
    final shutter = tester.widget<InkWell>(
      find.byKey(const ValueKey('image-camera-shutter')),
    );
    expect(shutter.onTap, isNotNull);

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
              height: 400,
              child: ImageAvatarCapture(
                height: 400,
                onAccepted: (_) {},
                onClosed: () {},
                loadCameras: platform.availableCameras,
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
    final shutter = tester.widget<InkWell>(
      find.byKey(const ValueKey('image-camera-shutter')),
    );
    expect(shutter.onTap, isNotNull);

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
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

class _TestCameraPlatform extends CameraPlatform {
  _TestCameraPlatform({
    Set<int>? blockDisposeForCameraIds,
    Set<int>? blockInitializeForCameraIds,
  }) : _blockDisposeForCameraIds = blockDisposeForCameraIds ?? const {},
       _blockInitializeForCameraIds = blockInitializeForCameraIds ?? const {};

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
  ) async {}

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
