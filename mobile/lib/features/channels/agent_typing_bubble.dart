import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/theme/theme.dart';

/// Pending reply with three staggered dots using the desktop typing bounce.
class AgentTypingBubble extends HookWidget {
  const AgentTypingBubble({super.key});

  @override
  Widget build(BuildContext context) {
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 1200),
    );
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    useEffect(() {
      if (reducedMotion) {
        animation.stop();
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, reducedMotion]);
    return Container(
      key: const ValueKey('agent-typing-bubble'),
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.xs,
        vertical: Grid.xxs,
      ),
      decoration: BoxDecoration(
        color: context.colors.secondaryContainer,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      child: RepaintBoundary(
        child: AnimatedBuilder(
          animation: animation,
          builder: (context, _) => SizedBox(
            height: 24,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: List.generate(3, (index) {
                // Desktop: 1.2s linear cycle, 80ms stagger, peak at 20%,
                // resting from 40% onward. Keep all three dots still for Reduce Motion.
                final phase = (animation.value - index * 80 / 1200) % 1;
                final hop = reducedMotion || phase >= 0.4
                    ? 0.0
                    : phase < 0.2
                    ? phase / 0.2
                    : (0.4 - phase) / 0.2;
                return Padding(
                  padding: EdgeInsets.only(left: index == 0 ? 0 : Grid.quarter),
                  child: Transform.translate(
                    key: ValueKey('agent-typing-dot-$index'),
                    offset: Offset(0, -2 * hop),
                    child: Opacity(
                      opacity: reducedMotion ? 1 : 0.4 + 0.6 * hop,
                      child: Container(
                        width: 7,
                        height: 7,
                        decoration: BoxDecoration(
                          color: context.colors.onSurfaceVariant,
                          shape: BoxShape.circle,
                        ),
                      ),
                    ),
                  ),
                );
              }),
            ),
          ),
        ),
      ),
    );
  }
}
