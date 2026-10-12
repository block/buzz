part of '../message_content.dart';

/// A soft conversation bubble whose final corner points toward the avatar.
class _MessageBubbleShape extends ShapeBorder {
  const _MessageBubbleShape({required this.tail});

  final bool tail;

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.zero;

  @override
  ShapeBorder scale(double t) => this;

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) {
    final radius = math.min(Radii.dialog - Grid.xxs / 2, rect.shortestSide / 2);
    if (!tail) {
      return Path()
        ..addRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)));
    }
    final left = rect.left;
    final right = rect.right;
    final top = rect.top;
    final bottom = rect.bottom;
    return Path()
      ..moveTo(left + radius, top)
      ..lineTo(right - radius, top)
      ..quadraticBezierTo(right, top, right, top + radius)
      ..lineTo(right, bottom - radius)
      ..quadraticBezierTo(right, bottom, right - radius, bottom)
      ..lineTo(left + radius, bottom)
      ..quadraticBezierTo(
        left + Grid.xxs,
        bottom,
        left + Grid.xxs / 2,
        bottom - Grid.quarter,
      )
      ..quadraticBezierTo(left, bottom, left - Grid.xxs, bottom)
      ..cubicTo(
        left,
        bottom - Grid.xxs / 2,
        left,
        bottom - Grid.xxs,
        left,
        bottom - radius,
      )
      ..lineTo(left, top + radius)
      ..quadraticBezierTo(left, top, left + radius, top)
      ..close();
  }

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      getOuterPath(rect, textDirection: textDirection);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {}
}
