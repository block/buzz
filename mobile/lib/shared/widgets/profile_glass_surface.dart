import 'dart:ui';
import 'package:flutter/material.dart';

class ProfileGlassSurface extends StatelessWidget {
  const ProfileGlassSurface({
    super.key,
    required this.shape,
    required this.child,
    this.dark = false,
  });
  final ShapeBorder shape;
  final Widget child;
  final bool dark;
  @override
  Widget build(BuildContext context) => ClipPath(
    clipper: ShapeBorderClipper(shape: shape),
    child: BackdropFilter(
      filter: ImageFilter.blur(sigmaX: 16, sigmaY: 16),
      child: CustomPaint(
        foregroundPainter: _GlassRim(shape),
        child: ColoredBox(
          color: dark ? const Color(0x851C1C1C) : const Color(0x85FFFFFF),
          child: child,
        ),
      ),
    ),
  );
}

/// The desktop glass material has one-point inset highlights on opposite edges.
class _GlassRim extends CustomPainter {
  const _GlassRim(this.border);
  final ShapeBorder border;

  @override
  void paint(Canvas canvas, Size size) {
    final shape = border.getOuterPath(Offset.zero & size);
    void edge(double dy, Color color) {
      final strip = Path.combine(
        PathOperation.difference,
        shape,
        shape.shift(Offset(0, dy)),
      );
      canvas.drawPath(strip, Paint()..color = color);
    }

    edge(1, const Color(0x73FFFFFF));
    edge(-1, const Color(0x1FFFFFFF));
  }

  @override
  bool shouldRepaint(_GlassRim oldDelegate) => border != oldDelegate.border;
}
