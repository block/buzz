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
      child: _SheetContentWithCloseButton(child: builder(sheetContext)),
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
    showDragHandle: showDragHandle,
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

class _SheetContentWithCloseButton extends StatelessWidget {
  const _SheetContentWithCloseButton({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.only(
            top: Grid.gutter,
            right: Grid.gutter,
            bottom: Grid.xs,
          ),
          child: Align(
            alignment: Alignment.centerRight,
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
                  backgroundColor: context.colors.surfaceContainerHighest,
                  foregroundColor: context.colors.onSurface,
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(Radii.dialog),
                  ),
                ),
                icon: const Icon(LucideIcons.x, size: 22),
              ),
            ),
          ),
        ),
        Flexible(child: child),
      ],
    );
  }
}

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
