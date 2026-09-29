import 'dart:math' show cos, pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'flapping_bee.dart';

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
              return FlappingBee(
                width: width,
                color: color,
                flapAmount: flapAmount,
              );
            },
          ),
        ),
      ),
    );
  }
}
