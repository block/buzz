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
