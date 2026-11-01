import 'dart:ui' show ImageFilter;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'concentric_sheet_surface.dart';

/// Native Liquid Glass on iOS; the translucent Flutter surface elsewhere.
class ExpandingMenuSurface extends StatelessWidget {
  const ExpandingMenuSurface({
    required this.progress,
    required this.child,
    super.key,
  });

  final double progress;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return ConcentricSheetSurface(
        enabled: true,
        usesGlass: true,
        providesSheetSurface: false,
        padding: EdgeInsets.zero,
        color: context.controlSurface,
        minimumRadius: Radii.navigationCapsule,
        contentClipRadius: Radii.navigationCapsule,
        child: Material(type: MaterialType.transparency, child: child),
      );
    }
    return BackdropFilter(
      filter: ImageFilter.blur(sigmaX: 16 * progress, sigmaY: 16 * progress),
      child: Material(
        color: context.controlSurface.withValues(alpha: 1 - .12 * progress),
        child: child,
      ),
    );
  }
}
