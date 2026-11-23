import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';

import 'ios_navigation_bar.dart';

/// Joins the agent shortcut to a native Duo rail control that stands in for it.
///
/// The shortcut stays mounted beside the rail, invisible while idle, so that
/// its recording logic and pill keep working. It publishes [action], which the
/// rail renders natively, and receives that control's press-and-hold through
/// [hold].
class AgentRailLink {
  /// The shortcut as a native rail control, once its state is known.
  final ValueNotifier<IosNavigationAction?> action = ValueNotifier(null);

  IosNavigationLongPressCallback? _holdHandler;

  /// Routes a native hold to the voice surface, if one is listening.
  void hold(IosNavigationLongPressPhase phase, Offset offsetFromOrigin) =>
      _holdHandler?.call(phase, offsetFromOrigin);

  /// Registers the voice surface; the returned callback unregisters it.
  VoidCallback attachHold(IosNavigationLongPressCallback handler) {
    _holdHandler = handler;
    return () {
      if (identical(_holdHandler, handler)) _holdHandler = null;
    };
  }
}
