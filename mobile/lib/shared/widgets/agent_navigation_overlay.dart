import 'package:flutter/widgets.dart';

import 'agent_rail_link.dart';

/// Coordinates the expanded agent control with the shortcuts it covers.
class AgentNavigationOverlay extends InheritedWidget {
  const AgentNavigationOverlay({
    super.key,
    required this.active,
    this.onRecordingChanged,
    this.menuOpen = false,
    this.controlSize = defaultControlSize,
    this.railLink,
    required super.child,
  });
  final ValueNotifier<bool> active;
  final ValueChanged<bool>? onRecordingChanged;
  final bool menuOpen;

  /// Side of the agent control and the height of its recording pill.
  final double controlSize;

  static const double defaultControlSize = 56;

  static double controlSizeOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
          ?.controlSize ??
      defaultControlSize;

  /// Set when a native Duo rail control stands in for the agent shortcut.
  final AgentRailLink? railLink;

  static AgentRailLink? railLinkOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
      ?.railLink;

  static bool menuOpenOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<AgentNavigationOverlay>()
          ?.menuOpen ??
      false;

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
      menuOpen != oldWidget.menuOpen ||
      controlSize != oldWidget.controlSize ||
      railLink != oldWidget.railLink ||
      onRecordingChanged != oldWidget.onRecordingChanged;
}
