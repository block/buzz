import 'dart:math' show cos, pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The Orbit mark that bounces slightly when the user taps it.
class TappableFlappingBee extends HookConsumerWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The color used for the silhouette.
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

    void bounceLogo() {
      if (reducedMotion) return;
      animation.forward(from: 0);
    }

    return Semantics(
      button: true,
      label: 'Orbit logo',
      hint: 'Tap to make it bounce',
      onTap: bounceLogo,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        excludeFromSemantics: true,
        onTap: bounceLogo,
        child: RepaintBoundary(
          child: AnimatedBuilder(
            animation: animation,
            builder: (context, _) {
              // Smooth cosine bounce: 1.0 -> 0.85 -> 1.0
              final scaleFactor = 1.0 - 0.15 * (0.5 - 0.5 * cos(animation.value * 2 * pi));
              return Transform.scale(
                scale: scaleFactor,
                child: SvgPicture.asset(
                  'assets/images/orbit.svg',
                  width: width,
                  height: width * 300 / 306,
                  colorFilter: ColorFilter.mode(color, BlendMode.srcIn),
                ),
              );
            },
          ),
        ),
      ),
    );
  }
}
