import 'package:flutter/material.dart';

/// Shares directional motion between the category preview and filmstrip.
class AgentCategoryTransition extends InheritedWidget {
  const AgentCategoryTransition({
    super.key,
    required this.animation,
    required this.direction,
    required super.child,
  });

  final Animation<double> animation;
  final double direction;

  @override
  bool updateShouldNotify(AgentCategoryTransition oldWidget) =>
      animation != oldWidget.animation || direction != oldWidget.direction;
}

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
