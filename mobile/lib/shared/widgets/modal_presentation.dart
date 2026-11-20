import 'package:flutter/foundation.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'buzz_sheet_header.dart';
import 'duo_toolbar.dart';
import 'ios_navigation_bar.dart';
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
/// Set [useDuoFullWidth] to false for a bounded sheet with local controls
/// in the nearest navigator (unless [useRootNavigator] is explicitly true).
/// Set [padHorizontal] to false when the content paints behind the Duo rail
/// and reserves its own horizontal safe area.
/// Set [padBottom] to false when the content owns its bottom edge spacing.
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
  bool useDuoFullWidth = true,
  bool isDismissible = true,
  bool enableDrag = true,
  bool? showDragHandle,
  bool showCloseButton = true,
  bool showRailCloseButton = false,
  bool useSafeArea = false,
  bool padBottom = true,
  bool padHorizontal = true,
  RouteSettings? routeSettings,
  AnimationController? transitionAnimationController,
  Offset? anchorPoint,
  AnimationStyle? sheetAnimationStyle,
  bool? requestFocus,
}) {
  final isIos = defaultTargetPlatform == TargetPlatform.iOS;
  final startsFullWidth =
      useDuoFullWidth &&
      (DuoToolbarScope.maybeOf(context)?.strip ??
              FoldGeometry.of(context).strip) !=
          null;
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
    builder: (sheetContext) {
      // Read the current window on every posture change. Keeping these values
      // outside the builder leaves Close owned by a rail that may disappear.
      final strip =
          DuoToolbarScope.maybeOf(sheetContext)?.strip ??
          FoldGeometry.of(sheetContext).strip;
      final fullWidth = strip != null && useDuoFullWidth;
      final content = _SheetLocalSafeArea(
        localize: strip != null && !fullWidth,
        child: DuoToolbarScope(
          strip: strip,
          active: true,
          child: DuoToolbarPortal(
            strip: isIos && fullWidth ? strip : null,
            active: ModalRoute.isCurrentOf(sheetContext) ?? true,
            actions: [
              if (showCloseButton || showRailCloseButton)
                IosNavigationAction(
                  label: 'Close ${title ?? 'sheet'}',
                  symbol: 'xmark',
                  onPressed: () => Navigator.of(sheetContext).pop(),
                ),
            ],
            child: DuoSheetCloseScope(
              inRail:
                  isIos &&
                  fullWidth &&
                  (showCloseButton || showRailCloseButton),
              child: Theme(
                data: sheetTheme,
                child: Builder(
                  builder: (themedContext) => ConcentricSheetSurface(
                    enabled: isIos,
                    padding: fullWidth
                        ? EdgeInsets.zero
                        : const EdgeInsets.only(
                            left: Grid.xxs,
                            right: Grid.xxs,
                            bottom: Grid.xxs,
                          ),
                    color: surfaceColor,
                    child: SafeArea(
                      top: false,
                      bottom: false,
                      left: fullWidth && padHorizontal,
                      right: fullWidth && padHorizontal,
                      child: _SheetContent(
                        title: title,
                        showCloseButton: showCloseButton,
                        showDragHandle: showDragHandle == true,
                        surfaceColor: surfaceColor,
                        // Keep the entire scroll viewport above gesture navigation. The
                        // inner builder sees a consumed bottom inset, so sheet-local safe
                        // areas and scroll padding do not reserve that system inset twice.
                        child: SafeArea(
                          top: false,
                          left: false,
                          right: false,
                          bottom: padBottom,
                          child: Padding(
                            padding: EdgeInsets.only(
                              bottom: padBottom ? Grid.half : 0,
                            ),
                            child: Builder(builder: builder),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      if (!isIos || !useDuoFullWidth) return content;
      // The route cannot change useSafeArea after presentation. Apply it here
      // so rail-owned sheets can use the full window after unfolding.
      final media = MediaQuery.of(sheetContext);
      final window = MediaQueryData.fromView(View.of(sheetContext));
      return MediaQuery(
        data: media.copyWith(
          padding: media.padding.copyWith(
            top: useSafeArea && !fullWidth
                ? window.padding.top
                : media.padding.top,
          ),
        ),
        child: SafeArea(
          top: useSafeArea && !fullWidth,
          left: useSafeArea && !fullWidth,
          right: useSafeArea && !fullWidth,
          bottom: false,
          child: Align(
            alignment: Alignment.bottomCenter,
            widthFactor: 1,
            heightFactor: 1,
            child: ConstrainedBox(
              constraints:
                  constraints ??
                  (fullWidth
                      ? const BoxConstraints(maxWidth: double.infinity)
                      : sheetTheme.bottomSheetTheme.constraints ??
                            BoxConstraints(
                              maxWidth: sheetTheme.useMaterial3
                                  ? 640
                                  : double.infinity,
                            )),
              child: content,
            ),
          ),
        ),
      );
    },
    backgroundColor: isIos || title != null ? Colors.transparent : surfaceColor,
    barrierLabel: barrierLabel,
    elevation: elevation,
    shape: shape,
    clipBehavior: clipBehavior,
    // The route must permit expansion; the builder owns the live width cap.
    constraints: isIos && useDuoFullWidth
        ? const BoxConstraints(maxWidth: double.infinity)
        : constraints,
    barrierColor: barrierColor,
    isScrollControlled: isScrollControlled,
    scrollControlDisabledMaxHeightRatio: scrollControlDisabledMaxHeightRatio,
    useRootNavigator:
        (isIos && useDuoFullWidth) || startsFullWidth || useRootNavigator,
    isDismissible: isDismissible,
    enableDrag: enableDrag,
    // The shared header owns the handle so Android does not reserve a second
    // handle band above the title row and iOS keeps it inside its native inset.
    showDragHandle: false,
    useSafeArea: isIos && useDuoFullWidth ? false : useSafeArea,
    routeSettings: routeSettings,
    transitionAnimationController: transitionAnimationController,
    anchorPoint: anchorPoint,
    sheetAnimationStyle: reduceMotion
        ? AnimationStyle.noAnimation
        : (sheetAnimationStyle ?? buzzModalAnimationStyle),
    requestFocus: requestFocus,
  );
}

/// Window insets only apply where they intersect a centered sheet's bounds.
class _SheetLocalSafeArea extends StatelessWidget {
  const _SheetLocalSafeArea({required this.localize, required this.child});

  final bool localize;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!localize) return child;
    return LayoutBuilder(
      builder: (context, constraints) {
        final media = MediaQuery.of(context);
        final margin = (media.size.width - constraints.maxWidth) / 2;
        EdgeInsets localInsets(EdgeInsets insets) => insets.copyWith(
          left: (insets.left - margin).clamp(0.0, double.infinity),
          right: (insets.right - margin).clamp(0.0, double.infinity),
        );
        return DuoToolbarScope(
          strip: null,
          active: true,
          child: MediaQuery(
            data: media.copyWith(
              padding: localInsets(media.padding),
              viewPadding: localInsets(media.viewPadding),
            ),
            child: child,
          ),
        );
      },
    );
  }
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
