import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';
import 'concentric_sheet_surface.dart';

/// Shared motion for occasional modal UI.
///
/// The strong ease-out makes entrances respond immediately, while the shorter
/// exit keeps dismissals from feeling sluggish.
const buzzModalAnimationStyle = AnimationStyle(
  curve: Cubic(0.23, 1, 0.32, 1),
  duration: Duration(milliseconds: 280),
  reverseCurve: Cubic(0.77, 0, 0.175, 1),
  reverseDuration: Duration(milliseconds: 220),
);

/// Shows a bottom sheet with Buzz's shared motion and sheet chrome.
///
/// Sheets include the shared close control by default. On iOS, the surface
/// uses native concentric corners when available and paints a requested drag
/// handle inside that inset surface; other platforms retain Flutter's handle.
Future<T?> showBuzzModalBottomSheet<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  Color? backgroundColor,
  String? barrierLabel,
  double? elevation,
  ShapeBorder? shape,
  Clip? clipBehavior,
  BoxConstraints? constraints,
  Color? barrierColor,
  bool isScrollControlled = false,
  double scrollControlDisabledMaxHeightRatio = 9.0 / 16.0,
  bool useRootNavigator = false,
  bool isDismissible = true,
  bool enableDrag = true,
  bool? showDragHandle,
  bool showCloseButton = true,
  bool useSafeArea = false,
  RouteSettings? routeSettings,
  AnimationController? transitionAnimationController,
  Offset? anchorPoint,
  AnimationStyle? sheetAnimationStyle,
  bool? requestFocus,
}) {
  final isIos = defaultTargetPlatform == TargetPlatform.iOS;
  final theme = Theme.of(context);
  final surfaceColor =
      backgroundColor ??
      theme.bottomSheetTheme.modalBackgroundColor ??
      theme.bottomSheetTheme.backgroundColor ??
      context.colors.surface;
  final reduceMotion = MediaQuery.disableAnimationsOf(context);

  return showModalBottomSheet<T>(
    context: context,
    builder: (sheetContext) => ConcentricSheetSurface(
      enabled: isIos,
      color: surfaceColor,
      child: _SheetContent(
        showCloseButton: showCloseButton,
        showDragHandle: isIos && showDragHandle == true,
        child: builder(sheetContext),
      ),
    ),
    backgroundColor: isIos ? Colors.transparent : backgroundColor,
    barrierLabel: barrierLabel,
    elevation: elevation,
    shape: shape,
    clipBehavior: clipBehavior,
    constraints: constraints,
    barrierColor: barrierColor,
    isScrollControlled: isScrollControlled,
    scrollControlDisabledMaxHeightRatio: scrollControlDisabledMaxHeightRatio,
    useRootNavigator: useRootNavigator,
    isDismissible: isDismissible,
    enableDrag: enableDrag,
    // The iOS route is transparent so its stock handle sits outside the inset
    // concentric surface. Paint it inside the surface above instead.
    showDragHandle: isIos ? false : showDragHandle,
    useSafeArea: useSafeArea,
    routeSettings: routeSettings,
    transitionAnimationController: transitionAnimationController,
    anchorPoint: anchorPoint,
    sheetAnimationStyle: reduceMotion
        ? AnimationStyle.noAnimation
        : (sheetAnimationStyle ?? buzzModalAnimationStyle),
    requestFocus: requestFocus,
  );
}

class _SheetContent extends StatelessWidget {
  const _SheetContent({
    required this.child,
    required this.showCloseButton,
    required this.showDragHandle,
  });

  final Widget child;
  final bool showCloseButton;
  final bool showDragHandle;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (showCloseButton)
          Padding(
            padding: const EdgeInsets.only(
              top: Grid.xxs,
              left: Grid.gutter,
              right: Grid.gutter,
              bottom: Grid.xs,
            ),
            child: SizedBox(
              height: 56,
              child: Stack(
                alignment: Alignment.topCenter,
                children: [
                  if (showDragHandle) const _SheetDragHandle(),
                  Align(
                    alignment: Alignment.bottomRight,
                    child: SizedBox.square(
                      dimension: 44,
                      child: IconButton(
                        tooltip: 'Close sheet',
                        onPressed: () {
                          unawaited(HapticFeedback.lightImpact());
                          Navigator.of(context).pop();
                        },
                        style: IconButton.styleFrom(
                          padding: EdgeInsets.zero,
                          backgroundColor:
                              context.colors.surfaceContainerHighest,
                          foregroundColor: context.colors.onSurface,
                          shape: RoundedRectangleBorder(
                            borderRadius: BorderRadius.circular(Radii.dialog),
                          ),
                        ),
                        icon: const Icon(LucideIcons.x, size: 22),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          )
        else if (showDragHandle)
          const Padding(
            padding: EdgeInsets.only(top: Grid.xxs, bottom: Grid.xs),
            child: _SheetDragHandle(),
          ),
        Flexible(child: child),
      ],
    );
  }
}

class _SheetDragHandle extends StatelessWidget {
  const _SheetDragHandle();

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'Drag handle',
      child: Container(
        key: const ValueKey('buzz-sheet-drag-handle'),
        width: 32,
        height: 4,
        decoration: BoxDecoration(
          color: context.colors.onSurfaceVariant.withValues(alpha: 0.4),
          borderRadius: BorderRadius.circular(Radii.full),
        ),
      ),
    );
  }
}

/// Shows a dialog with Buzz's shared motion, respecting reduced-motion settings.
Future<T?> showBuzzDialog<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  bool barrierDismissible = true,
  Color? barrierColor,
  String? barrierLabel,
  bool useSafeArea = true,
  bool useRootNavigator = true,
  RouteSettings? routeSettings,
  Offset? anchorPoint,
  TraversalEdgeBehavior? traversalEdgeBehavior,
  bool fullscreenDialog = false,
  bool? requestFocus,
  AnimationStyle? animationStyle,
}) {
  final reduceMotion = MediaQuery.disableAnimationsOf(context);

  return showDialog<T>(
    context: context,
    builder: builder,
    barrierDismissible: barrierDismissible,
    barrierColor: barrierColor,
    barrierLabel: barrierLabel,
    useSafeArea: useSafeArea,
    useRootNavigator: useRootNavigator,
    routeSettings: routeSettings,
    anchorPoint: anchorPoint,
    traversalEdgeBehavior: traversalEdgeBehavior,
    fullscreenDialog: fullscreenDialog,
    requestFocus: requestFocus,
    animationStyle: reduceMotion
        ? AnimationStyle.noAnimation
        : (animationStyle ?? buzzModalAnimationStyle),
  );
}
