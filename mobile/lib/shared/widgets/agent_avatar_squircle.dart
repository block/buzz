import 'package:flutter/widgets.dart';

/// The normalized SVG path used for every agent avatar on mobile.
///
/// These commands are mirrored by [AgentAvatarSquircleClipper], which scales
/// the coordinates to the avatar box so one path covers every size instead of
/// maintaining per-size corner radii.
const agentAvatarSquirclePath =
    'M 50,0 C 93,0 100,7 100,50 C 100,93 93,100 50,100 '
    'C 7,100 0,93 0,50 C 0,7 7,0 50,0 Z';

Path _agentAvatarSquirclePath(Size size) {
  final x = size.width / 100;
  final y = size.height / 100;
  return Path()
    ..moveTo(50 * x, 0)
    ..cubicTo(93 * x, 0, 100 * x, 7 * y, 100 * x, 50 * y)
    ..cubicTo(100 * x, 93 * y, 93 * x, 100 * y, 50 * x, 100 * y)
    ..cubicTo(7 * x, 100 * y, 0, 93 * y, 0, 50 * y)
    ..cubicTo(0, 7 * y, 7 * x, 0, 50 * x, 0)
    ..close();
}

/// Scales [agentAvatarSquirclePath] to the incoming avatar bounds.
class AgentAvatarSquircleClipper extends CustomClipper<Path> {
  const AgentAvatarSquircleClipper();

  @override
  Path getClip(Size size) => _agentAvatarSquirclePath(size);

  @override
  bool shouldReclip(covariant AgentAvatarSquircleClipper oldClipper) => false;
}

/// Clips [child] to the shared, size-independent agent-avatar silhouette.
class AgentAvatarSquircle extends StatelessWidget {
  const AgentAvatarSquircle({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) =>
      ClipPath(clipper: const AgentAvatarSquircleClipper(), child: child);
}
