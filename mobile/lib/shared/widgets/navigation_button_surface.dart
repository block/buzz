import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'concentric_sheet_surface.dart';

/// Native iOS glass with an opaque Material capsule on other platforms.
class NavigationButtonSurface extends StatelessWidget {
  const NavigationButtonSurface({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context) {
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
      providesSheetSurface: false,
      padding: EdgeInsets.zero,
      minimumRadius: Radii.navigationCapsule,
      contentClipRadius: Radii.navigationCapsule,
      child: child,
    );
  }
}
