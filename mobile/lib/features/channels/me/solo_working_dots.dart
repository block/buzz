import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Desktop's three quiet opacity pulses. The containing row owns semantics.
class SoloWorkingDots extends HookWidget {
  const SoloWorkingDots({super.key, required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 1400),
    );
    useEffect(() {
      if (reducedMotion) {
        animation.stop();
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, reducedMotion]);

    return ExcludeSemantics(
      child: RepaintBoundary(
        child: AnimatedBuilder(
          animation: animation,
          builder: (context, _) => SizedBox(
            height: 20,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (var index = 0; index < 3; index++) ...[
                  if (index > 0) const SizedBox(width: 2),
                  Opacity(
                    opacity: reducedMotion
                        ? 1
                        : 0.35 +
                              0.65 *
                                  (1 -
                                      math.cos(
                                        2 *
                                            math.pi *
                                            (animation.value - index / 7),
                                      )) /
                                  2,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: color,
                        shape: BoxShape.circle,
                      ),
                      child: const SizedBox.square(dimension: 4),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
