import 'bottom_up_page_route.dart';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Keeps native controls mounted for opaque sheets and their own exit motion.
/// Other covering Flutter routes suppress them to avoid platform-view bleed.
/// Used by Home's native-controls gate, the Channels route-currency sync and
/// `IosMessageCreateButton`. Rebuilds only when the visibility inputs change,
/// not on every animation frame. Must be called unconditionally from a hook
/// widget's build method.
bool useNativeRouteControlsVisible(BuildContext context) {
  final route = ModalRoute.of(context);
  final uncovered = useListenableSelector(
    route?.secondaryAnimation,
    () => (route?.secondaryAnimation?.value ?? 0) == 0,
  );
  final reversing = useListenableSelector(
    route?.animation,
    () => route?.animation?.status == AnimationStatus.reverse,
  );
  final sheets = route == null ? null : BottomUpPageRoute.sheetsAbove(route);
  useListenable(sheets);
  if ((sheets?.value ?? 0) > 0) {
    return true;
  }
  final exiting = route != null && !route.isActive && reversing;
  return ((route?.isCurrent ?? true) || exiting) && uncovered;
}
