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

const _avatarPreviewSize = 220.0;

/// Diameter of the expanded circular viewfinder while taking a profile photo.
const imageAvatarCameraPreviewSize = 245.0;
const _cameraControlSize = 48.0;
const _shutterSize = 88.0;
const _acceptedControlSize = 64.0;
const _captureMotionDuration = Duration(milliseconds: 180);

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
    this.initialCapturedBytes,
    this.loadCameras = availableCameras,
  });

  /// The vertical space available to the camera and its controls.
  final double height;

  /// Accepts the captured, square image as an unsaved avatar draft.
  final ValueChanged<Uint8List> onAccepted;

  /// Leaves camera mode without changing the current avatar draft.
  final VoidCallback onClosed;

  /// Seeds the captured-photo review state in focused widget tests.
  @visibleForTesting
  final Uint8List? initialCapturedBytes;

  /// Loads device cameras. Overridden by focused widget tests.
  @visibleForTesting
  final Future<List<CameraDescription>> Function() loadCameras;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final lifecycle = ref.watch(appLifecycleProvider);
    final controller = useState<CameraController?>(null);
    final controllerRef = useRef<CameraController?>(null);
    final cameras = useState<List<CameraDescription>>(const []);
    final selectedLens = useState(CameraLensDirection.front);
    final cameraGeneration = useState(0);
    final isInitializing = useState(initialCapturedBytes == null);
    final isCapturing = useState(false);
    final capturedBytes = useState<Uint8List?>(initialCapturedBytes);
    final controlsExpanded = useState(false);
    final error = useState<String?>(null);

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) controlsExpanded.value = true;
      });
      return null;
    }, const []);

    useEffect(() {
      var disposed = false;
      final generation = cameraGeneration.value;

      if (lifecycle != AppLifecycleState.resumed ||
          capturedBytes.value != null) {
        isInitializing.value = false;
        controller.value = null;
        return null;
      }

      isInitializing.value = true;
      controller.value = null;
      error.value = null;

      Future<void> initialize() async {
        CameraController? next;
        try {
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
          await next.initialize();
          if (disposed || generation != cameraGeneration.value) {
            await next.dispose();
            return;
          }
          controllerRef.value = next;
          controller.value = next;
        } catch (_) {
          await next?.dispose();
          if (!disposed && generation == cameraGeneration.value) {
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
        final active = controllerRef.value;
        controllerRef.value = null;
        unawaited(active?.dispose() ?? Future<void>.value());
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
        final prepared = await ref
            .read(mediaUploadServiceProvider)
            .prepareImageBytes(photo);
        final cropped = await compute(_centerCropCameraImage, prepared);
        if (context.mounted) capturedBytes.value = cropped;
      } catch (_) {
        if (context.mounted) {
          error.value = "We couldn't take that photo. Try again.";
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
        if (context.mounted) isCapturing.value = false;
      }
    }

    void flipCamera() {
      if (isInitializing.value ||
          isCapturing.value ||
          cameras.value.length < 2) {
        return;
      }
      final nextLens = selectedLens.value == CameraLensDirection.front
          ? CameraLensDirection.back
          : CameraLensDirection.front;
      if (!cameras.value.any((camera) => camera.lensDirection == nextLens)) {
        return;
      }
      unawaited(HapticFeedback.selectionClick());
      selectedLens.value = nextLens;
      cameraGeneration.value++;
    }

    void retake() {
      unawaited(HapticFeedback.selectionClick());
      capturedBytes.value = null;
      error.value = null;
      cameraGeneration.value++;
    }

    final captured = capturedBytes.value;
    final previewSize = captured == null && controlsExpanded.value
        ? imageAvatarCameraPreviewSize
        : _avatarPreviewSize;
    final captureEnabled = controller.value != null && !isCapturing.value;
    final flipEnabled = cameras.value.length > 1 && !isCapturing.value;

    return SizedBox(
      key: const ValueKey('image-avatar-camera'),
      height: height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Align(
            alignment: Alignment.topCenter,
            child: AnimatedContainer(
              key: const ValueKey('image-camera-preview-size'),
              duration: reduceMotion ? Duration.zero : _captureMotionDuration,
              curve: Curves.easeInOutCubic,
              width: previewSize,
              height: previewSize,
              child: ClipOval(
                child: ColoredBox(
                  color: Colors.black,
                  child: captured != null
                      ? Image.memory(captured, fit: BoxFit.cover)
                      : controller.value != null
                      ? _CameraPreview(controller: controller.value!)
                      : Center(
                          child: isInitializing.value
                              ? const BuzzLoadingIndicator(
                                  color: Colors.white,
                                  semanticLabel: 'Starting camera',
                                )
                              : const Icon(
                                  LucideIcons.cameraOff,
                                  color: Colors.white,
                                  size: 32,
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
            height: _shutterSize,
            child: TweenAnimationBuilder<double>(
              tween: Tween(end: controlsExpanded.value ? 1 : 0),
              duration: reduceMotion ? Duration.zero : _captureMotionDuration,
              curve: Curves.easeOutCubic,
              builder: (context, progress, _) => Stack(
                alignment: Alignment.center,
                children: [
                  Transform.translate(
                    offset: Offset(-52 - 60 * progress, 0),
                    child: _CameraIconButton(
                      key: const ValueKey('image-camera-close'),
                      icon: LucideIcons.x,
                      semanticLabel: 'Close camera',
                      onTap: isCapturing.value ? null : onClosed,
                    ),
                  ),
                  Transform.scale(
                    scale: 0.73 + 0.27 * progress,
                    child: Opacity(
                      opacity: progress,
                      child: _ShutterButton(
                        captured: captured != null,
                        busy: isCapturing.value,
                        onTap: captured != null
                            ? () {
                                unawaited(HapticFeedback.lightImpact());
                                onAccepted(captured);
                              }
                            : captureEnabled
                            ? () => unawaited(capture())
                            : null,
                      ),
                    ),
                  ),
                  Transform.translate(
                    offset: Offset(52 + 60 * progress, 0),
                    child: _CameraIconButton(
                      key: ValueKey(
                        captured == null
                            ? 'image-camera-flip'
                            : 'image-camera-retake',
                      ),
                      icon: LucideIcons.refreshCcw,
                      semanticLabel: captured == null
                          ? 'Flip camera'
                          : 'Retake photo',
                      onTap: captured != null
                          ? retake
                          : flipEnabled
                          ? flipCamera
                          : null,
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (error.value != null)
            Positioned(
              left: 0,
              right: 0,
              bottom: _shutterSize + Grid.xs,
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

class _CameraPreview extends StatelessWidget {
  const _CameraPreview({required this.controller});

  final CameraController controller;

  @override
  Widget build(BuildContext context) {
    final orientation = controller.value.deviceOrientation;
    final landscape =
        orientation == DeviceOrientation.landscapeLeft ||
        orientation == DeviceOrientation.landscapeRight;
    final aspectRatio = landscape
        ? controller.value.aspectRatio
        : 1 / controller.value.aspectRatio;
    return FittedBox(
      fit: BoxFit.cover,
      clipBehavior: Clip.hardEdge,
      child: SizedBox(
        width: imageAvatarCameraPreviewSize * aspectRatio,
        height: imageAvatarCameraPreviewSize,
        child: CameraPreview(controller),
      ),
    );
  }
}

class _CameraIconButton extends StatelessWidget {
  const _CameraIconButton({
    super.key,
    required this.icon,
    required this.semanticLabel,
    required this.onTap,
  });

  final IconData icon;
  final String semanticLabel;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    label: semanticLabel,
    button: true,
    enabled: onTap != null,
    child: ExcludeSemantics(
      child: Material(
        color: context.colors.surfaceContainerHighest,
        shape: const CircleBorder(),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: SizedBox.square(
            dimension: _cameraControlSize,
            child: Icon(
              icon,
              size: 22,
              color: onTap == null
                  ? context.colors.onSurface.withValues(alpha: 0.38)
                  : context.colors.onSurface,
            ),
          ),
        ),
      ),
    ),
  );
}

class _ShutterButton extends StatelessWidget {
  const _ShutterButton({
    required this.captured,
    required this.busy,
    required this.onTap,
  });

  final bool captured;
  final bool busy;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    return Semantics(
      label: captured ? 'Use photo' : 'Take photo',
      button: true,
      enabled: onTap != null,
      child: ExcludeSemantics(
        child: AnimatedContainer(
          key: const ValueKey('image-camera-shutter-morph'),
          duration: reduceMotion ? Duration.zero : _captureMotionDuration,
          curve: Curves.easeOutCubic,
          width: captured ? _acceptedControlSize : _shutterSize,
          height: captured ? _acceptedControlSize : _shutterSize,
          child: Material(
            color: captured
                ? context.colors.onSurface
                : context.colors.surfaceContainerHighest,
            shape: const CircleBorder(),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              key: const ValueKey('image-camera-shutter'),
              onTap: onTap,
              child: Center(
                child: AnimatedSwitcher(
                  duration: reduceMotion
                      ? Duration.zero
                      : const Duration(milliseconds: 140),
                  switchInCurve: Curves.easeOutCubic,
                  switchOutCurve: Curves.easeOutCubic,
                  child: busy
                      ? BuzzLoadingIndicator(
                          key: const ValueKey('image-camera-capturing'),
                          size: 24,
                          color: context.colors.onSurface,
                          semanticLabel: 'Taking photo',
                        )
                      : captured
                      ? Icon(
                          LucideIcons.check,
                          key: const ValueKey('image-camera-accept-icon'),
                          size: 28,
                          color: context.colors.surface,
                        )
                      : Container(
                          key: const ValueKey('image-camera-shutter-icon'),
                          width: 64,
                          height: 64,
                          decoration: BoxDecoration(
                            color: context.colors.onSurface,
                            shape: BoxShape.circle,
                            border: Border.all(
                              color: context.colors.surface,
                              width: 3,
                            ),
                          ),
                        ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

Uint8List _centerCropCameraImage(Uint8List bytes) {
  final decoded = image.decodeImage(bytes);
  if (decoded == null) throw const FormatException('Invalid camera image');
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
