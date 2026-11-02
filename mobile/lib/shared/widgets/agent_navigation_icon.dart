import 'package:flutter/material.dart';

/// The desktop designer-approved Soft & wide robot mark.
class AgentNavigationIcon extends StatelessWidget {
  const AgentNavigationIcon({this.color, this.size = 22, super.key});
  final Color? color;
  final double size;
  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size.square(size),
    painter: _AgentMarkPainter(
      color ?? IconTheme.of(context).color ?? Colors.black,
    ),
  );
}

class _AgentMarkPainter extends CustomPainter {
  const _AgentMarkPainter(this.color);
  final Color color;
  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 24, size.height / 24);
    final stroke = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        const Rect.fromLTWH(4.75, 7, 14.5, 13.25),
        const Radius.circular(4.5),
      ),
      stroke,
    );
    canvas.drawPath(
      Path()
        ..moveTo(12, 7)
        ..lineTo(12, 2.5)
        ..moveTo(3.25, 11.5)
        ..lineTo(3.25, 15.75)
        ..moveTo(20.75, 11.5)
        ..lineTo(20.75, 15.75),
      stroke,
    );
    final fill = Paint()..color = color;
    canvas.drawCircle(const Offset(9, 13.25), 1.4, fill);
    canvas.drawCircle(const Offset(15, 13.25), 1.4, fill);
  }

  @override
  bool shouldRepaint(_AgentMarkPainter oldDelegate) =>
      color != oldDelegate.color;
}
