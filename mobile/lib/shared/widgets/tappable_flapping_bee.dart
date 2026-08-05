import 'dart:math' show cos, min, pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The Buzz mark with a quick double strike when the user taps it.
///
/// The geometry matches the desktop mark. When reduced motion is enabled, the
/// mark stays static.
class TappableFlappingBee extends HookConsumerWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The color used for the mark.
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

    void strikeMark() {
      if (reducedMotion) return;
      animation.forward(from: 0);
    }

    return Semantics(
      button: true,
      label: 'Zorro mark',
      hint: 'Tap to animate the mark',
      onTap: strikeMark,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        excludeFromSemantics: true,
        onTap: strikeMark,
        child: RepaintBoundary(
          child: AnimatedBuilder(
            animation: animation,
            builder: (context, _) {
              final strikeAmount = 0.5 - (0.5 * cos(animation.value * 4 * pi));
              return CustomPaint(
                size: Size.square(width),
                painter: _FlappingBeePainter(
                  color: color,
                  flapAmount: strikeAmount,
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
    final animatedScale = 1 + (0.04 * flapAmount);
    final scale = min(size.width / 512, size.height / 512) * animatedScale;
    final renderedWidth = 512 * scale;
    final renderedHeight = 512 * scale;

    canvas
      ..save()
      ..translate(
        (size.width - renderedWidth) / 2,
        (size.height - renderedHeight) / 2,
      )
      ..scale(scale);

    final finishedMark = Path()
      ..moveTo(72, 64)
      ..lineTo(440, 64)
      ..lineTo(440, 168)
      ..lineTo(224, 344)
      ..lineTo(440, 344)
      ..lineTo(440, 448)
      ..lineTo(72, 448)
      ..lineTo(72, 344)
      ..lineTo(288, 168)
      ..lineTo(72, 168)
      ..close();

    canvas
      ..drawPath(finishedMark, Paint()..color = color)
      ..restore();
  }

  @override
  bool shouldRepaint(_FlappingBeePainter oldDelegate) =>
      color != oldDelegate.color || flapAmount != oldDelegate.flapAmount;
}
