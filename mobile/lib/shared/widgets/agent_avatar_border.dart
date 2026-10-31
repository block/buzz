import 'package:flutter/painting.dart';

/// Returns the agent avatar squircle scaled to [rect].
///
/// Matches desktop's `avatar-squircle.svg` mask so agent avatars share one
/// silhouette across clients.
Path agentAvatarPath(Rect rect) {
  Offset at(double x, double y) =>
      Offset(rect.left + rect.width * x, rect.top + rect.height * y);
  final start = at(.5, 0);
  final path = Path()..moveTo(start.dx, start.dy);
  void curve(Offset a, Offset b, Offset c) =>
      path.cubicTo(a.dx, a.dy, b.dx, b.dy, c.dx, c.dy);
  curve(at(.93, 0), at(1, .07), at(1, .5));
  curve(at(1, .93), at(.93, 1), at(.5, 1));
  curve(at(.07, 1), at(0, .93), at(0, .5));
  curve(at(0, .07), at(.07, 0), start);
  return path..close();
}

/// The shape of every agent avatar, its halos, and its placeholders.
class AgentAvatarBorder extends OutlinedBorder {
  const AgentAvatarBorder({super.side});

  @override
  AgentAvatarBorder copyWith({BorderSide? side}) =>
      AgentAvatarBorder(side: side ?? this.side);

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.all(side.strokeInset);

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      agentAvatarPath(rect.deflate(side.strokeInset));

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) =>
      agentAvatarPath(rect);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {
    if (side.style == BorderStyle.none) return;
    canvas.drawPath(
      agentAvatarPath(rect.inflate(side.strokeOffset / 2)),
      side.toPaint(),
    );
  }

  @override
  ShapeBorder scale(double t) => AgentAvatarBorder(side: side.scale(t));

  @override
  bool operator ==(Object other) =>
      other is AgentAvatarBorder && other.side == side;

  @override
  int get hashCode => side.hashCode;
}
