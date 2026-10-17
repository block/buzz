import 'dart:math' as math;
import 'package:flutter/material.dart';
import '../theme/buzz_icons.dart';

/// Tabler plus/edit vectors share one reversible, clockwise quarter-turn.
class ComposeMorphIcon extends StatelessWidget {
  const ComposeMorphIcon({
    super.key,
    required this.compose,
    required this.color,
    this.surfaceBuilder,
  });
  final bool compose;
  final Color color;

  /// Includes the button surface in the same scale pulse as its icon.
  final Widget Function(Widget icon)? surfaceBuilder;

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    return TweenAnimationBuilder<double>(
      tween: Tween(end: compose ? 1 : 0),
      duration: reduced ? Duration.zero : const Duration(milliseconds: 240),
      curve: Curves.easeInOutCubic,
      builder: (context, animatedProgress, _) {
        final progress = reduced ? (compose ? 1.0 : 0.0) : animatedProgress;
        final lift = math.sin(progress * math.pi);
        final Widget icon = progress == 0 || progress == 1
            ? Transform.rotate(
                angle: -progress * math.pi / 2,
                child: Icon(
                  progress == 0 ? BuzzIcons.plus : BuzzIcons.edit,
                  size: 24,
                  color: color,
                ),
              )
            : CustomPaint(
                size: const Size.square(24),
                painter: _ComposePainter(progress, color),
              );
        final rotatingIcon = Transform.rotate(
          key: const ValueKey('compose-icon-rotation'),
          angle: progress * math.pi / 2,
          child: icon,
        );
        return Transform.scale(
          key: const ValueKey('compose-surface-scale'),
          scale: 1 + 0.08 * lift,
          child: surfaceBuilder?.call(rotatingIcon) ?? rotatingIcon,
        );
      },
    );
  }
}

class _ComposePainter extends CustomPainter {
  _ComposePainter(this.progress, this.color);
  final double progress;
  final Color color;

  // Tabler edit.svg paths, sampled once to interpolate compatible contours.
  static final contours = <List<Offset>>[
    _sample(
      Path()
        ..moveTo(7, 7)
        ..lineTo(6, 7)
        ..quadraticBezierTo(4, 7, 4, 9)
        ..lineTo(4, 18)
        ..quadraticBezierTo(4, 20, 6, 20)
        ..lineTo(15, 20)
        ..quadraticBezierTo(17, 20, 17, 18)
        ..lineTo(17, 17),
    ),
    _sample(
      Path()
        ..moveTo(20.385, 6.585)
        ..cubicTo(22.365, 4.605, 19.395, 1.635, 17.415, 3.615)
        ..lineTo(9, 12)
        ..lineTo(9, 15)
        ..lineTo(12, 15)
        ..lineTo(20.385, 6.585),
    ),
    _sample(
      Path()
        ..moveTo(16, 5)
        ..lineTo(19, 8),
    ),
  ];

  static List<Offset> _sample(Path path) {
    final metric = path.computeMetrics().single;
    return List.generate(
      49,
      (i) => metric.getTangentForOffset(metric.length * i / 48)!.position,
    );
  }

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.scale(size.width / 24, size.height / 24);
    canvas.translate(12, 12);
    // The icon wrapper supplies the quarter-turn.
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    for (var stroke = 0; stroke < contours.length; stroke++) {
      final path = Path();
      for (var i = 0; i < contours[stroke].length; i++) {
        final fraction = i / (contours[stroke].length - 1);
        final start = stroke == 0
            ? Offset(0, -7 + 14 * fraction)
            : stroke == 1
            ? Offset(-7 + 14 * fraction, 0)
            : Offset.zero;
        final end = contours[stroke][i] - const Offset(12, 12);
        // Undo the final rotation in target space: edit ends upright.
        final point = Offset.lerp(start, Offset(end.dy, -end.dx), progress)!;
        if (i == 0) {
          path.moveTo(point.dx, point.dy);
        } else {
          path.lineTo(point.dx, point.dy);
        }
      }
      paint.color = color.withValues(
        alpha: color.a * (stroke == 2 ? progress : 1),
      );
      canvas.drawPath(path, paint);
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_ComposePainter oldDelegate) =>
      progress != oldDelegate.progress || color != oldDelegate.color;
}
