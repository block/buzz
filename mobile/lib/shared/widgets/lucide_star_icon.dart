import 'dart:math' as math;

import 'package:flutter/material.dart';

/// A Lucide-shaped star that can fill without changing its geometry.
class LucideStarIcon extends StatelessWidget {
  /// Creates a Lucide star using [color], optionally filled.
  const LucideStarIcon({
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
    painter: _LucideStarPainter(color: color, filled: filled),
  );
}

class _LucideStarPainter extends CustomPainter {
  const _LucideStarPainter({required this.color, required this.filled});

  final Color color;
  final bool filled;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = math.min(size.width, size.height) / 24;
    final path = Path()
      ..moveTo(12 * scale, 2 * scale)
      ..lineTo(15.09 * scale, 8.26 * scale)
      ..lineTo(22 * scale, 9.27 * scale)
      ..lineTo(17 * scale, 14.14 * scale)
      ..lineTo(18.18 * scale, 21.02 * scale)
      ..lineTo(12 * scale, 17.77 * scale)
      ..lineTo(5.82 * scale, 21.02 * scale)
      ..lineTo(7 * scale, 14.14 * scale)
      ..lineTo(2 * scale, 9.27 * scale)
      ..lineTo(8.91 * scale, 8.26 * scale)
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
  bool shouldRepaint(_LucideStarPainter oldDelegate) =>
      color != oldDelegate.color || filled != oldDelegate.filled;
}
