import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'concentric_sheet_surface.dart';

/// Native iOS glass with an opaque Material capsule on other platforms.
class NavigationButtonSurface extends StatelessWidget {
  const NavigationButtonSurface({
    required this.child,
    this.capsule = false,
    super.key,
  });

  final Widget child;

  /// Uses fully rounded ends instead of device-concentric corners.
  final bool capsule;

  @override
  Widget build(BuildContext context) {
    if (NavigationGlassGroup.maybeOf(context)) return child;
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return Material(
        color: context.controlSurface,
        shape: const StadiumBorder(),
        clipBehavior: Clip.antiAlias,
        child: child,
      );
    }
    return ConcentricSheetSurface(
      enabled: true,
      usesGlass: true,
      capsule: capsule,
      providesSheetSurface: false,
      padding: EdgeInsets.zero,
      minimumRadius: Radii.navigationCapsule,
      contentClipRadius: capsule ? 9999 : Radii.navigationCapsule,
      child: child,
    );
  }
}

/// Uses the shared native navigation glass instead of a separate surface.
class NavigationGlassGroup extends InheritedWidget {
  const NavigationGlassGroup({super.key, required super.child});

  static bool maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<NavigationGlassGroup>() !=
      null;

  @override
  bool updateShouldNotify(NavigationGlassGroup oldWidget) => false;
}
