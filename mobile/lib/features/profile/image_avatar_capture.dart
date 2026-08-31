import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:camera/camera.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image/image.dart' as image;
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'camera_disposal_barrier.dart';

part 'image_avatar_capture/camera_preview.dart';
part 'image_avatar_capture/morphing_camera_action.dart';
part 'image_avatar_capture/shutter_button.dart';

const _avatarPreviewSize = 220.0;

/// Diameter of the expanded circular viewfinder while taking a profile photo.
const imageAvatarCameraPreviewSize = _avatarPreviewSize * 1.25;
const _cameraControlSize = 64.0;
const _cameraControlRailHeight = 115.0;
const _shutterSize = _cameraControlSize * 1.5625;
const _shutterCoreSize = _shutterSize * (99 / 115);
const _reviewControlWidth = 112.0;
const _expandedControlOffset = 119.5;
const _reviewControlGap = Grid.twelve;
const _captureMotionDuration = Duration(milliseconds: 180);
const _shutterExitDuration = Duration(milliseconds: 150);
const _cameraFlipHalfDuration = Duration(milliseconds: 100);

/// Builds the inline still-photo camera used by the profile avatar editor.
typedef ImageAvatarCaptureBuilder =
    Widget Function({
      required double height,
      required ValueChanged<Uint8List> onAccepted,
      required VoidCallback onClosed,
    });

/// Captures a still profile photo inside the avatar's circular viewfinder.
class ImageAvatarCapture extends HookConsumerWidget {
  /// Creates the inline image capture surface.
  const ImageAvatarCapture({
    super.key,
    required this.height,
    required this.onAccepted,
    required this.onClosed,
    this.initialPreview,
    this.initialCapturedBytes,
    this.loadCameras = availableCameras,
    this.disposalBarrier,
  });

  /// The vertical space available to the camera and its controls.
  final double height;

  /// Accepts the captured, square image as an unsaved avatar draft.
  final ValueChanged<Uint8List> onAccepted;

  /// Leaves camera mode without changing the current avatar draft.
  final VoidCallback onClosed;

  /// The existing avatar shown while the same circular cutout becomes a camera.
  final Widget? initialPreview;

  /// Seeds the captured-photo review state in focused widget tests.
  @visibleForTesting
  final Uint8List? initialCapturedBytes;

  /// Loads device cameras. Overridden by focused widget tests.
  @visibleForTesting
  final Future<List<CameraDescription>> Function() loadCameras;

  /// Serializes ownership release with another profile capture surface.
  final CameraDisposalBarrier? disposalBarrier;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final lifecycle = ref.watch(appLifecycleProvider);
    final controller = useState<CameraController?>(null);
    final controllerRef = useRef<CameraController?>(null);
    final controllerDisposal = useRef(
      disposalBarrier ?? CameraDisposalBarrier(),
    );
    final candidateRef = useRef<CameraController?>(null);
    final cameras = useState<List<CameraDescription>>(const []);
    final selectedLens = useState(CameraLensDirection.front);
    final cameraGeneration = useState(0);
    final flipAnimation = useAnimationController(
      duration: _cameraFlipHalfDuration * 2,
    );
    final flipDirection = useState(1.0);
    final isInitializing = useState(initialCapturedBytes == null);
    final isFlipping = useState(false);
    final isCapturing = useState(false);
    final isProcessingCapture = useState(false);
    final capturedBytes = useState<Uint8List?>(initialCapturedBytes);
    final controlsExpanded = useState(false);
    final isClosing = useState(false);
    final error = useState<String?>(null);

