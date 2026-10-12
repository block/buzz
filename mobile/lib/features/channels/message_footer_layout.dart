import 'dart:math' as math;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../../shared/theme/grid.dart';

/// Places a footer beside one-line content, below it, or overlapping its edge.
/// Both children remain inside the hit-test bounds, including overlapping badges.
class MessageFooterLayout extends MultiChildRenderObjectWidget {
  MessageFooterLayout({
    super.key,
    required Widget child,
    required Widget footer,
    this.inlineLineHeight,
    this.overlap = 0,
  }) : super(children: [child, footer]);

  /// Maximum rendered content height eligible for an inline footer.
  final double? inlineLineHeight;

  /// Space shared by the content's bottom edge and the footer.
  final double overlap;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMessageFooter(inlineLineHeight, overlap);

  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderBox renderObject,
  ) {
    (renderObject as _RenderMessageFooter)
      ..lineHeight = inlineLineHeight
      ..overlap = overlap
      ..markNeedsLayout();
  }
}

class _RenderMessageFooter extends RenderBox
    with
        ContainerRenderObjectMixin<
          RenderBox,
          ContainerBoxParentData<RenderBox>
        >,
        RenderBoxContainerDefaultsMixin<
          RenderBox,
          ContainerBoxParentData<RenderBox>
        > {
  _RenderMessageFooter(this.lineHeight, this.overlap);

  double? lineHeight;
  double overlap;

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! ContainerBoxParentData<RenderBox>) {
      child.parentData = _FooterParentData();
    }
  }

  Size _layout(BoxConstraints constraints, {required bool dry}) {
    final content = firstChild!;
    final footer = lastChild!;
    final loose = constraints.loosen();
    Size measure(RenderBox child) {
      if (dry) return child.getDryLayout(loose);
      child.layout(loose, parentUsesSize: true);
      return child.size;
    }

    final contentSize = measure(content);
    final footerSize = measure(footer);
    const gap = Grid.xxs;
    final inline =
        lineHeight != null &&
        contentSize.height <= lineHeight! + 1 &&
        contentSize.width + gap + footerSize.width <= constraints.maxWidth;
    final width = inline
        ? contentSize.width + gap + footerSize.width
        : math.max(contentSize.width, footerSize.width);
    final footerOffset = inline
        ? Offset(
            contentSize.width + gap,
            math.max(0, contentSize.height - footerSize.height),
          )
        : Offset(
            lineHeight == null ? 0 : width - footerSize.width,
            math.max(0, contentSize.height - overlap),
          );
    if (!dry) {
      (content.parentData! as ContainerBoxParentData<RenderBox>).offset =
          Offset.zero;
      (footer.parentData! as ContainerBoxParentData<RenderBox>).offset =
          footerOffset;
    }
    return constraints.constrain(
      Size(
        width,
        math.max(contentSize.height, footerOffset.dy + footerSize.height),
      ),
    );
  }

  @override
  Size computeDryLayout(BoxConstraints constraints) =>
      _layout(constraints, dry: true);

  @override
  void performLayout() => size = _layout(constraints, dry: false);

  @override
  void paint(PaintingContext context, Offset offset) =>
      defaultPaint(context, offset);

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      defaultHitTestChildren(result, position: position);
}

class _FooterParentData extends ContainerBoxParentData<RenderBox> {}
