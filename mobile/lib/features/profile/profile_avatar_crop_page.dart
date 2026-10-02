import 'dart:async';
import 'dart:math' as math;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:image/image.dart' as image;

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';

const _cropOutputSize = 512;

/// Lets a person pan and zoom a selected image inside the final avatar mask.
class ProfileAvatarCropPage extends HookWidget {
  /// Creates a crop page that resolves the selected image asynchronously.
  const ProfileAvatarCropPage({super.key, required this.imageBytes});

  /// The selected image bytes, or null when selection was cancelled.
  final Future<Uint8List?> imageBytes;

  @override
  Widget build(BuildContext context) {
    final controller = useTransformationController();
    final cropScale = useState(1.0);
    final preparedBytes = useState<Uint8List?>(null);
    final loadError = useState(false);
    final dimensionsFuture = useMemoized(
      () => preparedBytes.value == null
          ? null
          : compute(_decodeDimensions, preparedBytes.value!),
      [preparedBytes.value],
    );
    final dimensionsSnapshot = useFuture(dimensionsFuture);
    final dimensionsData = dimensionsSnapshot.data;
    final dimensions = dimensionsData == null
        ? null
        : _ImageDimensions(dimensionsData[0], dimensionsData[1]);
    final initializedForSize = useRef<Size?>(null);
    final cropGeometry = useRef<_CropGeometry?>(null);
    final displaySize = useRef<Size?>(null);
    final isClampingTransform = useRef(false);
    final isCropping = useState(false);

    useEffect(() {
      void clampTransform() {
        if (isClampingTransform.value) return;
        final geometry = cropGeometry.value;
        final imageSize = displaySize.value;
        if (geometry == null || imageSize == null) return;

        final matrix = controller.value.clone();
        final scale = matrix.getMaxScaleOnAxis();
        final cropLeft = (geometry.canvasWidth - geometry.cropDiameter) / 2;
        final cropTop = (geometry.canvasHeight - geometry.cropDiameter) / 2;
        final cropRight = cropLeft + geometry.cropDiameter;
        final cropBottom = cropTop + geometry.cropDiameter;
        final minX = cropRight - imageSize.width * scale;
        final minY = cropBottom - imageSize.height * scale;
        final nextX = matrix.storage[12].clamp(minX, cropLeft).toDouble();
        final nextY = matrix.storage[13].clamp(minY, cropTop).toDouble();
        if ((nextX - matrix.storage[12]).abs() < 0.01 &&
            (nextY - matrix.storage[13]).abs() < 0.01) {
          return;
        }
        matrix.storage[12] = nextX;
        matrix.storage[13] = nextY;
        isClampingTransform.value = true;
        controller.value = matrix;
        isClampingTransform.value = false;
      }

      controller.addListener(clampTransform);
      return () => controller.removeListener(clampTransform);
    }, [controller]);

    useEffect(() {
      var disposed = false;
      imageBytes.then(
        (bytes) {
          if (disposed) return;
          if (bytes == null) {
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (context.mounted) Navigator.pop(context);
            });
            return;
          }
          preparedBytes.value = bytes;
        },
        onError: (_) {
          if (!disposed) loadError.value = true;
        },
      );
      return () => disposed = true;
    }, [imageBytes]);

    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        centerTitle: true,
        titleSpacing: 0,
        title: Text(
          'Position Photo',
          maxLines: 1,
          style: context.textTheme.titleSmall?.copyWith(
            color: Colors.white,
            fontWeight: FontWeight.w600,
          ),
        ),
        leadingWidth: 120,
        leading: Padding(
          padding: const EdgeInsets.only(left: Grid.gutter),
          child: Align(
            alignment: Alignment.centerLeft,
            child: _CropHeaderButton(
              label: 'Cancel',
              onPressed: isCropping.value ? null : () => Navigator.pop(context),
            ),
          ),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: Grid.gutter),
            child: _CropHeaderButton(
              key: const ValueKey('avatar-crop-use-photo'),
              label: 'Save',
              loading: isCropping.value,
              onPressed:
                  isCropping.value ||
                      preparedBytes.value == null ||
                      dimensions == null
                  ? null
                  : () async {
                      isCropping.value = true;
                      unawaited(HapticFeedback.lightImpact());
                      try {
                        if (!context.mounted) return;
                        final geometry = cropGeometry.value;
                        if (geometry == null) return;
                        final cropped = await compute(
                          _cropAvatar,
                          _CropRequest(
                            bytes: preparedBytes.value!,
                            sourceWidth: dimensions.width,
                            sourceHeight: dimensions.height,
                            canvasWidth: geometry.canvasWidth,
                            canvasHeight: geometry.canvasHeight,
                            cropDiameter: geometry.cropDiameter,
                            transform: List<double>.from(
                              controller.value.storage,
                            ),
                          ),
                        );
                        if (context.mounted) Navigator.pop(context, cropped);
                      } finally {
                        if (context.mounted) isCropping.value = false;
                      }
                    },
            ),
          ),
        ],
      ),
      body: loadError.value || dimensionsSnapshot.hasError
          ? const Center(
              child: Text(
                "We couldn't prepare that photo.",
                style: TextStyle(color: Colors.white),
              ),
            )
          : preparedBytes.value == null || dimensions == null
          ? const Center(
              child: BuzzLoadingIndicator(
                color: Colors.white,
                semanticLabel: 'Preparing photo',
              ),
            )
          : LayoutBuilder(
              builder: (context, constraints) {
                final canvasWidth = constraints.maxWidth;
                final canvasHeight = math.max(1.0, constraints.maxHeight);
                final cropDiameter = math.min(canvasWidth, canvasHeight);
                cropGeometry.value = _CropGeometry(
                  canvasWidth: canvasWidth,
                  canvasHeight: canvasHeight,
                  cropDiameter: cropDiameter,
                );
                final aspect = dimensions.width / dimensions.height;
                final displayWidth = aspect >= 1
                    ? cropDiameter * aspect
                    : cropDiameter;
                final displayHeight = aspect >= 1
                    ? cropDiameter
                    : cropDiameter / aspect;
                displaySize.value = Size(displayWidth, displayHeight);
                final canvasSize = Size(canvasWidth, canvasHeight);
                if (initializedForSize.value != canvasSize) {
                  initializedForSize.value = canvasSize;
                  controller.value = Matrix4.identity()
                    ..translateByDouble(
                      (canvasWidth - displayWidth) / 2,
                      (canvasHeight - displayHeight) / 2,
                      0,
                      1,
                    );
                }
                void movePhoto(Offset delta) {
                  unawaited(HapticFeedback.selectionClick());
                  final matrix = controller.value.clone();
                  matrix.storage[12] += delta.dx;
                  matrix.storage[13] += delta.dy;
                  controller.value = matrix;
                }

                void zoomPhoto(double factor) {
                  unawaited(HapticFeedback.selectionClick());
                  final matrix = controller.value.clone();
                  final currentScale = matrix.getMaxScaleOnAxis();
                  final nextScale = (currentScale * factor)
                      .clamp(1.0, 5.0)
                      .toDouble();
                  final appliedFactor = nextScale / currentScale;
                  final center = Offset(canvasWidth / 2, canvasHeight / 2);
                  matrix.storage[12] =
                      center.dx -
                      (center.dx - matrix.storage[12]) * appliedFactor;
                  matrix.storage[13] =
                      center.dy -
                      (center.dy - matrix.storage[13]) * appliedFactor;
                  matrix.storage[0] *= appliedFactor;
                  matrix.storage[5] *= appliedFactor;
                  matrix.storage[10] *= appliedFactor;
                  controller.value = matrix;
                  cropScale.value = nextScale;
                }

                final currentScale = cropScale.value;
                final maskBottom = (canvasHeight + cropDiameter) / 2;
                return Stack(
                  children: [
                    SizedBox(
                      width: canvasWidth,
                      height: canvasHeight,
                      child: Stack(
                        fit: StackFit.expand,
                        children: [
                          ClipRect(
                            child: Semantics(
                              label: 'Photo crop',
                              value: '${(currentScale * 100).round()}% zoom',
                              customSemanticsActions: {
                                const CustomSemanticsAction(
                                  label: 'Move left',
                                ): () =>
                                    movePhoto(const Offset(-24, 0)),
                                const CustomSemanticsAction(
                                  label: 'Move right',
                                ): () =>
                                    movePhoto(const Offset(24, 0)),
                                const CustomSemanticsAction(
                                  label: 'Move up',
                                ): () =>
                                    movePhoto(const Offset(0, -24)),
                                const CustomSemanticsAction(
                                  label: 'Move down',
                                ): () =>
                                    movePhoto(const Offset(0, 24)),
                                if (currentScale < 5)
                                  const CustomSemanticsAction(
                                    label: 'Zoom in',
                                  ): () =>
                                      zoomPhoto(1.1),
                                if (currentScale > 1)
                                  const CustomSemanticsAction(
                                    label: 'Zoom out',
                                  ): () =>
                                      zoomPhoto(1 / 1.1),
                              },
                              child: ExcludeSemantics(
                                child: InteractiveViewer(
                                  key: const ValueKey('avatar-crop-viewer'),
                                  transformationController: controller,
                                  constrained: false,
                                  panEnabled: true,
                                  scaleEnabled: true,
                                  minScale: 1,
                                  maxScale: 5,
                                  onInteractionUpdate: (_) {
                                    final nextScale = controller.value
                                        .getMaxScaleOnAxis();
                                    if ((cropScale.value - nextScale).abs() >
                                        0.001) {
                                      cropScale.value = nextScale;
                                    }
                                  },
                                  boundaryMargin: EdgeInsets.all(
                                    math.max(canvasWidth, canvasHeight),
                                  ),
                                  child: Image.memory(
                                    preparedBytes.value!,
                                    width: displayWidth,
                                    height: displayHeight,
                                    fit: BoxFit.fill,
                                    gaplessPlayback: true,
                                  ),
                                ),
                              ),
                            ),
                          ),
                          IgnorePointer(
                            child: CustomPaint(
                              painter: _CropMaskPainter(cropDiameter),
                            ),
                          ),
                        ],
                      ),
                    ),
                    Positioned(
                      left: 0,
                      right: 0,
                      top: math.min(
                        maskBottom + Grid.twelve,
                        canvasHeight - 24,
                      ),
                      child: const Center(
                        child: Text(
                          'Move and scale',
                          style: TextStyle(color: Colors.white70),
                        ),
                      ),
                    ),
                  ],
                );
              },
            ),
    );
  }
}

