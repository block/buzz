import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'collapsible_section_body.dart';

/// The section-body transition for a lazy, downward-scrolling vertical sliver.
///
/// Clips the section's extent instead of shrinking every row to zero, which
/// would force a lazy list to mount all its rows near the end of a collapse.
class CollapsibleSliverBody extends HookWidget {
  const CollapsibleSliverBody({
    required this.expanded,
    required this.sliver,
    super.key,
  });

  final bool expanded;
  final Widget sliver;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(initialValue: expanded ? 1 : 0);
    final animation = useMemoized(
      () => CurvedAnimation(
        parent: controller,
        curve: sectionExpandCurve,
        reverseCurve: sectionCollapseCurve,
      ),
      [controller],
    );
    useEffect(() => animation.dispose, [animation]);
    useEffect(() {
      controller.duration = sectionExpandDuration;
      controller.reverseDuration = sectionCollapseDuration;
      if (reducedMotion) {
        controller.value = expanded ? 1 : 0;
      } else if (expanded) {
        unawaited(controller.forward());
      } else {
        unawaited(controller.reverse());
      }
      return null;
    }, [controller, expanded, reducedMotion]);

    return AnimatedBuilder(
      animation: animation,
      child: sliver,
      builder: (context, child) {
        final value = animation.value.clamp(0.0, 1.0);
        if (value == 0) return const SliverToBoxAdapter();
        return SliverIgnorePointer(
          ignoring: !expanded,
          sliver: SliverFadeTransition(
            opacity: animation,
            sliver: _ClippedSectionSliver(
              factor: value,
              expanded: expanded,
              sliver: child!,
            ),
          ),
        );
      },
    );
  }
}

class _ClippedSectionSliver extends SingleChildRenderObjectWidget {
  const _ClippedSectionSliver({
    required this.factor,
    required this.expanded,
    required Widget sliver,
  }) : super(child: sliver);

  final double factor;
  final bool expanded;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderClippedSection(factor, expanded);

  @override
  void updateRenderObject(
    BuildContext context,
    covariant _RenderClippedSection renderObject,
  ) {
    renderObject
      ..factor = factor
      ..expanded = expanded;
  }
}

class _RenderClippedSection extends RenderProxySliver {
  _RenderClippedSection(this._factor, this._expanded);

  double _factor;
  set factor(double value) {
    if (_factor == value) return;
    _factor = value;
    markNeedsLayout();
  }

  bool _expanded;
  set expanded(bool value) {
    if (_expanded == value) return;
    _expanded = value;
    markNeedsSemanticsUpdate();
  }

  double get _scale =>
      sectionCollapsedScaleY + (1 - sectionCollapsedScaleY) * _factor;

  Matrix4 get _transform => Matrix4.identity()
    ..translateByDouble(0, -constraints.scrollOffset * (1 - _scale), 0, 1)
    ..scaleByDouble(1, _scale, 1, 1);

  @override
  void performLayout() {
    assert(constraints.axisDirection == AxisDirection.down);
    child!.layout(constraints, parentUsesSize: true);
    final full = child!.geometry!;
    if (full.scrollOffsetCorrection != null || _factor == 1) {
      geometry = full;
      return;
    }
    final extent = full.scrollExtent * _factor;
    final paint = math.min(
      full.paintExtent,
      calculatePaintOffset(constraints, from: 0, to: extent),
    );
    geometry = SliverGeometry(
      scrollExtent: extent,
      paintExtent: paint,
      layoutExtent: math.min(full.layoutExtent, paint),
      maxPaintExtent: full.maxPaintExtent * _factor,
      cacheExtent: math.min(
        full.cacheExtent,
        calculateCacheOffset(constraints, from: 0, to: extent),
      ),
      hitTestExtent: math.min(full.hitTestExtent, paint),
      hasVisualOverflow: full.hasVisualOverflow || _factor < 1,
    );
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    if (!geometry!.visible) return;
    if (_factor == 1) {
      super.paint(context, offset);
      return;
    }
    context.pushClipRect(
      needsCompositing,
      offset,
      Rect.fromLTWH(0, 0, constraints.crossAxisExtent, geometry!.paintExtent),
      (context, offset) => context.pushTransform(
        needsCompositing,
        offset,
        _transform,
        (context, offset) => context.paintChild(child!, offset),
      ),
    );
  }

  @override
  bool hitTestChildren(
    SliverHitTestResult result, {
    required double mainAxisPosition,
    required double crossAxisPosition,
  }) => BoxHitTestResult.wrap(result).addWithOutOfBandPosition(
    paintTransform: _transform,
    hitTest: (result) => child!.hitTest(
      SliverHitTestResult.wrap(result),
      mainAxisPosition:
          (mainAxisPosition + constraints.scrollOffset * (1 - _scale)) / _scale,
      crossAxisPosition: crossAxisPosition,
    ),
  );

  @override
  void applyPaintTransform(RenderObject child, Matrix4 transform) {
    transform.multiply(_transform);
  }

  @override
  Rect? describeApproximatePaintClip(covariant RenderObject child) =>
      _factor == 1
      ? null
      : Rect.fromLTWH(0, 0, constraints.crossAxisExtent, geometry!.paintExtent);

  @override
  void visitChildrenForSemantics(RenderObjectVisitor visitor) {
    if (_expanded) super.visitChildrenForSemantics(visitor);
  }
}
