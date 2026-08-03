import 'dart:math' as math;
import 'dart:ui' show SemanticsRole;

import 'package:flutter/material.dart';

import '../theme/theme.dart';

const _popoverEnterDuration = Duration(milliseconds: 150);
const _popoverExitDuration = Duration(milliseconds: 110);
const _popoverStartScale = 0.96;

/// Elevation shared by anchored menus and composer popover surfaces.
const appPopoverElevation = 8.0;

/// Returns the translucent surface color shared by app popovers.
Color appPopoverColor(BuildContext context) =>
    context.colors.surface.withValues(alpha: 0.98);

/// Returns the shadow color shared by app popovers.
Color appPopoverShadowColor(BuildContext context) =>
    context.colors.shadow.withValues(alpha: 0.18);

/// Returns the 20px shape and composer-matching hairline shared by popovers.
RoundedRectangleBorder appPopoverShape(BuildContext context) =>
    RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(Radii.popover),
      side: BorderSide(color: Colors.black.withValues(alpha: 0.04), width: 1),
    );

/// The horizontal edge a popover aligns to on its triggering control.
enum AnchoredPopoverAlignment {
  /// Aligns the popover's leading edge with the trigger's leading edge.
  start,

  /// Aligns the popover's trailing edge with the trigger's trailing edge.
  end,
}

/// Shows an anchored, cross-platform popup menu with the Activity controls'
/// sizing, motion, and safe-area placement.
Future<T?> showAnchoredPopover<T>({
  required BuildContext context,
  required List<PopupMenuEntry<T>> items,
  required double width,
  required AnchoredPopoverAlignment alignment,
  Color? color,
  ShapeBorder? shape,
  double elevation = appPopoverElevation,
  Color? shadowColor,
  Offset offset = Offset.zero,
  EdgeInsetsGeometry menuPadding = EdgeInsets.zero,
  Clip clipBehavior = Clip.antiAlias,
  Key? surfaceKey,
}) {
  final navigator = Navigator.of(context);
  final overlay = navigator.overlay;
  final triggerRenderObject = context.findRenderObject();
  final overlayRenderObject = overlay?.context.findRenderObject();
  if (triggerRenderObject is! RenderBox || overlayRenderObject is! RenderBox) {
    return Future<T?>.value();
  }

  final triggerRect = MatrixUtils.transformRect(
    triggerRenderObject.getTransformTo(overlayRenderObject),
    Offset.zero & triggerRenderObject.size,
  );
  final overlayRect = Offset.zero & overlayRenderObject.size;
  final mediaQuery = MediaQuery.of(context);

  return navigator.push<T>(
    _AnchoredPopoverRoute<T>(
      position: RelativeRect.fromRect(triggerRect, overlayRect),
      items: items,
      width: width,
      alignment: alignment,
      offset: offset,
      color: color ?? appPopoverColor(context),
      shape: shape ?? appPopoverShape(context),
      elevation: elevation,
      shadowColor: shadowColor ?? appPopoverShadowColor(context),
      menuPadding: menuPadding,
      clipBehavior: clipBehavior,
      surfaceKey: surfaceKey,
      screenPadding: EdgeInsets.fromLTRB(
        math.max(Grid.xxs, mediaQuery.padding.left),
        math.max(Grid.xxs, mediaQuery.padding.top),
        math.max(Grid.xxs, mediaQuery.padding.right),
        math.max(Grid.xxs, mediaQuery.padding.bottom),
      ),
      reducedMotion: mediaQuery.disableAnimations,
      barrierLabel: MaterialLocalizations.of(context).modalBarrierDismissLabel,
    ),
  );
}

class _AnchoredPopoverRoute<T> extends PopupRoute<T> {
  final RelativeRect position;
  final List<PopupMenuEntry<T>> items;
  final double width;
  final AnchoredPopoverAlignment alignment;
  final Offset offset;
  final Color color;
  final ShapeBorder shape;
  final double elevation;
  final Color shadowColor;
  final EdgeInsetsGeometry menuPadding;
  final Clip clipBehavior;
  final Key? surfaceKey;
  final EdgeInsets screenPadding;
  final bool reducedMotion;
  final String _barrierLabel;

