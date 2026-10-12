part of '../message_content.dart';

/// Connected-looking corners with a two-circle thought detail at the group end.
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
          bottomLeft: end
              ? const Radius.circular((Radii.sm + Radii.container) / 2)
              : joined,
          bottomRight: round,
        ),
      );
    if (!end) return body;
    // The larger circle overlaps the body; the little one sits in the avatar
    // gutter with a small gap. Union avoids seams in translucent fills.
    final attached = Path()
      ..addOval(
        Rect.fromCircle(
          center: Offset(rect.left + Grid.half, rect.bottom - Grid.xxs),
          radius: Grid.xxs,
        ),
      );
    return Path.combine(PathOperation.union, body, attached)..addOval(
      Rect.fromCircle(
        center: Offset(rect.left - Grid.xxs, rect.bottom + Grid.quarter),
        radius: Grid.quarter + 1,
      ),
    );
  }

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      getOuterPath(rect, textDirection: textDirection);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {}
}
