import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'anchored_popover_menu.dart';

const _arrowHeight = Grid.xxs;
const _arrowWidth = Grid.xs;
const _targetGap = Grid.half;

/// A non-blocking tip that points down at [child] while [visible].
///
/// The tip floats above everything else on the current route without a scrim,
/// so the screen underneath stays usable. Any tap, on the tip or elsewhere,
/// calls [onDismiss]; so does a screen reader's dismiss action. The tip hides
/// while another route covers this one.
class BuzzCallout extends HookWidget {
  const BuzzCallout({
    required this.message,
    required this.visible,
    required this.onDismiss,
    required this.child,
    super.key,
  });

  final String message;
  final bool visible;
  final VoidCallback onDismiss;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final showing = visible && (ModalRoute.of(context)?.isCurrent ?? true);
    final controller = useMemoized(OverlayPortalController.new);
    useEffect(() {
      // The controller can't change during a build, so it follows a frame
      // behind; the builder below hides the tip in the meantime.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || showing == controller.isShowing) return;
        showing ? controller.show() : controller.hide();
      });
      return null;
    }, [showing]);
    return OverlayPortal.overlayChildLayoutBuilder(
      controller: controller,
      overlayChildBuilder: (context, info) {
        if (!showing) return const SizedBox.shrink();
        final target = MatrixUtils.transformRect(
          info.childPaintTransform,
          Offset.zero & info.childSize,
        );
        final padding = MediaQuery.paddingOf(context);
        return _CalloutPlacement(
          target: target,
          margin: EdgeInsets.only(
            left: padding.left + Grid.xs,
            right: padding.right + Grid.xs,
            top: padding.top + Grid.xs,
          ),
          // Inverse colors set the tip apart from menus and the page.
          color: context.colors.inverseSurface,
          shadowColor: appPopoverShadowColor(context),
          child: TapRegion(
            onTapInside: (_) => onDismiss(),
            onTapOutside: (_) => onDismiss(),
            child: Semantics(
              container: true,
              liveRegion: true,
              onDismiss: onDismiss,
              onTap: onDismiss,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.xs,
                  vertical: Grid.twelve,
                ),
                child: Text(
                  message,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: context.colors.onInverseSurface,
                  ),
                ),
              ),
            ),
          ),
        );
      },
      child: child,
    );
  }
}

/// Fills the overlay, placing its child above [target] inside [margin] and
/// painting one surface for the bubble and its arrow. Only the bubble is
/// hit-testable, so the rest of the screen keeps receiving touches.
class _CalloutPlacement extends SingleChildRenderObjectWidget {
  const _CalloutPlacement({
    required this.target,
    required this.margin,
    required this.color,
    required this.shadowColor,
    required super.child,
  });

  final Rect target;
  final EdgeInsets margin;
  final Color color;
  final Color shadowColor;

  @override
  _RenderCalloutPlacement createRenderObject(BuildContext context) =>
      _RenderCalloutPlacement(target, margin, color, shadowColor);

  @override
  void updateRenderObject(
    BuildContext context,
    _RenderCalloutPlacement renderObject,
  ) {
    renderObject
      ..target = target
      ..margin = margin
      ..color = color
      ..shadowColor = shadowColor;
  }
}

class _RenderCalloutPlacement extends RenderShiftedBox {
  _RenderCalloutPlacement(
    this._target,
    this._margin,
    this._color,
    this._shadowColor,
  ) : super(null);

  Rect _target;
  set target(Rect value) {
    if (value == _target) return;
    _target = value;
    markNeedsLayout();
  }

  EdgeInsets _margin;
  set margin(EdgeInsets value) {
    if (value == _margin) return;
    _margin = value;
    markNeedsLayout();
  }

  Color _color;
  set color(Color value) {
    if (value == _color) return;
    _color = value;
    markNeedsPaint();
  }

  Color _shadowColor;
  set shadowColor(Color value) {
    if (value == _shadowColor) return;
    _shadowColor = value;
    markNeedsPaint();
  }

  Path _path = Path();

  @override
  void performLayout() {
    size = constraints.biggest;
    final child = this.child;
    if (child == null) return;
    final bottom = _target.top - _targetGap - _arrowHeight;
    child.layout(
      BoxConstraints(
        maxWidth: math.max(0, size.width - _margin.horizontal),
        maxHeight: math.max(0, bottom - _margin.top),
      ),
      parentUsesSize: true,
    );
    final left = (_target.center.dx - child.size.width / 2)
        .clamp(
          _margin.left,
          math.max(_margin.left, size.width - _margin.right - child.size.width),
        )
        .toDouble();
    final offset = Offset(left, bottom - child.size.height);
    (child.parentData! as BoxParentData).offset = offset;

    final bubble = offset & child.size;
    final radius = math.min(Radii.popover, bubble.shortestSide / 2);
    // Keep the arrow's base clear of the rounded corners.
    final arrowX = _target.center.dx
        .clamp(
          bubble.left + radius + _arrowWidth / 2,
          math.max(
            bubble.left + radius + _arrowWidth / 2,
            bubble.right - radius - _arrowWidth / 2,
          ),
        )
        .toDouble();
    // One outline, so the shadow doesn't cross where the arrow joins.
    _path = Path.combine(
      PathOperation.union,
      Path()
        ..addRRect(RRect.fromRectAndRadius(bubble, Radius.circular(radius))),
      Path()..addPolygon([
        Offset(arrowX - _arrowWidth / 2, bubble.bottom - 1),
        Offset(arrowX, bubble.bottom + _arrowHeight),
        Offset(arrowX + _arrowWidth / 2, bubble.bottom - 1),
      ], true),
    );
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final child = this.child;
    if (child == null) return;
    final path = _path.shift(offset);
    final canvas = context.canvas;
    canvas.drawShadow(path, _shadowColor, appPopoverElevation, _color.a < 1);
    canvas.drawPath(path, Paint()..color = _color);
    super.paint(context, offset);
  }

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) =>
      hitTestChildren(result, position: position);
}
