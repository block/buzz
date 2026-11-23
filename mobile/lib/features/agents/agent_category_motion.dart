import 'package:flutter/material.dart';

import 'agent_category_transition.dart';

/// Applies the category transition to a stable child.
class AgentCategoryMotion extends StatelessWidget {
  const AgentCategoryMotion({
    super.key,
    required this.child,
    this.slide = true,
  });

  final Widget child;
  final bool slide;

  @override
  Widget build(BuildContext context) {
    final transition = context
        .dependOnInheritedWidgetOfExactType<AgentCategoryTransition>();
    if (transition == null) return child;
    return AnimatedBuilder(
      animation: transition.animation,
      child: child,
      builder: (context, child) {
        final outgoing = transition.animation.status == AnimationStatus.reverse;
        if (!slide) {
          // Incoming contents fade over the old tray, keeping its surface solid.
          return Opacity(
            opacity: outgoing ? 1 : transition.animation.value,
            child: child,
          );
        }
        return FractionalTranslation(
          translation: Offset(
            (outgoing ? -transition.direction : transition.direction) *
                (1 - transition.animation.value),
            0,
          ),
          child: child,
        );
      },
    );
  }
}
