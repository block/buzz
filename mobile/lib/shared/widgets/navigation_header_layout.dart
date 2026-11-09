import 'package:flutter/material.dart';
import 'buzz_navigation_metrics.dart';

/// Optional geometry for navigation hosted in a pane or beside an occlusion.
///
/// The host supplies geometry explicitly; ordinary phone routes use the default
/// UIKit layout. This layer does not detect hardware or create split panes.
@immutable
class NavigationHeaderLayout {
  const NavigationHeaderLayout({
    this.hardEdge = false,
    this.paneControls = false,
    this.titleInsets = EdgeInsets.zero,
    this.topSpacing = 0,
    this.controlCenterY,
    this.centerInVisibleHeight = false,
    this.alignCollapsedBottom = false,
    this.nativeEdgeClearance = 8,
    this.customEdgeClearance = 8,
  }) : assert(topSpacing >= 0),
       assert(nativeEdgeClearance >= 0),
       assert(customEdgeClearance >= 0);

  /// Minimum shared pane row, retaining the custom filter touch targets.
  double paneRowHeight(double compactHeight) =>
      (compactHeight.clamp(buzzNavigationActionSize, double.infinity) +
              topSpacing)
          .clamp(buzzNavigationRowHeight, double.infinity);

  /// Resolves an explicit axis within the protected content area.
  double? resolvedControlCenter(double safeTop, double rowHeight) =>
      controlCenterY?.clamp(safeTop + rowHeight / 2, double.infinity);

  final bool hardEdge;

  /// Places native titles and actions within this header's content bounds.
  final bool paneControls;
  final EdgeInsets titleInsets;

  /// Additional space above a pane's navigation row.
  final double topSpacing;

  /// Optional control axis in header-local coordinates, for camera alignment.
  /// The host converts any window occlusion geometry into the header space.
  /// Pane headers grow to contain the row around this axis, including hit bounds.
  /// Axes above the safe content area are clamped to half a control row below
  /// the top inset. Decorative clearance never extends the hit-test area.
  final double? controlCenterY;

  /// Centers collapsed controls within the visible hard backdrop, including
  /// its decorative lower extension, below the top safe-area inset. The
  /// automatic center is safeTop + (contentHeight + clearance) / 2, where
  /// contentHeight excludes safeTop. An explicit [controlCenterY] overrides
  /// this center in header-local coordinates. Expanded titles stay unchanged.
  /// Geometry is the same on older iOS versions with a softer material fallback.
  final bool centerInVisibleHeight;

  /// Centers a collapsing custom bottom row on the title/control row without
  /// changing the controls' dimensions.
  final bool alignCollapsedBottom;
  final double nativeEdgeClearance;
  final double customEdgeClearance;
}

/// Supplies navigation geometry to a route, including its sticky-date overlay.
class NavigationHeaderScope extends InheritedWidget {
  const NavigationHeaderScope({
    super.key,
    required this.layout,
    required super.child,
  });

  final NavigationHeaderLayout layout;

  static NavigationHeaderLayout of(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<NavigationHeaderScope>()
          ?.layout ??
      const NavigationHeaderLayout();

  @override
  bool updateShouldNotify(NavigationHeaderScope oldWidget) =>
      layout != oldWidget.layout;
}
