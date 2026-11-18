import 'dart:math' as math;

import 'package:flutter/material.dart';

/// A Tabler-shaped star that can fill without changing its geometry.
class TablerStarIcon extends StatelessWidget {
  /// Creates a Tabler star using [color], optionally filled.
  const TablerStarIcon({
    super.key,
    required this.color,
    this.filled = false,
    this.size = 22,
  });

  /// Color of the star's stroke or fill.
  final Color color;

  /// Whether to paint the star filled instead of outlined.
  final bool filled;

  /// Width and height of the square icon canvas.
  final double size;

  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size.square(size),
    painter: _TablerStarPainter(color: color, filled: filled),
  );
}

class _TablerStarPainter extends CustomPainter {
  const _TablerStarPainter({required this.color, required this.filled});

  final Color color;
  final bool filled;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = math.min(size.width, size.height) / 24;
    // Tabler 3.46.0 outline/star.svg, shared by both Star action surfaces.
    final path = Path()
      ..moveTo(12 * scale, 17.75 * scale)
      ..relativeLineTo(-6.172 * scale, 3.245 * scale)
      ..relativeLineTo(1.179 * scale, -6.873 * scale)
      ..relativeLineTo(-5 * scale, -4.867 * scale)
      ..relativeLineTo(6.9 * scale, -1 * scale)
      ..relativeLineTo(3.086 * scale, -6.253 * scale)
      ..relativeLineTo(3.086 * scale, 6.253 * scale)
      ..relativeLineTo(6.9 * scale, 1 * scale)
      ..relativeLineTo(-5 * scale, 4.867 * scale)
      ..relativeLineTo(1.179 * scale, 6.873 * scale)
      ..relativeLineTo(-6.158 * scale, -3.245 * scale)
      ..close();

    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = filled ? PaintingStyle.fill : PaintingStyle.stroke
        ..strokeWidth = 2 * scale
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_TablerStarPainter oldDelegate) =>
      color != oldDelegate.color || filled != oldDelegate.filled;
}
