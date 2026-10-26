import 'dart:math' as math;

import 'package:flutter/cupertino.dart';

import '../../shared/theme/buzz_icons.dart';

/// Matches the native setup avatar, scaled from its 96-point source bounds.
class ProfileSetupEmptyAvatar extends StatelessWidget {
  const ProfileSetupEmptyAvatar({super.key});

  @override
  Widget build(BuildContext context) => FittedBox(
    child: SizedBox.square(
      dimension: 96,
      child: CustomPaint(
        painter: _RingPainter(
          CupertinoDynamicColor.resolve(CupertinoColors.tertiaryLabel, context),
        ),
        child: Center(
          child: Icon(
            BuzzIcons.plus,
            size: 28,
            color: CupertinoDynamicColor.resolve(
              CupertinoColors.label,
              context,
            ),
          ),
        ),
      ),
    ),
  );
}

class _RingPainter extends CustomPainter {
  const _RingPainter(this.color);
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2;
    final bounds = Offset.zero & size;
    final radius = (size.width - 4) / 2;
    final circumference = 2 * math.pi * radius;
    final count = (circumference / 9).round();
    final step = 2 * math.pi / count;
    for (var index = 0; index < count; index++) {
      canvas.drawArc(
        bounds.deflate(2),
        index * step,
        step * 4 / 9,
        false,
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_RingPainter oldDelegate) => oldDelegate.color != color;
}
