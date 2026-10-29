import 'dart:math' as math;

import 'package:flutter/cupertino.dart';

import '../../shared/theme/buzz_icons.dart';

/// Matches the native setup avatar, using the same 220-point preview bounds.
class ProfileSetupEmptyAvatar extends StatelessWidget {
  const ProfileSetupEmptyAvatar({super.key});

  @override
  Widget build(BuildContext context) => FittedBox(
    child: SizedBox.square(
      dimension: 220,
      child: CustomPaint(
        painter: _RingPainter(
          CupertinoDynamicColor.resolve(CupertinoColors.tertiaryLabel, context),
        ),
        child: Center(
          child: Icon(
            BuzzIcons.plus,
            size: 40,
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
      ..strokeWidth = 3;
    final bounds = Offset.zero & size;

    const count = 32;
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
