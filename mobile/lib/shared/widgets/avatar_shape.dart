import 'package:flutter/widgets.dart';

/// The single source of avatar silhouettes.
///
/// Agents use desktop's squircle (`ROUNDED_SQUIRCLE_PATH` in
/// `desktop/src/shared/ui/AvatarClipPaths.tsx`) and people use circles. Every avatar frame, clip, halo and placeholder takes its shape
/// from here, so the two never drift apart across surfaces or clients.
OutlinedBorder avatarShape({
  required bool isAgent,
  BorderSide side = BorderSide.none,
}) => isAgent ? _AgentSquircleBorder(side: side) : CircleBorder(side: side);

/// The avatar silhouette scaled to [rect], for canvas painting.
Path avatarPath(Rect rect, {required bool isAgent}) =>
    avatarShape(isAgent: isAgent).getOuterPath(rect);

/// Clips [child] to the avatar silhouette for [isAgent].
class AvatarClip extends StatelessWidget {
  /// Creates an avatar clip.
  const AvatarClip({required this.isAgent, required this.child, super.key});

  /// Whether [child] is an agent avatar.
  final bool isAgent;

  /// The avatar artwork.
  final Widget child;

  @override
  Widget build(BuildContext context) => isAgent
      ? ClipPath(
          clipper: ShapeBorderClipper(shape: avatarShape(isAgent: true)),
          child: child,
        )
      // People keep the dedicated oval clip, the fast path in dense lists.
      : ClipOval(child: child);
}

Path _agentSquirclePath(Rect rect) {
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

/// Private so agent avatars can only get their shape from [avatarShape].
class _AgentSquircleBorder extends OutlinedBorder {
  const _AgentSquircleBorder({super.side});

  @override
  _AgentSquircleBorder copyWith({BorderSide? side}) =>
      _AgentSquircleBorder(side: side ?? this.side);

  @override
  EdgeInsetsGeometry get dimensions => EdgeInsets.all(side.strokeInset);

  @override
  Path getInnerPath(Rect rect, {TextDirection? textDirection}) =>
      _agentSquirclePath(rect.deflate(side.strokeInset));

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) =>
      _agentSquirclePath(rect);

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {
    if (side.style == BorderStyle.none) return;
    canvas.drawPath(
      _agentSquirclePath(rect.inflate(side.strokeOffset / 2)),
      side.toPaint(),
    );
  }

  @override
  ShapeBorder scale(double t) => _AgentSquircleBorder(side: side.scale(t));

  @override
  bool operator ==(Object other) =>
      other is _AgentSquircleBorder && other.side == side;

  @override
  int get hashCode => side.hashCode;
}
