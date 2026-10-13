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
    final origin = motion?.origin;
    return progress < 1 && origin != null
        ? bounds.expandToInclude(origin.shift(-localToGlobal(Offset.zero)))
        : bounds;
  }

  @override
  void paint(PaintingContext context, Offset offset) {
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
    final rect = Rect.fromLTWH(
      horizontal.left,
      vertical.top,
      horizontal.width,
      vertical.height,
    );
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
