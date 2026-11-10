import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'local_message_send_transition.dart';

/// Paints a local-send bubble from the captured composer rectangle.
/// The text travels with the bubble while keeping its final size and wrapping.
class LocalSendBubbleSurface extends SingleChildRenderObjectWidget {
  /// Creates an animated background without changing the child layout.
  const LocalSendBubbleSurface({
    super.key,
    required this.color,
    required this.shape,
    required super.child,
  });

  /// Bubble fill in the active theme.
  final Color color;

  /// Final conversation geometry, including grouped corners and thought dots.
  final ShapeBorder shape;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderSendBubble(color, shape, LocalSendProgress.scopeOf(context));

  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderProxyBox renderObject,
  ) {
    (renderObject as _RenderSendBubble).update(
      color,
      shape,
      LocalSendProgress.scopeOf(context),
    );
  }
}

class _RenderSendBubble extends RenderProxyBox {
  _RenderSendBubble(this.color, this.shape, this.motion);

  Color color;
  ShapeBorder shape;
  LocalSendProgress? motion;
  double get progress => motion?.progress ?? 1;

  void update(
    Color nextColor,
    ShapeBorder nextShape,
    LocalSendProgress? nextMotion,
  ) {
    if (color == nextColor && shape == nextShape && motion == nextMotion) {
      return;
    }
    color = nextColor;
    shape = nextShape;
    motion = nextMotion;
    markNeedsPaint();
    markNeedsSemanticsUpdate();
  }

  @override
  Rect get paintBounds {
    final width = constraints.hasBoundedWidth
        ? constraints.maxWidth
        : size.width;
    final bounds = shape
        .getOuterPath(Offset.zero & Size(width, size.height))
        .getBounds()
        .expandToInclude(super.paintBounds);
    return bounds
        .expandToInclude(animatedRect)
        .expandToInclude(child?.paintBounds.shift(childOffset) ?? Rect.zero);
  }

  Rect get animatedRect {
    final end = Offset.zero & size;
    final origin = motion?.origin;
    final start = origin == null
        ? Rect.fromLTWH(
            0,
            motion?.startOffset ?? 0,
            constraints.hasBoundedWidth ? constraints.maxWidth : size.width,
            size.height,
          )
        : origin.shift(-localToGlobal(Offset.zero));
    // Contract horizontally first, then finish lifting into the timeline.
    // Both tracks share a clock, but their overlapping curves differ.
    final horizontal = Rect.lerp(
      start,
      end,
      motion?.widthProgress ?? progress,
    )!;
    final vertical = Rect.lerp(start, end, progress)!;
    return Rect.fromLTWH(
      horizontal.left,
      vertical.top,
      horizontal.width,
      vertical.height,
    );
  }

  Offset get childOffset => progress < 1 ? animatedRect.topLeft : Offset.zero;

  @override
  void applyPaintTransform(RenderBox child, Matrix4 transform) {
    transform.translateByDouble(childOffset.dx, childOffset.dy, 0, 1);
  }

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    final rect = animatedRect;
    if (!shape.getOuterPath(rect).contains(position)) return false;
    if (!hitTestChildren(result, position: position)) return false;
    result.add(BoxHitTestEntry(this, position));
    return true;
  }

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      result.addWithPaintOffset(
        offset: childOffset,
        position: position,
        hitTest: (result, position) =>
            child?.hitTest(result, position: position) ?? false,
      );

  @override
  void paint(PaintingContext context, Offset offset) {
    final rect = animatedRect;
    final path = shape.getOuterPath(rect.shift(offset));
    context.canvas.drawPath(path, Paint()..color = color);
    if (child case final child?) {
      if (progress < 1) {
        context.pushClipPath(
          needsCompositing,
          Offset.zero,
          path.getBounds(),
          path,
          (context, _) => context.paintChild(child, offset + rect.topLeft),
        );
      } else {
        context.paintChild(child, offset);
      }
    }
  }
}
