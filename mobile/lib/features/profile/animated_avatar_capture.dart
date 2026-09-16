import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:camera/camera.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:google_mlkit_selfie_segmentation/google_mlkit_selfie_segmentation.dart';
import 'package:image/image.dart' as image;
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:path_provider/path_provider.dart';

import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'avatar_background_grid.dart';
import 'camera_disposal_barrier.dart';
import 'avatar_editor_option_button.dart';
import 'animated_avatar_orientation.dart';
import 'profile_avatar_draft.dart';

part 'animated_avatar_capture/review_controls.dart';
part 'animated_avatar_capture/capture_controls.dart';
part 'animated_avatar_capture/frame_processing.dart';
part 'animated_avatar_capture/error_text.dart';

const _captureDuration = Duration(seconds: 3);
const _captureFrameInterval = Duration(milliseconds: 125);
const _captureFrameCount = 24;
const _outputSize = 256;
const _mobileDefaultPersonScale = 1.15;
const _animatedReviewRailHeight = 88.0;

/// Records and prepares a short camera animation for a profile avatar.
class AnimatedAvatarCapture extends HookConsumerWidget {
  /// Creates an animated-avatar capture and review surface.
  const AnimatedAvatarCapture({
    super.key,
    required this.height,
    required this.onPrepareChanged,
    this.initialFrames = const [],
    this.disposalBarrier,
  });

  /// The vertical space available to the capture surface.
  final double height;

  /// Reports the current deferred draft-preparation callback to the parent.
  final ValueChanged<Future<ProfileAvatarDraft?> Function()?> onPrepareChanged;

  /// Seeds processed frames in lifecycle-focused widget tests.
  @visibleForTesting
  final List<Uint8List> initialFrames;

  /// Serializes ownership release with another profile capture surface.
  final CameraDisposalBarrier? disposalBarrier;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useState<CameraController?>(null);
    final controllerRef = useRef<CameraController?>(null);
    final controllerDisposal = useRef(
      disposalBarrier ?? CameraDisposalBarrier(),
    );
    final candidateRef = useRef<CameraController?>(null);
    final captureEpoch = useRef(0);
    final cameraGeneration = useState(0);
    final isInitializing = useState(true);
    final isRecording = useState(false);
    final isPreparingFrames = useState(false);
    final isProcessing = useState(false);
    final progress = useState(0.0);
    final frames = useState<List<Uint8List>>(initialFrames);
    final posterIndex = useState(0);
    final previewFrameIndex = useState(0);
    final scale = useState(_mobileDefaultPersonScale);
    final offset = useState(Offset.zero);
    final shapeScale = useState(1.12);
    final shapeOffset = useState(const Offset(0, -7 / 60));
    final activeSection = useState(_AnimatedReviewSection.person);
    final backdropColor = useState(emojiAvatarColors[8]);
    final personOutline = useState(true);
    final gestureStartScale = useRef(_mobileDefaultPersonScale);
    final error = useState<String?>(null);
    final encodedCache = useRef<_EncodedAvatarCache?>(null);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final lifecycle = ref.watch(appLifecycleProvider);
    final encodeKey = frames.value.isEmpty
        ? null
        : _EncodeKey(
            frames: frames.value,
            posterIndex: posterIndex.value,
            scale: scale.value,
            offsetX: offset.value.dx,
            offsetY: offset.value.dy,
            backdropColor: backdropColor.value,
            personOutline: personOutline.value,
            shapeScale: shapeScale.value,
            shapeOffsetX: shapeOffset.value.dx,
            shapeOffsetY: shapeOffset.value.dy,
          );
    final latestEncodeKey = useRef<_EncodeKey?>(null);
    latestEncodeKey.value = encodeKey;
    useEffect(() {
      encodedCache.value = null;
      final key = encodeKey;
      if (key == null) return null;
      final timer = Timer(const Duration(milliseconds: 180), () {
        if (encodedCache.value?.key == key) return;
        final future = compute(_encodeAvatar, key.toRequest());
        encodedCache.value = _EncodedAvatarCache(key, future);
      });
      return timer.cancel;
    }, [encodeKey]);

