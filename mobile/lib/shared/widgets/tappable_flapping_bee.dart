import 'dart:math' show cos, pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'flapping_bee.dart';

/// The Zorro mark with a quick double strike when the user taps it.
///
/// The geometry matches the desktop mark. When reduced motion is enabled, the
/// mark stays static.
class TappableZorroHat extends HookWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The color used for the mark.
  final Color color;

  const TappableZorroHat({required this.width, required this.color, super.key});

  @override
  Widget build(BuildContext context) {
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
              return FlappingBee(
                width: width,
                color: color,
                flapAmount: strikeAmount,
              );
            },
          ),
        ),
      ),
    );
  }
}

/// Compatibility wrapper for the former tappable loading-bee API.
class TappableFlappingBee extends StatelessWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The base ink color used for the mark.
  final Color color;

  const TappableFlappingBee({
    required this.width,
    required this.color,
    super.key,
  });

  @override
  Widget build(BuildContext context) =>
      TappableZorroHat(width: width, color: color);
}