  _AnchoredPopoverRoute({
    required this.position,
    required this.items,
    required this.width,
    required this.alignment,
    required this.offset,
    required this.color,
    required this.shape,
    required this.elevation,
    required this.shadowColor,
    required this.menuPadding,
    required this.clipBehavior,
    required this.surfaceKey,
    required this.screenPadding,
    required this.reducedMotion,
    required String barrierLabel,
  }) : _barrierLabel = barrierLabel;

  @override
  Color? get barrierColor => null;

  @override
  bool get barrierDismissible => true;

  @override
  String? get barrierLabel => _barrierLabel;

  @override
  Duration get transitionDuration =>
      reducedMotion ? Duration.zero : _popoverEnterDuration;

  @override
  Duration get reverseTransitionDuration =>
      reducedMotion ? Duration.zero : _popoverExitDuration;

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) {
    final curvedAnimation = animation.drive(
      CurveTween(curve: Curves.easeOutCubic),
    );
    final scaleAnimation = Tween<double>(
      begin: _popoverStartScale,
      end: 1,
    ).animate(curvedAnimation);
    final transformOrigin = switch (alignment) {
      AnchoredPopoverAlignment.start => Alignment.topLeft,
      AnchoredPopoverAlignment.end => Alignment.topRight,
    };

    return CustomSingleChildLayout(
      delegate: _AnchoredPopoverLayoutDelegate(
        position: position,
        alignment: alignment,
        offset: offset,
        screenPadding: screenPadding,
      ),
      child: FadeTransition(
        key: const ValueKey('activity-popover-fade'),
        opacity: curvedAnimation,
        child: ScaleTransition(
          key: const ValueKey('activity-popover-scale'),
          scale: scaleAnimation,
          alignment: transformOrigin,
          child: Material(
            key: surfaceKey,
            type: MaterialType.card,
            color: color,
            surfaceTintColor: Colors.transparent,
            elevation: elevation,
            shadowColor: shadowColor,
            shape: shape,
            clipBehavior: clipBehavior,
            child: SizedBox(
              width: width,
              child: Semantics(
                role: SemanticsRole.menu,
                scopesRoute: true,
                namesRoute: true,
                explicitChildNodes: true,
                child: SingleChildScrollView(
                  padding: menuPadding,
                  child: ListBody(children: items),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _AnchoredPopoverLayoutDelegate extends SingleChildLayoutDelegate {
  final RelativeRect position;
  final AnchoredPopoverAlignment alignment;
  final Offset offset;
  final EdgeInsets screenPadding;

  const _AnchoredPopoverLayoutDelegate({
    required this.position,
    required this.alignment,
    required this.offset,
    required this.screenPadding,
  });

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) {
    return BoxConstraints.loose(
      Size(
        constraints.maxWidth - screenPadding.horizontal,
        constraints.maxHeight - screenPadding.vertical,
      ),
    );
  }

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    final anchorBottom = size.height - position.bottom;
    final desiredX = switch (alignment) {
      AnchoredPopoverAlignment.start => position.left + offset.dx,
      AnchoredPopoverAlignment.end =>
        size.width - position.right - childSize.width + offset.dx,
    };
    final minX = screenPadding.left;
    final maxX = size.width - screenPadding.right - childSize.width;
    final x = desiredX.clamp(minX, maxX).toDouble();

    final belowY = anchorBottom + offset.dy;
    final aboveY = position.top - childSize.height - offset.dy;
    final maxY = size.height - screenPadding.bottom - childSize.height;
    final desiredY =
        belowY + childSize.height <= size.height - screenPadding.bottom
        ? belowY
        : aboveY;
    final y = desiredY.clamp(screenPadding.top, maxY).toDouble();

    return Offset(x, y);
  }

  @override
  bool shouldRelayout(_AnchoredPopoverLayoutDelegate oldDelegate) {
    return position != oldDelegate.position ||
        alignment != oldDelegate.alignment ||
        offset != oldDelegate.offset ||
        screenPadding != oldDelegate.screenPadding;
  }
}
