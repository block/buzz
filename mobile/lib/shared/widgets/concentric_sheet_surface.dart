import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../theme/theme.dart';

/// An iOS-native sheet surface that adopts the system's concentric corners on
/// iOS 26 and newer. Other platforms keep the normal Flutter shape.
class ConcentricSheetSurface extends StatelessWidget {
  const ConcentricSheetSurface({
    required this.child,
    required this.enabled,
    this.color,
    super.key,
  });

  final Widget child;
  final bool enabled;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    if (!enabled) {
      return child;
    }

    return Padding(
      padding: const EdgeInsets.only(
        left: Grid.xxs,
        right: Grid.xxs,
        bottom: Grid.xxs,
      ),
      child: Stack(
        children: [
          Positioned.fill(
            child: IgnorePointer(
              child: UiKitView(
                viewType: 'buzz/concentric_sheet_surface',
                hitTestBehavior: PlatformViewHitTestBehavior.transparent,
                creationParams: <String, Object>{
                  'color': (color ?? context.colors.surface).toARGB32(),
                  'minimumRadius': Radii.dialog,
                },
                creationParamsCodec: const StandardMessageCodec(),
              ),
            ),
          ),
          child,
        ],
      ),
    );
  }
}
