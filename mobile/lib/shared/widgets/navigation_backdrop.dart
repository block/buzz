import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'ios_navigation_bar.dart';

/// The shared top-edge treatment, independent of a header's controls.
///
/// iOS uses the same native soft edge as [IosNavigationBar]. Other platforms
/// use Home's translucent Flutter blur. The child remains interactive and
/// supplies its own safe-area padding.
class NavigationBackdrop extends StatelessWidget {
  const NavigationBackdrop({
    super.key,
    required this.child,
    this.active = true,
    this.suppressNativeView = false,
  });

  final Widget child;

  /// Activates the Flutter blur; UIKit owns native scroll-edge visibility.
  final bool active;

  /// Flutter backdrop transitions cannot composite over a native platform view.
  final bool suppressNativeView;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS && !suppressNativeView) {
      return Stack(
        children: [
          Positioned.fill(
            child: IgnorePointer(
              child: ExcludeSemantics(
                child: IosNavigationBar(
                  title: '',
                  backdropOnly: true,
                  alwaysFrosted: true,
                ),
              ),
            ),
          ),
          child,
        ],
      );
    }
    return ClipRect(
      child: BackdropFilter(
        enabled: active,
        filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
        child: ColoredBox(
          color: active
              ? context.colors.surface.withValues(alpha: 0.45)
              : Colors.transparent,
          child: child,
        ),
      ),
    );
  }
}
