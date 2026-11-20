import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Grows a message row upward while keeping the child's bottom anchored.
class LocalSendRow extends SingleChildRenderObjectWidget {
  /// Creates a row driven by the same progress as its bubble entrance.
  const LocalSendRow({super.key, required this.progress, required super.child});

  /// Fraction of the final row height allocated in the conversation.
  final double progress;
  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderSendRow(progress);
  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderProxyBox renderObject,
  ) {
    (renderObject as _RenderSendRow).setProgress(progress);
  }
}

class _RenderSendRow extends RenderProxyBox {
  _RenderSendRow(this.progress);
  double progress;
  void setProgress(double value) {
    if (progress == value) return;
    progress = value;
    markNeedsLayout();
    markNeedsSemanticsUpdate();
  }

  Offset get childOffset => Offset(0, size.height - (child?.size.height ?? 0));
  @override
  void performLayout() {
    child?.layout(constraints, parentUsesSize: true);
    // A zero-extent item can be culled by the indexed list before the lift
    // begins, briefly removing its avatar. Keep it laid out from the first frame.
    size = constraints.constrain(
      Size(
        child?.size.width ?? 0,
        (child?.size.height ?? 0) * progress.clamp(0.0001, 1.0),
      ),
    );
  }

  @override
  Rect get paintBounds => super.paintBounds.expandToInclude(
    child?.paintBounds.shift(childOffset) ?? Rect.zero,
  );

  @override
  void paint(PaintingContext context, Offset offset) {
    if (child case final child?) {
      context.paintChild(child, offset + childOffset);
    }
  }

  @override
  void applyPaintTransform(RenderBox child, Matrix4 transform) {
    transform.translateByDouble(childOffset.dx, childOffset.dy, 0, 1);
  }

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    if (!paintBounds.contains(position)) return false;
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
}
