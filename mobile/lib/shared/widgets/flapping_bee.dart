import 'dart:math' show min;

import 'package:flutter/material.dart';

/// The Buzz mark at a caller-controlled wing position.
///
/// The geometry matches the desktop loading bee. Callers drive the wings
/// themselves, which lets one painter serve both the tap-to-flutter mark
/// ([TappableFlappingBee]) and the pull-to-refresh indicator
/// ([BeeRefreshIndicator]).
class FlappingBee extends StatelessWidget {
  /// The rendered width of the complete bee mark.
  ///
  /// Height follows from the mark's 466:309 aspect ratio.
  final double width;

  /// The color used for the bee silhouette, wings, and pupils.
  final Color color;

  /// How far the wings are tucked toward the body, from 0 to 1.
  ///
  /// 0 renders the wings fully spread; 1 renders them at their innermost
  /// tuck. Callers animate this to flap the wings.
  final double flapAmount;

  /// How far the pupils have grown, from 0 to 1, or null for cutout eyes.
  ///
  /// Only the pull-to-refresh treatment sets this. Leaving it null preserves
  /// the mark's ordinary cutout eyes; a non-null value fills them in, with 1
  /// drawing the pupils at full size.
  final double? eyeProgress;

  const FlappingBee({
    required this.width,
    required this.color,
    required this.flapAmount,
    this.eyeProgress,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: CustomPaint(
        size: Size(width, width * 309 / 466),
        painter: _FlappingBeePainter(
          color: color,
          flapAmount: flapAmount,
          eyeProgress: eyeProgress,
        ),
      ),
    );
  }
}

class _FlappingBeePainter extends CustomPainter {
  final Color color;
  final double flapAmount;
  final double? eyeProgress;

  const _FlappingBeePainter({
    required this.color,
    required this.flapAmount,
    this.eyeProgress,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final scale = min(size.width / 466, size.height / 309);
    final renderedWidth = 466 * scale;
    final renderedHeight = 309 * scale;

    canvas
      ..save()
      ..translate(
        (size.width - renderedWidth) / 2,
        (size.height - renderedHeight) / 2,
      )
      ..scale(scale);

    final wingRadiusX = 91.7 * (1 - (0.38 * flapAmount));
    final wingTranslation = 30 * flapAmount;
    final leftWing = Path()
      ..addOval(
        Rect.fromCenter(
          center: Offset(91.7 + wingTranslation, 154.5),
          width: wingRadiusX * 2,
          height: 183.4,
        ),
      );
    final rightWing = Path()
      ..addOval(
        Rect.fromCenter(
          center: Offset(374.3 - wingTranslation, 154.5),
          width: wingRadiusX * 2,
          height: 183.4,
        ),
      );
    final body = Path()
      ..addRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(128, 0, 210, 309),
          const Radius.circular(34),
        ),
      );
    final cutouts = Path()
      ..addOval(
        Rect.fromCenter(
          center: const Offset(193.3, 84.4),
          width: 54,
          height: 54,
        ),
      )
      ..addOval(
        Rect.fromCenter(center: const Offset(276, 84.4), width: 54, height: 54),
      )
      ..addRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(166.3, 157.2, 136.9, 38.3),
          const Radius.circular(5),
        ),
      )
      ..addRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(166.9, 235.1, 136.2, 37.6),
          const Radius.circular(5),
        ),
      );
    final wings = Path.combine(PathOperation.union, leftWing, rightWing);
    final silhouette = Path.combine(PathOperation.union, wings, body);
    final finishedMark = Path.combine(
      PathOperation.difference,
      silhouette,
      cutouts,
    );

    canvas.drawPath(finishedMark, Paint()..color = color);

    if (eyeProgress case final progress?) {
      final pupilRadius = 20 * progress.clamp(0.0, 1.0);
      final pupilPaint = Paint()..color = color;
      canvas
        ..drawCircle(const Offset(193.3, 84.4), pupilRadius, pupilPaint)
        ..drawCircle(const Offset(276, 84.4), pupilRadius, pupilPaint);
    }

    canvas.restore();
  }

  @override
  bool shouldRepaint(_FlappingBeePainter oldDelegate) =>
      color != oldDelegate.color ||
      flapAmount != oldDelegate.flapAmount ||
      eyeProgress != oldDelegate.eyeProgress;
}