class _CropHeaderButton extends StatelessWidget {
  const _CropHeaderButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.loading = false,
  });

  final String label;
  final VoidCallback? onPressed;
  final bool loading;

  @override
  Widget build(BuildContext context) => ConstrainedBox(
    constraints: const BoxConstraints(minWidth: 64),
    child: SizedBox(
      height: 40,
      child: TextButton(
        style: TextButton.styleFrom(
          foregroundColor: Colors.white,
          disabledForegroundColor: Colors.white54,
          backgroundColor: Colors.white.withValues(alpha: 0.18),
          disabledBackgroundColor: Colors.white.withValues(alpha: 0.1),
          shape: const StadiumBorder(),
          padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
          tapTargetSize: MaterialTapTargetSize.shrinkWrap,
        ),
        onPressed: onPressed,
        child: loading
            ? const BuzzLoadingIndicator(
                size: 18,
                color: Colors.white,
                semanticLabel: 'Saving photo',
              )
            : Text(label, maxLines: 1, softWrap: false),
      ),
    ),
  );
}

class _CropMaskPainter extends CustomPainter {
  const _CropMaskPainter(this.cropDiameter);

  final double cropDiameter;

  @override
  void paint(Canvas canvas, Size size) {
    final circle = Rect.fromCircle(
      center: size.center(Offset.zero),
      radius: cropDiameter / 2 - 2,
    );
    final mask = Path()
      ..fillType = PathFillType.evenOdd
      ..addRect(Offset.zero & size)
      ..addOval(circle);
    canvas.drawPath(
      mask,
      Paint()..color = Colors.black.withValues(alpha: 0.58),
    );
    canvas.drawOval(
      circle,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2
        ..color = Colors.white.withValues(alpha: 0.9),
    );
  }

