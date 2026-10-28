import 'package:flutter/widgets.dart';

/// Coordinates the expanded agent control with the shortcuts it covers.
class AgentNavigationOverlay extends InheritedWidget {
  const AgentNavigationOverlay({
    super.key,
    required this.active,
    required super.child,
  });
  final ValueNotifier<bool> active;

  static ValueNotifier<bool>? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
      ?.active;

  @override
  bool updateShouldNotify(AgentNavigationOverlay oldWidget) =>
      active != oldWidget.active;
}
