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

const _avatarPreviewSize = 220.0;

/// Diameter of the expanded circular viewfinder while taking a profile photo.
const imageAvatarCameraPreviewSize = _avatarPreviewSize * 1.25;
const _cameraControlSize = 64.0;
const _shutterSize = 115.0;
const _shutterCoreSize = _shutterSize - Grid.xxs * 2;
const _reviewControlWidth = 112.0;
const _expandedControlOffset = 119.5;
const _reviewControlGap = Grid.twelve;
const _captureMotionDuration = Duration(milliseconds: 180);
const _shutterExitDuration = Duration(milliseconds: 150);

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
    final isFlipping = useState(false);
    final isCapturing = useState(false);
    final isProcessingCapture = useState(false);
    final capturedBytes = useState<Uint8List?>(initialCapturedBytes);
    final controlsExpanded = useState(false);
    final isClosing = useState(false);
    final error = useState<String?>(null);

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
        unawaited(active?.dispose() ?? Future<void>.value());
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
        unawaited(active?.dispose() ?? Future<void>.value());
        return null;
      }

      isInitializing.value = true;
      error.value = null;

      Future<void> initialize() async {
        CameraController? next;
        var installed = false;
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
          final previous = controllerRef.value;
          controllerRef.value = next;
          controller.value = next;
          installed = true;
          if (previous != null && previous != next) {
            unawaited(previous.dispose());
          }
        } catch (_) {
          if (!installed) await next?.dispose();
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
            // Reinitialization remains available if this backend cannot resume.
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
      error.value = null;
      try {
        await active.setDescription(matches.first);
        if (context.mounted) selectedLens.value = nextLens;
      } on CameraException {
        if (context.mounted) error.value = 'Could not switch cameras.';
      } finally {
        if (context.mounted) isFlipping.value = false;
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
        ? controller.value != null || captured != null
              ? imageAvatarCameraPreviewSize
              : _avatarPreviewSize
        : _avatarPreviewSize;
    final captureEnabled =
        controller.value != null &&
        !isInitializing.value &&
        !isFlipping.value &&
        !isCapturing.value &&
        !isClosing.value;
    final flipEnabled =
        cameras.value.length > 1 &&
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
                        : initialPreview ??
                              Center(
                                child: isInitializing.value
                                    ? const BuzzLoadingIndicator(
                                        semanticLabel: 'Starting camera',
                                      )
                                    : const Icon(
                                        LucideIcons.cameraOff,
                                        size: 32,
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
            height: _shutterSize,
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
                            height: _shutterSize,
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
                            height: _shutterSize,
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

class _MorphingCameraAction extends StatelessWidget {
  const _MorphingCameraAction({
    required this.controlKey,
    required this.width,
    required this.icon,
    required this.iosIcon,
    required this.label,
    required this.transitionLabel,
    required this.transitionLabelMaxWidth,
    required this.transitionProgress,
    required this.showEnabledAppearance,
    required this.semanticLabel,
    required this.onTap,
  });

  final Key controlKey;
  final double width;
  final IconData icon;
  final IosGlassNavigationIcon iosIcon;
  final String? label;
  final String? transitionLabel;
  final double transitionLabelMaxWidth;
  final double transitionProgress;
  final bool showEnabledAppearance;
  final String semanticLabel;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    Widget control;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      control = IgnorePointer(
        ignoring: onTap == null,
        child: IosGlassNavigationButton(
          icon: iosIcon,
          label: label,
          semanticLabel: semanticLabel,
          onPressed: onTap ?? (showEnabledAppearance ? () {} : null),
          width: width,
          height: _cameraControlSize,
          controlSize: _cameraControlSize,
          fillWidth: true,
          foregroundColor: context.colors.onSurface,
        ),
      );
    } else {
      final dimmed = onTap == null && !showEnabledAppearance;
      control = Semantics(
        label: semanticLabel,
        button: true,
        enabled: onTap != null,
        child: ExcludeSemantics(
          child: Material(
            color: context.colors.surfaceContainerHighest,
            shape: const StadiumBorder(),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: onTap,
              child: SizedBox(
                width: width,
                height: _cameraControlSize,
                child: Center(
                  child: AnimatedSwitcher(
                    duration: MediaQuery.disableAnimationsOf(context)
                        ? Duration.zero
                        : const Duration(milliseconds: 120),
                    switchInCurve: Curves.easeOutCubic,
                    switchOutCurve: Curves.easeOutCubic,
                    child: label == null
                        ? Icon(
                            icon,
                            key: ValueKey('camera-action-icon-${iosIcon.name}'),
                            size: 26,
                            color: dimmed
                                ? context.colors.onSurface.withValues(
                                    alpha: 0.38,
                                  )
                                : context.colors.onSurface,
                          )
                        : Text(
                            label!,
                            key: ValueKey(label),
                            maxLines: 1,
                            style: context.textTheme.labelMedium?.copyWith(
                              color: dimmed
                                  ? context.colors.onSurface.withValues(
                                      alpha: 0.38,
                                    )
                                  : context.colors.onSurface,
                              fontWeight: FontWeight.w600,
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

    final labelProgress = transitionProgress.clamp(0.0, 1.0);
    return Stack(
      clipBehavior: Clip.none,
      children: [
        Positioned(
          key: controlKey,
          left: 0,
          right: 0,
          top: (_shutterSize - _cameraControlSize) / 2,
          height: _cameraControlSize,
          child: Transform.translate(
            offset: Offset(0, 1.5 * labelProgress),
            child: control,
          ),
        ),
        if (transitionLabel != null)
          Positioned(
            left: 0,
            right: 0,
            top: _shutterSize - 20,
            height: 20,
            child: Opacity(
              key: ValueKey('camera-transition-label-${transitionLabel!}'),
              opacity: labelProgress,
              child: Transform.translate(
                offset: Offset(0, 2 * (1 - labelProgress)),
                child: OverflowBox(
                  minWidth: 0,
                  maxWidth: transitionLabelMaxWidth,
                  maxHeight: 20,
                  child: Text(
                    transitionLabel!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

class _ShutterButton extends StatelessWidget {
  const _ShutterButton({required this.busy, required this.onTap});

  final bool busy;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    return Semantics(
      label: 'Take photo',
      button: true,
      enabled: onTap != null,
      child: ExcludeSemantics(
        child: SizedBox(
          key: const ValueKey('image-camera-shutter-morph'),
          width: _shutterSize,
          height: _shutterSize,
          child: defaultTargetPlatform == TargetPlatform.iOS
              ? IosGlassNavigationButton(
                  icon: IosGlassNavigationIcon.shutter,
                  semanticLabel: 'Take photo',
                  onPressed: onTap,
                  width: _shutterSize,
                  height: _shutterSize,
                  controlSize: _shutterSize,
                  foregroundColor: context.colors.onSurface,
                  isBusy: busy,
                )
              : Material(
                  color: context.colors.surfaceContainerHighest,
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
                            : Container(
                                key: const ValueKey(
                                  'image-camera-shutter-icon',
                                ),
                                width: _shutterCoreSize,
                                height: _shutterCoreSize,
                                decoration: BoxDecoration(
                                  color: context.colors.onSurface,
                                  shape: BoxShape.circle,
                                  border: Border.all(
                                    color: context.colors.surface,
                                    width: 1.5,
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