  @override
  bool shouldRepaint(_CropMaskPainter oldDelegate) =>
      oldDelegate.cropDiameter != cropDiameter;
}

class _CropGeometry {
  const _CropGeometry({
    required this.canvasWidth,
    required this.canvasHeight,
    required this.cropDiameter,
  });

  final double canvasWidth;
  final double canvasHeight;
  final double cropDiameter;
}

class _ImageDimensions {
  const _ImageDimensions(this.width, this.height);
  final int width;
  final int height;
}

List<int> _decodeDimensions(Uint8List bytes) {
  final decoded = image.decodeImage(bytes);
  if (decoded == null) throw const FormatException('Unsupported image');
  return [decoded.width, decoded.height];
}

class _CropRequest {
  const _CropRequest({
    required this.bytes,
    required this.sourceWidth,
    required this.sourceHeight,
    required this.canvasWidth,
    required this.canvasHeight,
    required this.cropDiameter,
    required this.transform,
  });

  final Uint8List bytes;
  final int sourceWidth;
  final int sourceHeight;
  final double canvasWidth;
  final double canvasHeight;
  final double cropDiameter;
  final List<double> transform;
}

Uint8List _cropAvatar(_CropRequest request) {
  final source = image.decodeImage(request.bytes);
  if (source == null) throw const FormatException('Unsupported image');
  final matrix = Matrix4.fromList(request.transform);
  final scale = matrix.getMaxScaleOnAxis();
  final aspect = request.sourceWidth / request.sourceHeight;
  final displayWidth = aspect >= 1
      ? request.cropDiameter * aspect
      : request.cropDiameter;
  final pixelsPerPoint = request.sourceWidth / displayWidth;
  final cropSize = (request.cropDiameter / scale * pixelsPerPoint)
      .round()
      .clamp(1, source.width < source.height ? source.width : source.height);
  final cropLeft = (request.canvasWidth - request.cropDiameter) / 2;
  final cropTop = (request.canvasHeight - request.cropDiameter) / 2;
  final x = ((cropLeft - matrix.storage[12]) / scale * pixelsPerPoint)
      .round()
      .clamp(0, source.width - cropSize);
  final y = ((cropTop - matrix.storage[13]) / scale * pixelsPerPoint)
      .round()
      .clamp(0, source.height - cropSize);
  final cropped = image.copyCrop(
    source,
    x: x,
    y: y,
    width: cropSize,
    height: cropSize,
  );
  final resized = image.copyResize(
    cropped,
    width: _cropOutputSize,
    height: _cropOutputSize,
  );
  return image.encodeJpg(resized, quality: 90);
}
