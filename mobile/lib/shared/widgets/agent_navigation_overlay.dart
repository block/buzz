import 'package:flutter/widgets.dart';

/// Coordinates the expanded agent control with the shortcuts it covers.
class AgentNavigationOverlay extends InheritedWidget {
  const AgentNavigationOverlay({
    super.key,
    required this.active,
    this.onRecordingChanged,
    required super.child,
  });
  final ValueNotifier<bool> active;
  final ValueChanged<bool>? onRecordingChanged;

  static ValueChanged<bool>? recordingCallbackOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
          ?.onRecordingChanged;

  static ValueNotifier<bool>? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
      ?.active;

  @override
  bool updateShouldNotify(AgentNavigationOverlay oldWidget) =>
      active != oldWidget.active ||
      onRecordingChanged != oldWidget.onRecordingChanged;
}
