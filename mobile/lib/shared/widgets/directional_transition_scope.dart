import 'package:flutter/material.dart';

/// Supplies a shared directional entrance to separate foreground surfaces.
///
/// Descendants opt in with [DirectionalTransitionMotion], which lets a page
/// move its body and app-bar content together while leaving decorative
/// backgrounds stationary.
class DirectionalTransitionScope extends InheritedWidget {
  /// Horizontal displacement remaining in the transition.
  final double horizontalOffset;

  /// Current foreground opacity, from zero to one.
  final double opacity;

  /// Creates a directional transition scope.
  const DirectionalTransitionScope({
    super.key,
    required this.horizontalOffset,
    required this.opacity,
    required super.child,
  });

  /// Returns the closest transition, or null outside a transitioning surface.
  static DirectionalTransitionScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<DirectionalTransitionScope>();

  @override
  bool updateShouldNotify(DirectionalTransitionScope oldWidget) =>
      horizontalOffset != oldWidget.horizontalOffset ||
      opacity != oldWidget.opacity;
}

/// Applies the nearest [DirectionalTransitionScope] to one foreground layer.
class DirectionalTransitionMotion extends StatelessWidget {
  /// Foreground content that participates in the shared transition.
  final Widget child;

  /// Optional key for inspecting the composited translation layer.
  final Key? transformKey;

  /// Optional key for inspecting the composited opacity layer.
  final Key? opacityKey;

  /// Creates a foreground transition participant.
  const DirectionalTransitionMotion({
    super.key,
    required this.child,
    this.transformKey,
    this.opacityKey,
  });

  @override
  Widget build(BuildContext context) {
    final transition = DirectionalTransitionScope.maybeOf(context);
    if (transition == null) return child;

    return Transform.translate(
      key: transformKey,
      offset: Offset(transition.horizontalOffset, 0),
      child: Opacity(
        key: opacityKey,
        opacity: transition.opacity,
        child: child,
      ),
    );
  }
}