    Future<void> releaseController(CameraController? active) {
      if (active == null) return controllerDisposal.value.settled;
      return controllerDisposal.value.release(active.dispose);
    }

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        controlsExpanded.value = true;
      });
      return null;
    }, const []);

    useEffect(
      () => () {
        final active = controllerRef.value;
        controllerRef.value = null;
        unawaited(releaseController(active));
      },
      const [],
    );

    useEffect(() {
      var disposed = false;
      final generation = cameraGeneration.value;

      if (lifecycle != AppLifecycleState.resumed ||
          capturedBytes.value != null) {
        isInitializing.value = false;
        final active = controllerRef.value;
        controllerRef.value = null;
        controller.value = null;
        unawaited(releaseController(active));
        return null;
      }

      isInitializing.value = true;
      error.value = null;

      Future<void> initialize() async {
        CameraController? next;
        CameraDisposalReservation? reservation;
        var installed = false;
        try {
          await controllerDisposal.value.settled;
          if (disposed || generation != cameraGeneration.value) return;
          final available = await loadCameras();
          if (disposed || generation != cameraGeneration.value) return;
          cameras.value = available;
          if (available.isEmpty) {
            throw CameraException(
              'no-cameras',
              'No cameras are available on this device.',
            );
          }
          final description = available.firstWhere(
            (candidate) => candidate.lensDirection == selectedLens.value,
            orElse: () => available.first,
          );
          selectedLens.value = description.lensDirection;
          next = CameraController(
            description,
            ResolutionPreset.high,
            enableAudio: false,
          );
          reservation = controllerDisposal.value.reserve();
          candidateRef.value = next;
          await reservation.ready;
          if (disposed) {
            if (identical(candidateRef.value, next)) candidateRef.value = null;
            await reservation.dispose(next.dispose);
            return;
          }
          await next.initialize();
          await next.lockCaptureOrientation(DeviceOrientation.portraitUp);
          if (disposed || generation != cameraGeneration.value) {
            if (identical(candidateRef.value, next)) candidateRef.value = null;
            await reservation.dispose(next.dispose);
            return;
          }
          candidateRef.value = null;
          reservation.complete();
          final previous = controllerRef.value;
          controllerRef.value = next;
          controller.value = next;
          installed = true;
          if (previous != null && previous != next) {
            unawaited(releaseController(previous));
          }
        } catch (_) {
          if (!installed && next != null) {
            final activeCandidate = identical(candidateRef.value, next);
            if (activeCandidate) candidateRef.value = null;
            if (activeCandidate) {
              await reservation?.dispose(next.dispose);
            }
          }
          if (!disposed && generation == cameraGeneration.value) {
            final active = controllerRef.value;
            if (active != null) {
              selectedLens.value = active.description.lensDirection;
            }
            error.value = 'Could not access the camera.';
          }
        } finally {
          if (!disposed && generation == cameraGeneration.value) {
            isInitializing.value = false;
          }
        }
      }

      unawaited(initialize());
      return () {
        disposed = true;
      };
    }, [lifecycle, capturedBytes.value == null, cameraGeneration.value]);

    Future<void> capture() async {
      final active = controller.value;
      if (active == null || isCapturing.value || active.value.isTakingPicture) {
        return;
      }
      isCapturing.value = true;
      error.value = null;
      XFile? photo;
      try {
        unawaited(HapticFeedback.mediumImpact());
        photo = await active.takePicture();
        try {
          await active.pausePreview();
        } on CameraException {
          // Some camera backends pause automatically after a still capture.
        }
        if (context.mounted) isProcessingCapture.value = true;
        final prepared = await ref
            .read(mediaUploadServiceProvider)
            .prepareImageBytes(photo);
        final cropped = await compute(_centerCropCameraImage, (
          bytes: prepared,
          mirror: active.description.lensDirection == CameraLensDirection.front,
        ));
        if (context.mounted) capturedBytes.value = cropped;
      } catch (_) {
        if (context.mounted) {
          error.value = "We couldn't take that photo. Try again.";
          try {
            await active.resumePreview();
          } on CameraException {
            if (identical(controllerRef.value, active)) {
              controllerRef.value = null;
              controller.value = null;
              await releaseController(active);
              if (context.mounted) cameraGeneration.value++;
            }
          }
        }
      } finally {
        final path = photo?.path;
        if (path != null && path.isNotEmpty) {
          try {
            await File(path).delete();
          } on FileSystemException {
            // The camera plugin can remove its temporary file independently.
          }
        }
        if (context.mounted) {
          isCapturing.value = false;
          isProcessingCapture.value = false;
        }
      }
    }

    Future<void> flipCamera() async {
      if (isInitializing.value ||
          isFlipping.value ||
          isCapturing.value ||
          cameras.value.length < 2) {
        return;
      }
      final active = controller.value;
      if (active == null) return;
      final nextLens = selectedLens.value == CameraLensDirection.front
          ? CameraLensDirection.back
          : CameraLensDirection.front;
      final matches = cameras.value.where(
        (camera) => camera.lensDirection == nextLens,
      );
      if (matches.isEmpty) return;
      unawaited(HapticFeedback.selectionClick());
      isFlipping.value = true;
      flipDirection.value = nextLens == CameraLensDirection.back ? 1 : -1;
      error.value = null;
      final flipMotion = reduceMotion
          ? null
          : flipAnimation.animateTo(
              1,
              duration: _cameraFlipHalfDuration * 2,
              curve: Curves.easeInOutCubic,
            );
      try {
        if (!reduceMotion) await Future<void>.delayed(_cameraFlipHalfDuration);
        if (!context.mounted) return;
        await active.setDescription(matches.first);
        if (context.mounted) selectedLens.value = nextLens;
        await active.lockCaptureOrientation(DeviceOrientation.portraitUp);
      } on CameraException {
        if (context.mounted) error.value = 'Could not switch cameras.';
      } finally {
        if (context.mounted && flipMotion != null) {
          try {
            await flipMotion.orCancel;
          } on TickerCanceled {
            // The view was disposed while the camera was switching.
          }
        }
        if (context.mounted) {
          flipAnimation.value = 0;
          isFlipping.value = false;
        }
      }
    }

    void retake() {
      unawaited(HapticFeedback.selectionClick());
      capturedBytes.value = null;
      error.value = null;
      cameraGeneration.value++;
    }

    Future<void> leaveCamera(Uint8List? acceptedBytes) async {
      if (isClosing.value) return;
      unawaited(
        acceptedBytes == null
            ? HapticFeedback.selectionClick()
            : HapticFeedback.mediumImpact(),
      );
      isClosing.value = true;
      controlsExpanded.value = false;
      if (!reduceMotion) await Future<void>.delayed(_captureMotionDuration);
      if (!context.mounted) return;
      if (acceptedBytes == null) {
        onClosed();
      } else {
        onAccepted(acceptedBytes);
      }
    }

    final captured = capturedBytes.value;
    final previewSize = controlsExpanded.value
        ? imageAvatarCameraPreviewSize
        : _avatarPreviewSize;
    final captureEnabled =
        controller.value != null &&
        !isInitializing.value &&
        !isFlipping.value &&
        !isCapturing.value &&
        !isClosing.value;
    final hasOppositeLens = cameras.value.any(
      (camera) =>
          camera.lensDirection ==
          (selectedLens.value == CameraLensDirection.front
              ? CameraLensDirection.back
              : CameraLensDirection.front),
    );
    final flipEnabled =
        hasOppositeLens &&
        !isInitializing.value &&
        !isFlipping.value &&
        !isCapturing.value &&
        !isClosing.value;

    return SizedBox(
      key: const ValueKey('image-avatar-camera'),
      height: height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned(
            left: 0,
            right: 0,
            top: 0,
            height: imageAvatarCameraPreviewSize,
            child: Center(
              child: AnimatedBuilder(
                animation: flipAnimation,
                builder: (context, child) {
                  final progress = flipAnimation.value;
                  final angle = progress <= 0.5
                      ? progress * pi
                      : (progress - 1) * pi;
                  return Transform(
                    key: const ValueKey('image-camera-preview-flip'),
                    alignment: Alignment.center,
                    transform: Matrix4.identity()
                      ..setEntry(3, 2, 0.0015)
                      ..rotateY(angle * flipDirection.value),
                    child: child,
                  );
                },
                child: AnimatedContainer(
                  key: const ValueKey('image-camera-preview-size'),
                  duration: reduceMotion
                      ? Duration.zero
                      : _captureMotionDuration,
                  curve: Curves.easeOutCubic,
                  width: previewSize,
                  height: previewSize,
                  child: ClipOval(
                    child: ColoredBox(
                      color: Colors.black,
                      child: captured != null
                          ? Image.memory(captured, fit: BoxFit.cover)
                          : controller.value != null
                          ? _CameraPreview(controller: controller.value!)
                          : initialPreview != null
                          ? FittedBox(
                              key: const ValueKey(
                                'image-camera-initial-preview-scale',
                              ),
                              fit: BoxFit.cover,
                              child: SizedBox.square(
                                dimension: _avatarPreviewSize,
                                child: initialPreview,
                              ),
                            )
                          : Center(
                              child: isInitializing.value
                                  ? const BuzzLoadingIndicator(
                                      semanticLabel: 'Starting camera',
                                    )
                                  : const Icon(LucideIcons.cameraOff, size: 32),
                            ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          Positioned(
            left: 0,
            right: 0,
            bottom: 0,
            height: _cameraControlRailHeight,
            child: TweenAnimationBuilder<double>(
              tween: Tween(end: controlsExpanded.value ? 1 : 0),
              duration: reduceMotion ? Duration.zero : _captureMotionDuration,
              curve: Curves.easeOutCubic,
              builder: (context, progress, _) => LayoutBuilder(
                builder: (context, constraints) {
                  final collapsedControlOffset =
                      (constraints.maxWidth + Grid.half * 3) / 8;
                  final sideOffset =
                      collapsedControlOffset +
                      (_expandedControlOffset - collapsedControlOffset) *
                          progress;
                  return TweenAnimationBuilder<double>(
                    tween: Tween(
                      end: captured == null || isClosing.value ? 0 : 1,
                    ),
                    duration: reduceMotion
                        ? Duration.zero
                        : _captureMotionDuration,
                    curve: Curves.easeInOutCubic,
                    builder: (context, reviewProgress, _) {
                      final sideWidth =
                          _cameraControlSize +
                          (_reviewControlWidth - _cameraControlSize) *
                              reviewProgress;
                      final reviewSideOffset =
                          _reviewControlWidth / 2 + _reviewControlGap / 2;
                      final effectiveSideOffset =
                          sideOffset +
                          (reviewSideOffset - sideOffset) * reviewProgress;
                      return Stack(
                        alignment: Alignment.center,
                        children: [
                          Positioned(
                            left:
                                constraints.maxWidth / 2 -
                                effectiveSideOffset -
                                sideWidth / 2,
                            width: sideWidth,
                            height: _cameraControlRailHeight,
                            child: _MorphingCameraAction(
                              controlKey: const ValueKey(
                                'image-camera-left-action',
                              ),
                              width: sideWidth,
                              icon: isClosing.value
                                  ? LucideIcons.camera
                                  : LucideIcons.x,
                              iosIcon: isClosing.value
                                  ? IosGlassNavigationIcon.camera
                                  : IosGlassNavigationIcon.close,
                              label: captured != null && !isClosing.value
                                  ? 'Retry'
                                  : null,
                              transitionLabel: isClosing.value
                                  ? 'Camera'
                                  : null,
                              transitionLabelMaxWidth: 96,
                              transitionProgress: isClosing.value
                                  ? 1 - progress
                                  : 0,
                              showEnabledAppearance: isClosing.value,
                              semanticLabel: isClosing.value
                                  ? 'Camera'
                                  : captured == null
                                  ? 'Close camera'
                                  : 'Retry',
                              onTap:
                                  isFlipping.value ||
                                      isCapturing.value ||
                                      isClosing.value
                                  ? null
                                  : captured == null
                                  ? () => unawaited(leaveCamera(null))
                                  : retake,
                            ),
                          ),
                          TweenAnimationBuilder<double>(
                            tween: Tween(end: controlsExpanded.value ? 1 : 0),
                            duration: reduceMotion
                                ? Duration.zero
                                : isClosing.value
                                ? _shutterExitDuration
                                : _captureMotionDuration,
                            curve: Curves.easeOutCubic,
                            builder: (context, shutterProgress, _) =>
                                Transform.scale(
                                  scale:
                                      (0.73 + 0.27 * shutterProgress) *
                                      (1 - 0.28 * reviewProgress),
                                  child: Opacity(
                                    key: const ValueKey(
                                      'image-camera-shutter-exit-opacity',
                                    ),
                                    opacity:
                                        shutterProgress * (1 - reviewProgress),
                                    child: IgnorePointer(
                                      ignoring: captured != null,
                                      child: _ShutterButton(
                                        busy: isProcessingCapture.value,
                                        onTap: captureEnabled
                                            ? () => unawaited(capture())
                                            : null,
                                      ),
                                    ),
                                  ),
                                ),
                          ),
                          Positioned(
                            left:
                                constraints.maxWidth / 2 +
                                effectiveSideOffset -
                                sideWidth / 2,
                            width: sideWidth,
                            height: _cameraControlRailHeight,
                            child: _MorphingCameraAction(
                              controlKey: const ValueKey(
                                'image-camera-right-action',
                              ),
                              width: sideWidth,
                              icon: isClosing.value
                                  ? LucideIcons.images
                                  : LucideIcons.switchCamera,
                              iosIcon: isClosing.value
                                  ? IosGlassNavigationIcon.photoLibrary
                                  : IosGlassNavigationIcon.rotateCamera,
                              label: captured != null && !isClosing.value
                                  ? 'Use Photo'
                                  : null,
                              transitionLabel: isClosing.value
                                  ? 'Photo Library'
                                  : null,
                              transitionLabelMaxWidth: 104,
                              transitionProgress: isClosing.value
                                  ? 1 - progress
                                  : 0,
                              showEnabledAppearance: isClosing.value,
                              semanticLabel: isClosing.value
                                  ? 'Photo Library'
                                  : captured == null
                                  ? 'Flip camera'
                                  : 'Use Photo',
                              onTap: isClosing.value
                                  ? null
                                  : captured != null
                                  ? () => unawaited(leaveCamera(captured))
                                  : flipEnabled
                                  ? () => unawaited(flipCamera())
                                  : null,
                            ),
                          ),
                        ],
                      );
                    },
                  );
                },
              ),
            ),
          ),
          if (error.value != null)
            Positioned(
              left: 0,
              right: 0,
              bottom: _cameraControlRailHeight + Grid.xs,
              child: Semantics(
                liveRegion: true,
                child: Text(
                  error.value!,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

Uint8List _centerCropCameraImage(({Uint8List bytes, bool mirror}) request) {
  var decoded = image.decodeImage(request.bytes);
  if (decoded == null) throw const FormatException('Invalid camera image');
  if (request.mirror) decoded = image.flipHorizontal(decoded);
  final side = min(decoded.width, decoded.height);
  final cropped = image.copyCrop(
    decoded,
    x: (decoded.width - side) ~/ 2,
    y: (decoded.height - side) ~/ 2,
    width: side,
    height: side,
  );
  final resized = side == 512
      ? cropped
      : image.copyResize(
          cropped,
          width: 512,
          height: 512,
          interpolation: image.Interpolation.cubic,
        );
  return Uint8List.fromList(image.encodeJpg(resized, quality: 92));
}

/// Prepares a camera image with the same mirroring and crop used by capture.
@visibleForTesting
Uint8List prepareCameraImageForTesting(
  Uint8List bytes, {
  required bool mirror,
}) => _centerCropCameraImage((bytes: bytes, mirror: mirror));
