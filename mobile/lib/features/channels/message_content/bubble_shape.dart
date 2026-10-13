part of '../message_content.dart';

/// Grouped message corners without a tail.
class _MessageBubbleShape extends ShapeBorder {
  const _MessageBubbleShape({required this.start, required this.end});

  final bool start;
  final bool end;

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.zero;

  @override
  ShapeBorder scale(double t) => this;

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) {
    final round = Radius.circular(
      math.min(Radii.container, rect.shortestSide / 2),
    );
    const joined = Radius.circular(Radii.sm);
    final body = Path()
      ..addRRect(
        RRect.fromRectAndCorners(
          rect,
          topLeft: start ? round : joined,
          topRight: round,
          bottomLeft: end ? round : joined,
          bottomRight: round,
        ),
      );
    return body;
  }

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      getOuterPath(rect, textDirection: textDirection);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {}
}

BorderRadius _attachmentCorners({required bool start, required bool end}) =>
    BorderRadius.only(
      topLeft: Radius.circular(start ? Radii.container : Radii.sm),
      topRight: const Radius.circular(Radii.container),
      bottomLeft: Radius.circular(end ? Radii.container : Radii.sm),
      bottomRight: const Radius.circular(Radii.container),
    );