    useEffect(() {
      if (reduceMotion || frames.value.length < 2) return null;
      final timer = Timer.periodic(_captureFrameInterval, (_) {
        if (activeSection.value == _AnimatedReviewSection.poster) return;
        previewFrameIndex.value =
            (previewFrameIndex.value + 1) % frames.value.length;
      });
      return timer.cancel;
    }, [frames.value, reduceMotion]);

    useEffect(() {
      var disposed = false;

      if (lifecycle != AppLifecycleState.resumed || frames.value.isNotEmpty) {
        isInitializing.value = false;
        controller.value = null;
        return null;
      }

      isInitializing.value = true;

      Future<void> releaseController(CameraController? active) {
        if (active == null) return controllerDisposal.value.settled;
        return controllerDisposal.value.release(active.dispose);
      }

      Future<void> initialize() async {
        CameraController? next;
        CameraDisposalReservation? reservation;
        var installed = false;
        try {
          await controllerDisposal.value.settled;
          if (disposed) return;
          final cameras = await availableCameras();
          if (disposed || cameras.isEmpty) return;
          final selected = cameras.firstWhere(
            (camera) => camera.lensDirection == CameraLensDirection.front,
            orElse: () => cameras.first,
          );
          next = CameraController(
            selected,
            ResolutionPreset.medium,
            enableAudio: false,
            imageFormatGroup: defaultTargetPlatform == TargetPlatform.iOS
                ? ImageFormatGroup.bgra8888
                : ImageFormatGroup.yuv420,
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
          if (disposed) {
            if (identical(candidateRef.value, next)) candidateRef.value = null;
            await reservation.dispose(next.dispose);
            return;
          }
          candidateRef.value = null;
          reservation.complete();
          controllerRef.value = next;
          controller.value = next;
          installed = true;
        } catch (_) {
          if (!installed &&
              next != null &&
              identical(candidateRef.value, next)) {
            candidateRef.value = null;
            await reservation?.dispose(next.dispose);
          }
          if (!disposed) error.value = 'Could not access the camera.';
        } finally {
          if (!disposed) isInitializing.value = false;
        }
      }

      unawaited(initialize());
      return () {
        disposed = true;
        captureEpoch.value++;
        final active = controllerRef.value;
        controllerRef.value = null;
        unawaited(releaseController(active));
      };
    }, [lifecycle, frames.value.isEmpty, cameraGeneration.value]);

    Future<ProfileAvatarDraft?> prepare() async {
      final key = latestEncodeKey.value;
      if (key == null) return null;
      isProcessing.value = true;
      error.value = null;
      try {
        final cached = encodedCache.value;
        final encoding = cached != null && cached.key == key
            ? cached.future
            : compute(_encodeAvatar, key.toRequest());
        encodedCache.value = _EncodedAvatarCache(key, encoding);
        final result = await encoding;
        return ProfileAnimatedAvatarDraft(
          animation: result.animation,
          poster: result.poster,
        );
      } catch (_) {
        error.value = "We couldn't create that animation. Try again.";
        return null;
      } finally {
        if (context.mounted) isProcessing.value = false;
      }
    }

    useEffect(
      () {
        final nextPrepare = frames.value.isEmpty ? null : prepare;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) onPrepareChanged(nextPrepare);
        });
        return null;
      },
      [
        frames.value,
        posterIndex.value,
        scale.value,
        offset.value,
        backdropColor.value,
        personOutline.value,
        shapeScale.value,
        shapeOffset.value,
      ],
    );

    Future<void> record() async {
      final active = controller.value;
      if (active == null || isRecording.value) return;
      final currentCapture = ++captureEpoch.value;
      frames.value = const [];
      posterIndex.value = 0;
      previewFrameIndex.value = 0;
      scale.value = _mobileDefaultPersonScale;
      offset.value = Offset.zero;
      shapeScale.value = 1.12;
      shapeOffset.value = const Offset(0, -7 / 60);
      error.value = null;
      progress.value = 0;
      isRecording.value = true;
      final captured = <Uint8List>[];
      final startedAt = DateTime.now();
      var lastFrameAt = DateTime.fromMillisecondsSinceEpoch(0);
      var converting = false;
      var releasedCamera = false;

      Future<void> releaseCamera() async {
        if (releasedCamera || captureEpoch.value != currentCapture) return;
        releasedCamera = true;
        if (identical(controllerRef.value, active)) {
          controllerRef.value = null;
        }
        if (context.mounted && identical(controller.value, active)) {
          controller.value = null;
        }
        await controllerDisposal.value.release(active.dispose);
      }

      final timer = Timer.periodic(const Duration(milliseconds: 40), (_) {
        if (!context.mounted || captureEpoch.value != currentCapture) return;
        final elapsed = DateTime.now().difference(startedAt);
        progress.value =
            (elapsed.inMilliseconds / _captureDuration.inMilliseconds).clamp(
              0,
              1,
            );
      });

      try {
        await active.startImageStream((cameraImage) async {
          final now = DateTime.now();
          if (captureEpoch.value != currentCapture ||
              converting ||
              now.difference(lastFrameAt) < _captureFrameInterval ||
              now.difference(startedAt) >= _captureDuration) {
            return;
          }
          converting = true;
          lastFrameAt = now;
          try {
            final request = _FrameRequest.fromCameraImage(
              cameraImage,
              rotationDegrees: animatedAvatarFrameRotationDegrees(
                sensorOrientation: active.description.sensorOrientation,
                deviceOrientation: active.value.deviceOrientation,
                lensDirection: active.description.lensDirection,
              ),
              mirror:
                  active.description.lensDirection == CameraLensDirection.front,
            );
            final frame = await compute(_convertCameraFrame, request);
            if (captureEpoch.value == currentCapture) captured.add(frame);
          } finally {
            converting = false;
          }
        });
        await Future<void>.delayed(_captureDuration);
        if (captureEpoch.value != currentCapture || !context.mounted) return;
        await active.stopImageStream();
        while (converting && captureEpoch.value == currentCapture) {
          await Future<void>.delayed(const Duration(milliseconds: 10));
        }
        if (captureEpoch.value != currentCapture || !context.mounted) return;
        if (captured.length < 2) {
          throw StateError('Not enough frames were captured.');
        }
        isRecording.value = false;
        // Enter the processing state before releasing the controller. Clearing
        // the camera first briefly exposed the unavailable-camera placeholder.
        isPreparingFrames.value = true;
        await releaseCamera();
        // Cut out only the frames each device captured, then resample the
        // three-second window so Android and iOS use the same playback cadence.
        final cutouts = await _removeBackgrounds(captured);
        if (captureEpoch.value != currentCapture || !context.mounted) return;
        final processed = List<Uint8List>.unmodifiable(
          _resampleCapturedFrames(cutouts, _captureFrameCount),
        );
        if (!context.mounted) return;
        await Future.wait([
          for (final frame in processed)
            precacheImage(MemoryImage(frame), context),
        ]);
        if (context.mounted) frames.value = processed;
      } catch (_) {
        if (captureEpoch.value != currentCapture || !context.mounted) return;
        try {
          if (!releasedCamera && active.value.isStreamingImages) {
            await active.stopImageStream();
          }
        } on CameraException {
          // The camera can stop independently while the capture is unwinding.
        }
        await releaseCamera();
        if (context.mounted) {
          cameraGeneration.value++;
          error.value = 'Recording failed. Try again.';
        }
      } finally {
        timer.cancel();
        if (context.mounted) {
          progress.value = 1;
          isRecording.value = false;
          isPreparingFrames.value = false;
        }
      }
    }

    if (frames.value.isNotEmpty) {
      final selectedFrame =
          frames.value[activeSection.value == _AnimatedReviewSection.poster
              ? posterIndex.value
              : previewFrameIndex.value];
      final reduceMotion = MediaQuery.disableAnimationsOf(context);
      final previewTop = activeSection.value == _AnimatedReviewSection.color
          ? -avatarBackgroundPreviewShift
          : 0.0;
      final controlsTop = previewTop + _animatedAvatarPreviewSize;
      return SizedBox(
        height: height,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            AnimatedPositioned(
              key: const ValueKey('animated-review-preview-position'),
              duration: reduceMotion
                  ? Duration.zero
                  : const Duration(milliseconds: 150),
              curve: Curves.easeOutCubic,
              left: 0,
              right: 0,
              top: previewTop,
              height: _animatedAvatarPreviewSize,
              child: Center(
                child: _RepositionablePreviewSemantics(
                  offset: offset.value,
                  onMove: (delta) {
                    unawaited(HapticFeedback.selectionClick());
                    offset.value = Offset(
                      (offset.value.dx + delta.dx).clamp(-1.0, 1.0),
                      (offset.value.dy + delta.dy).clamp(-1.0, 1.0),
                    );
                  },
                  child: GestureDetector(
                    key: const ValueKey('animated-avatar-review-preview'),
                    behavior: HitTestBehavior.opaque,
                    onScaleStart: (_) => gestureStartScale.value = scale.value,
                    onScaleUpdate: (details) {
                      final next = Offset(
                        (offset.value.dx + details.focalPointDelta.dx / 96)
                            .clamp(-1, 1),
                        (offset.value.dy + details.focalPointDelta.dy / 96)
                            .clamp(-1, 1),
                      );
                      offset.value = next;
                      scale.value = (gestureStartScale.value * details.scale)
                          .clamp(0.7, 2.0)
                          .toDouble();
                    },
                    child: SizedBox.square(
                      dimension: _animatedAvatarPreviewSize,
                      child: Stack(
                        fit: StackFit.expand,
                        children: [
                          ClipOval(
                            child: Stack(
                              fit: StackFit.expand,
                              children: [
                                Center(
                                  child: Transform.translate(
                                    offset:
                                        const Offset(
                                          0,
                                          _animatedAvatarShapeYOffset,
                                        ) +
                                        shapeOffset.value *
                                            _animatedAvatarShapeTranslation,
                                    child: Transform.scale(
                                      scale: shapeScale.value,
                                      child: Container(
                                        width: _animatedAvatarShapeSize,
                                        height: _animatedAvatarShapeSize,
                                        decoration: BoxDecoration(
                                          color: Color(backdropColor.value),
                                          shape: BoxShape.circle,
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                                _AnimatedPersonPreview(
                                  bytes: selectedFrame,
                                  offset:
                                      offset.value *
                                      _animatedAvatarPersonTranslation,
                                  scale: scale.value,
                                  outline: personOutline.value,
                                  outlineColor: _personOutlineColor(
                                    backdropColor.value,
                                  ),
                                ),
                              ],
                            ),
                          ),
                          IgnorePointer(
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                shape: BoxShape.circle,
                                border: Border.all(
                                  color: context.colors.onSurface.withValues(
                                    alpha: 0.1,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
            AnimatedPositioned(
              key: const ValueKey('animated-review-controls-position'),
              duration: reduceMotion
                  ? Duration.zero
                  : const Duration(milliseconds: 150),
              curve: Curves.easeOutCubic,
              left: 0,
              right: 0,
              top: controlsTop,
              bottom: _animatedReviewRailHeight,
              child: Align(
                alignment: Alignment.center,
                child: SizedBox(
                  width: double.infinity,
                  child: switch (activeSection.value) {
                    _AnimatedReviewSection.person => _AnimatedFramingControl(
                      key: const ValueKey('animated-review-you'),
                      scale: scale.value,
                      outline: personOutline.value,
                      onScaleChanged: (value) => scale.value = value,
                      onOutlineChanged: (value) => personOutline.value = value,
                    ),
                    _AnimatedReviewSection.color => AvatarBackgroundGrid(
                      key: const ValueKey('animated-review-background'),
                      selectedColor: backdropColor.value,
                      onColorSelected: (value) => backdropColor.value = value,
                      colorKeyPrefix: 'animated-avatar-color',
                    ),
                    _AnimatedReviewSection.poster => _AnimatedFrameControl(
                      key: const ValueKey('animated-review-frame'),
                      frames: frames.value,
                      selectedIndex: posterIndex.value,
                      onSelected: (value) => posterIndex.value = value,
                    ),
                  },
                ),
              ),
            ),
            Positioned(
              left: 0,
              right: 0,
              bottom: 0,
              height: _animatedReviewRailHeight,
              child: _AnimatedReviewNav(
                selected: activeSection.value,
                onSelected: (section) => activeSection.value = section,
                onRetake: isProcessing.value
                    ? null
                    : () {
                        frames.value = const [];
                        onPrepareChanged(null);
                      },
              ),
            ),
            if (error.value != null)
              Positioned(
                left: 0,
                right: 0,
                bottom: _animatedReviewRailHeight + Grid.xs,
                child: _ErrorText(error.value!),
              ),
          ],
        ),
      );
    }

    final active = controller.value;
    final compactCapture = height < 400;
    final textScale = MediaQuery.textScalerOf(context).scale(1);
    final statusStyle = context.textTheme.bodyMedium;
    final errorStyle = context.textTheme.bodySmall;
    final statusHeight =
        ((statusStyle?.fontSize ?? 14) *
                (statusStyle?.height ?? 1.2) *
                textScale)
            .floorToDouble();
    final errorHeight = error.value == null
        ? 0.0
        : Grid.xs +
              (errorStyle?.fontSize ?? 12) *
                  (errorStyle?.height ?? 1.2) *
                  textScale *
                  2;
    final capturePreviewSize = compactCapture ? 180.0 : 228.0;
    final recordGap = max(
      Grid.xs,
      height - capturePreviewSize - Grid.xs - statusHeight - errorHeight - 64,
    );
    Widget captureContent() => Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox.square(
          key: const ValueKey('animated-avatar-capture-preview'),
          dimension: capturePreviewSize,
          child: Stack(
            fit: StackFit.expand,
            children: [
              Padding(
                padding: const EdgeInsets.all(4),
                child: ClipOval(
                  child: ColoredBox(
                    color: Colors.black,
                    child: isPreparingFrames.value
                        ? Center(
                            child: BuzzLoadingIndicator(
                              size: 44,
                              color: context.colors.onSurface,
                              semanticLabel: 'Preparing animated avatar',
                            ),
                          )
                        : active != null
                        ? _AspectCorrectCameraPreview(controller: active)
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
              if (isRecording.value)
                Padding(
                  // A progress stroke is centred on its circular path. Inset
                  // it so the outer half is not clipped by the preview stack.
                  padding: const EdgeInsets.all(2),
                  child: CircularProgressIndicator(
                    key: const ValueKey('animated-avatar-recording-ring'),
                    value: progress.value,
                    strokeWidth: 4,
                    strokeCap: StrokeCap.round,
                    color: context.colors.onSurface,
                    backgroundColor: context.colors.outlineVariant,
                  ),
                ),
            ],
          ),
        ),
        const SizedBox(height: Grid.xs),
        Text(
          isRecording.value
              ? 'Recording…'
              : isPreparingFrames.value
              ? 'Cutting you out of the background…'
              : 'Line up your shot.',
          textAlign: TextAlign.center,
          style: context.textTheme.bodyMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
        if (error.value != null) _ErrorText(error.value!),
        SizedBox(height: recordGap),
        _AnimatedRecordButton(
          busy: isRecording.value || isPreparingFrames.value,
          onPressed: active == null
              ? null
              : () {
                  unawaited(HapticFeedback.mediumImpact());
                  unawaited(record());
                },
        ),
      ],
    );
    return SizedBox(
      height: height,
      child: SingleChildScrollView(child: captureContent()),
    );
  }
}

List<Uint8List> _resampleCapturedFrames(
  List<Uint8List> frames,
  int targetCount,
) {
  if (frames.isEmpty || targetCount <= 0) return const [];
  if (targetCount == 1) return [frames.first];
  if (frames.length == 1) return List.filled(targetCount, frames.single);
  if (frames.length == targetCount) return frames;

  return List.generate(targetCount, (index) {
    final progress = index / (targetCount - 1);
    final sourceIndex = (progress * (frames.length - 1)).round();
    return frames[sourceIndex];
  }, growable: false);
}

@immutable
class _FramePlane {
  const _FramePlane(this.bytes, this.bytesPerRow, this.bytesPerPixel);

  final Uint8List bytes;
  final int bytesPerRow;
  final int bytesPerPixel;
}

@immutable
class _FrameRequest {
  const _FrameRequest({
    required this.width,
    required this.height,
    required this.planes,
    required this.isBgra,
    required this.rotationDegrees,
    required this.mirror,
  });

  factory _FrameRequest.fromCameraImage(
    CameraImage frame, {
    required int rotationDegrees,
    required bool mirror,
  }) => _FrameRequest(
    width: frame.width,
    height: frame.height,
    planes: frame.planes
        .map(
          (plane) => _FramePlane(
            Uint8List.fromList(plane.bytes),
            plane.bytesPerRow,
            frame.format.group == ImageFormatGroup.bgra8888
                ? 4
                : plane.bytesPerPixel ?? 1,
          ),
        )
        .toList(growable: false),
    isBgra: frame.format.group == ImageFormatGroup.bgra8888,
    rotationDegrees: rotationDegrees,
    mirror: mirror,
  );

  final int width;
  final int height;
  final List<_FramePlane> planes;
  final bool isBgra;
  final int rotationDegrees;
  final bool mirror;
}

Uint8List _convertCameraFrame(_FrameRequest request) {
  var result = image.Image(width: request.width, height: request.height);
  if (request.isBgra) {
    final plane = request.planes.first;
    for (var y = 0; y < request.height; y++) {
      for (var x = 0; x < request.width; x++) {
        final index = y * plane.bytesPerRow + x * plane.bytesPerPixel;
        result.setPixelRgba(
          x,
          y,
          plane.bytes[index + 2],
          plane.bytes[index + 1],
          plane.bytes[index],
          plane.bytes[index + 3],
        );
      }
    }
  } else {
    final yPlane = request.planes[0];
    final uPlane = request.planes[1];
    final vPlane = request.planes[2];
    for (var y = 0; y < request.height; y++) {
      for (var x = 0; x < request.width; x++) {
        final yValue = yPlane.bytes[y * yPlane.bytesPerRow + x];
        final uvX = x ~/ 2;
        final uvY = y ~/ 2;
        final u =
            uPlane.bytes[uvY * uPlane.bytesPerRow + uvX * uPlane.bytesPerPixel];
        final v =
            vPlane.bytes[uvY * vPlane.bytesPerRow + uvX * vPlane.bytesPerPixel];
        final red = (yValue + 1.402 * (v - 128)).round().clamp(0, 255);
        final green = (yValue - 0.344136 * (u - 128) - 0.714136 * (v - 128))
            .round()
            .clamp(0, 255);
        final blue = (yValue + 1.772 * (u - 128)).round().clamp(0, 255);
        result.setPixelRgb(x, y, red, green, blue);
      }
    }
  }
  // iOS pre-rotates and mirrors BGRA buffers. Android YUV buffers remain
  // sensor-oriented and need both corrections here.
  if (!request.isBgra && request.rotationDegrees != 0) {
    result = image.copyRotate(result, angle: request.rotationDegrees);
  }
  if (!request.isBgra && request.mirror) {
    result = image.flipHorizontal(result);
  }
  final side = result.width < result.height ? result.width : result.height;
  result = image.copyCrop(
    result,
    x: (result.width - side) ~/ 2,
    y: (result.height - side) ~/ 2,
    width: side,
    height: side,
  );
  result = image.copyResize(result, width: _outputSize, height: _outputSize);
  return image.encodePng(result, level: 4);
}

@immutable
class _EncodeRequest {
  const _EncodeRequest({
    required this.frames,
    required this.posterIndex,
    required this.scale,
    required this.offsetX,
    required this.offsetY,
    required this.backdropColor,
    required this.personOutline,
    required this.shapeScale,
    required this.shapeOffsetX,
    required this.shapeOffsetY,
  });

  final List<Uint8List> frames;
  final int posterIndex;
  final double scale;
  final double offsetX;
  final double offsetY;
  final int backdropColor;
  final bool personOutline;
  final double shapeScale;
  final double shapeOffsetX;
  final double shapeOffsetY;
}

@immutable
class _EncodeKey {
  const _EncodeKey({
    required this.frames,
    required this.posterIndex,
    required this.scale,
    required this.offsetX,
    required this.offsetY,
    required this.backdropColor,
    required this.personOutline,
    required this.shapeScale,
    required this.shapeOffsetX,
    required this.shapeOffsetY,
  });

  final List<Uint8List> frames;
  final int posterIndex;
  final double scale;
  final double offsetX;
  final double offsetY;
  final int backdropColor;
  final bool personOutline;
  final double shapeScale;
  final double shapeOffsetX;
  final double shapeOffsetY;

  _EncodeRequest toRequest() => _EncodeRequest(
    frames: frames,
    posterIndex: posterIndex,
    scale: scale,
    offsetX: offsetX,
    offsetY: offsetY,
    backdropColor: backdropColor,
    personOutline: personOutline,
    shapeScale: shapeScale,
    shapeOffsetX: shapeOffsetX,
    shapeOffsetY: shapeOffsetY,
  );

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is _EncodeKey &&
          identical(frames, other.frames) &&
          posterIndex == other.posterIndex &&
          scale == other.scale &&
          offsetX == other.offsetX &&
          offsetY == other.offsetY &&
          backdropColor == other.backdropColor &&
          personOutline == other.personOutline &&
          shapeScale == other.shapeScale &&
          shapeOffsetX == other.shapeOffsetX &&
          shapeOffsetY == other.shapeOffsetY;

  @override
  int get hashCode => Object.hash(
    identityHashCode(frames),
    posterIndex,
    scale,
    offsetX,
    offsetY,
    backdropColor,
    personOutline,
    shapeScale,
    shapeOffsetX,
    shapeOffsetY,
  );
}

class _EncodedAvatarCache {
  const _EncodedAvatarCache(this.key, this.future);

  final _EncodeKey key;
  final Future<_EncodedAvatar> future;
}

@immutable
class _EncodedAvatar {
  const _EncodedAvatar(this.animation, this.poster);

  final Uint8List animation;
  final Uint8List poster;
}

_EncodedAvatar _encodeAvatar(_EncodeRequest request) {
  final composed = request.frames
      .map((bytes) {
        final source = image.decodePng(bytes)!;
        final scaledSize = (_outputSize * request.scale).round().clamp(
          1,
          _outputSize * 2,
        );
        final scaledPerson = image.copyResize(
          source,
          width: scaledSize,
          height: scaledSize,
        );
        final person = image.Image(
          width: _outputSize,
          height: _outputSize,
          numChannels: 4,
        );
        const previewSize = _animatedAvatarPreviewSize;
        const previewTranslation = _animatedAvatarPersonTranslation;
        final translationScale = _outputSize / previewSize;
        image.compositeImage(
          person,
          scaledPerson,
          dstW: scaledSize,
          dstH: scaledSize,
          dstX:
              ((_outputSize - scaledSize) / 2 +
                      request.offsetX * previewTranslation * translationScale)
                  .round(),
          dstY:
              ((_outputSize - scaledSize) / 2 +
                      request.offsetY * previewTranslation * translationScale)
                  .round(),
        );
        final frame = image.Image(
          width: _outputSize,
          height: _outputSize,
          numChannels: 4,
        );
        final color = request.backdropColor;
        image.fillCircle(
          frame,
          x: _animatedAvatarShapeX(request.shapeOffsetX),
          y: _animatedAvatarShapeY(request.shapeOffsetY),
          radius: _animatedAvatarShapeRadius(request.shapeScale),
          color: image.ColorRgba8(
            (color >> 16) & 0xff,
            (color >> 8) & 0xff,
            color & 0xff,
            255,
          ),
        );
        if (request.personOutline) {
          final outline = image.Image.from(person);
          final outlineColor = _personOutlineColor(request.backdropColor);
          for (final pixel in outline) {
            pixel
              ..r = (outlineColor.r * 255).round()
              ..g = (outlineColor.g * 255).round()
              ..b = (outlineColor.b * 255).round()
              ..a = (pixel.a * 0.92).round();
          }
          for (final (x, y) in _animatedAvatarOutlineOffsets) {
            image.compositeImage(frame, outline, dstX: x, dstY: y);
          }
        }
        image.compositeImage(frame, person);
        frame.frameDuration = _captureFrameInterval.inMilliseconds;
        return frame;
      })
      .toList(growable: false);
  final pingPong = <image.Image>[
    ...composed,
    ...composed.reversed.skip(1).skipLast(1),
  ];
  final encoder = image.PngEncoder()..start(pingPong.length);
  for (final frame in pingPong) {
    encoder.addFrame(frame);
  }
  final animation = encoder.finish()!;
  final poster = image.encodePng(
    composed[request.posterIndex.clamp(0, composed.length - 1)],
  );
  return _EncodedAvatar(animation, poster);
}

extension<T> on Iterable<T> {
  Iterable<T> skipLast(int count) {
    final values = toList(growable: false);
    return values.take((values.length - count).clamp(0, values.length));
  }
}
