import 'dart:math' as math;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Places reactions at the bottom edge of a message.
/// Both children remain inside the hit-test bounds, including overlapping badges.
class MessageFooterLayout extends StatelessWidget {
  /// Creates a message with overlapping reaction controls.
  const MessageFooterLayout({
    super.key,
    required this.child,
    required this.footer,
    this.overlap = 0,
    this.animateSize = false,
  });

  /// Main message content.
  final Widget child;

  /// Controls attached to the bottom of the message.
  final Widget footer;

  /// Vertical distance shared by the message and its controls.
  final double overlap;

  /// Smoothly adjusts the space occupied by added or removed reactions.
  final bool animateSize;

  @override
  Widget build(BuildContext context) {
    final layout = _FooterLayout(overlap: overlap, children: [child, footer]);
    if (!animateSize || MediaQuery.disableAnimationsOf(context)) return layout;
    return AnimatedSize(
      duration: const Duration(milliseconds: 200),
      curve: Curves.easeInOutCubic,
      alignment: Alignment.topLeft,
      clipBehavior: Clip.none,
      child: layout,
    );
  }
}

class _FooterLayout extends MultiChildRenderObjectWidget {
  const _FooterLayout({required this.overlap, required super.children});
  final double overlap;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderMessageFooter(overlap);

  @override
  void updateRenderObject(
    BuildContext context,
    covariant RenderBox renderObject,
  ) {
    (renderObject as _RenderMessageFooter)
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
  _RenderMessageFooter(this.overlap);

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
    final width = math.max(contentSize.width, footerSize.width);
    final footerOffset = Offset(0, math.max(0, contentSize.height - overlap));
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
