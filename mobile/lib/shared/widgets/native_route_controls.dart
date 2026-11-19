import 'bottom_up_page_route.dart';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Keeps native controls mounted for opaque sheets and their own exit motion.
/// Other covering Flutter routes suppress them to avoid platform-view bleed.
/// Must be called unconditionally from a hook widget's build method.
bool useNativeRouteControlsVisible(BuildContext context) {
  final route = ModalRoute.of(context);
  final coveringProgress = useAnimation(
    route?.secondaryAnimation ?? kAlwaysDismissedAnimation,
  );
  final sheets = route == null ? null : BottomUpPageRoute.sheetsAbove(route);
  useListenable(sheets);
  useListenable(route?.animation);
  if ((sheets?.value ?? 0) > 0) {
    return true;
  }
  final exiting =
      route != null &&
      !route.isActive &&
      route.animation?.status == AnimationStatus.reverse;
  return ((route?.isCurrent ?? true) || exiting) && coveringProgress == 0;
}
