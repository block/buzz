import 'dart:math' show cos, min, pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The Buzz mark with wings that flutter twice when the user taps it.
///
/// The geometry and wing tuck match the desktop loading bee. When reduced
/// motion is enabled, the mark stays static.
class TappableFlappingBee extends HookConsumerWidget {
  /// The rendered width of the complete bee mark.
  final double width;

  /// The color used for the bee silhouette.
  final Color color;

  const TappableFlappingBee({
    required this.width,
    required this.color,
    super.key,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 480),
    );
    final reducedMotion = MediaQuery.disableAnimationsOf(context);

    void flutterWings() {
      if (reducedMotion) return;
      animation.forward(from: 0);
    }

    return Semantics(
      button: true,
      label: 'Buzz bee',
      hint: 'Tap to make its wings flutter',
      onTap: flutterWings,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        excludeFromSemantics: true,
        onTap: flutterWings,
        child: RepaintBoundary(
          child: AnimatedBuilder(
            animation: animation,
            builder: (context, _) {
              final flapAmount = 0.5 - (0.5 * cos(animation.value * 4 * pi));
              return CustomPaint(
                size: Size(width, width * 309 / 466),
                painter: _FlappingBeePainter(
                  color: color,
                  flapAmount: flapAmount,
                ),
              );
            },
          ),
        ),
      ),
    );
  }
}

class _FlappingBeePainter extends CustomPainter {
  final Color color;
  final double flapAmount;

  const _FlappingBeePainter({required this.color, required this.flapAmount});

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

    canvas
      ..drawPath(finishedMark, Paint()..color = color)
      ..restore();
  }

  @override
  bool shouldRepaint(_FlappingBeePainter oldDelegate) =>
      color != oldDelegate.color || flapAmount != oldDelegate.flapAmount;
}
