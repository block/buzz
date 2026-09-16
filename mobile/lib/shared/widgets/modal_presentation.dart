import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'buzz_sheet_header.dart';
import 'buzz_titled_sheet_layout.dart';
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
/// handle inside the shared header so its spacing is consistent on every
/// platform.
Future<T?> showBuzzModalBottomSheet<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  String? title,
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
  // Explicitly colored sheets (for example the dark Huddle drawer) own their
  // complete surface treatment. Standard utility sheets use the quieter page
  // canvas and promote their controls onto the base app surface.
  final sheetTheme = backgroundColor == null
      ? utilitySurfaceThemeData(theme)
      : theme;
  final surfaceColor =
      backgroundColor ??
      sheetTheme.bottomSheetTheme.modalBackgroundColor ??
      sheetTheme.bottomSheetTheme.backgroundColor ??
      sheetTheme.colorScheme.surface;
  final reduceMotion = MediaQuery.disableAnimationsOf(context);

  return showModalBottomSheet<T>(
    context: context,
    builder: (sheetContext) => Theme(
      data: sheetTheme,
      child: Builder(
        builder: (themedContext) => ConcentricSheetSurface(
          enabled: isIos,
          color: surfaceColor,
          child: _SheetContent(
            title: title,
            showCloseButton: showCloseButton,
            showDragHandle: showDragHandle == true,
            surfaceColor: surfaceColor,
            child: builder(themedContext),
          ),
        ),
      ),
    ),
    backgroundColor: isIos || title != null ? Colors.transparent : surfaceColor,
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
    // The shared header owns the handle so Android does not reserve a second
    // handle band above the title row and iOS keeps it inside its native inset.
    showDragHandle: false,
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
    required this.title,
    required this.showCloseButton,
    required this.showDragHandle,
    required this.surfaceColor,
  });

  final Widget child;
  final String? title;
  final bool showCloseButton;
  final bool showDragHandle;
  final Color surfaceColor;

  @override
  Widget build(BuildContext context) {
    if (title == null || !showCloseButton) {
      return Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (showCloseButton)
            BuzzSheetHeader(title: title, showDragHandle: showDragHandle)
          else if (showDragHandle)
            const Padding(
              padding: EdgeInsets.only(top: Grid.xxs, bottom: Grid.xs),
              child: _StandaloneSheetDragHandle(),
            ),
          Flexible(child: child),
        ],
      );
    }

    return BuzzTitledSheetLayout(
      title: title!,
      showDragHandle: showDragHandle,
      surfaceColor: surfaceColor,
      child: child,
    );
  }
}

class _StandaloneSheetDragHandle extends StatelessWidget {
  const _StandaloneSheetDragHandle();

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: MaterialLocalizations.of(context).modalBarrierDismissLabel,
      container: true,
      button: true,
      onTap: () => Navigator.of(context).pop(),
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
