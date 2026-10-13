import 'dart:ui' show lerpDouble;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'local_message_send_transition.dart';

/// Paints a local-send bubble from the available line width to its final width.
/// Text keeps its final layout throughout: only the background changes shape.
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
      _RenderSendBubble(color, shape, LocalSendProgress.of(context));

  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderProxyBox renderObject,
  ) {
    (renderObject as _RenderSendBubble).update(
      color,
      shape,
      LocalSendProgress.of(context),
    );
  }
}

class _RenderSendBubble extends RenderProxyBox {
  _RenderSendBubble(this.color, this.shape, this.progress);

  Color color;
  ShapeBorder shape;
  double progress;

  void update(Color nextColor, ShapeBorder nextShape, double nextProgress) {
    if (color == nextColor && shape == nextShape && progress == nextProgress) {
      return;
    }
    color = nextColor;
    shape = nextShape;
    progress = nextProgress;
    markNeedsPaint();
  }

  @override
  Rect get paintBounds {
    final width = constraints.hasBoundedWidth
        ? constraints.maxWidth
        : size.width;
    return shape
        .getOuterPath(Offset.zero & Size(width, size.height))
        .getBounds()
        .expandToInclude(super.paintBounds);
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final startWidth = constraints.hasBoundedWidth
        ? constraints.maxWidth
        : size.width;
    final width = lerpDouble(startWidth, size.width, progress)!;
    final rect = offset & Size(width, size.height);
    context.canvas.drawPath(shape.getOuterPath(rect), Paint()..color = color);
    super.paint(context, offset);
  }
}
