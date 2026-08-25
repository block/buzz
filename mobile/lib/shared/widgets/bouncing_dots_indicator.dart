import 'dart:math' show pi, sin;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Three subtly bouncing dots for short, indeterminate waiting states.
class BouncingDotsIndicator extends HookWidget {
  /// The color of each dot.
  final Color color;

  /// The diameter of each dot.
  final double dotSize;

  /// The horizontal space between dots.
  final double gap;

  /// The accessibility announcement for this waiting state.
  final String semanticLabel;

  /// Creates a three-dot indeterminate activity indicator.
  const BouncingDotsIndicator({
    required this.color,
    required this.semanticLabel,
    this.dotSize = 6,
    this.gap = 4,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 1000),
    );

    useEffect(() {
      if (reducedMotion) {
        animation
          ..stop()
          ..value = 0;
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, reducedMotion]);

    return Semantics(
      label: semanticLabel,
      liveRegion: true,
      child: ExcludeSemantics(
        child: AnimatedBuilder(
          animation: animation,
          builder: (context, _) => Row(
            key: const ValueKey('bouncing-dots-indicator'),
            mainAxisSize: MainAxisSize.min,
            children: [
              for (var index = 0; index < 3; index++) ...[
                if (index > 0) SizedBox(width: gap),
                Transform.translate(
                  key: ValueKey('bouncing-dot-$index'),
                  offset: Offset(
                    0,
                    reducedMotion
                        ? 0
                        : -dotSize *
                              0.32 *
                              _positiveWave(animation.value, index),
                  ),
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      color: color,
                    ),
                    child: SizedBox.square(dimension: dotSize),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

double _positiveWave(double progress, int index) {
  // Match desktop onboarding's 120 ms stagger while keeping the motion soft.
  final staggeredProgress = progress - (index * 0.12);
  return sin(staggeredProgress * 2 * pi).clamp(0, 1).toDouble();
}
