import 'dart:math' show min;

import 'package:flutter/material.dart';

/// The Zorro hat mark at a caller-controlled strike position.
///
/// The geometry mirrors the desktop Cordovan hat. Callers animate
/// [strikeAmount] to give the mark a quick, restrained slash-like pulse.
class ZorroHatMark extends StatelessWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The base ink color used for the hat.
  final Color color;

  /// Progress through the strike pulse, from 0 to 1.
  final double strikeAmount;

  const ZorroHatMark({
    required this.width,
    required this.color,
    required this.strikeAmount,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: CustomPaint(
        size: Size.square(width),
        painter: _ZorroHatPainter(color: color, strikeAmount: strikeAmount),
      ),
    );
  }
}

/// Compatibility wrapper for the former loading-bee API.
///
/// Callers keep their stable widget type while the rendered product mark is
/// the Zorro hat. [flapAmount] now drives the hat's strike pulse.
class FlappingBee extends StatelessWidget {
  /// The rendered width of the complete mark.
  final double width;

  /// The base ink color used for the mark.
  final Color color;

  /// Compatibility animation value, interpreted as strike progress.
  final double flapAmount;

  /// Retained for source compatibility with the former expressive-eye mark.
  ///
  /// The Zorro hat has no eyes, so this value does not alter its geometry.
  final double? eyeProgress;

  const FlappingBee({
    required this.width,
    required this.color,
    required this.flapAmount,
    this.eyeProgress,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return ZorroHatMark(width: width, color: color, strikeAmount: flapAmount);
  }
}

class _ZorroHatPainter extends CustomPainter {
  final Color color;
  final double strikeAmount;

  const _ZorroHatPainter({required this.color, required this.strikeAmount});

  @override
  void paint(Canvas canvas, Size size) {
    final animatedScale = 1 + (0.04 * strikeAmount.clamp(0.0, 1.0));
    final scale = min(size.width / 512, size.height / 512) * animatedScale;
    final renderedSize = 512 * scale;

    canvas
      ..save()
      ..translate(
        (size.width - renderedSize) / 2,
        (size.height - renderedSize) / 2,
      )
      ..scale(scale);

    final brim = Path()
      ..moveTo(39, 346)
      ..cubicTo(70, 308, 171, 287, 256, 287)
      ..cubicTo(341, 287, 442, 308, 473, 346)
      ..cubicTo(491, 368, 480, 392, 448, 405)
      ..cubicTo(399, 425, 328, 436, 256, 436)
      ..cubicTo(184, 436, 113, 425, 64, 405)
      ..cubicTo(32, 392, 21, 368, 39, 346)
      ..close();
    final brimInset = Path()
      ..moveTo(60, 344)
      ..cubicTo(102, 317, 179, 302, 256, 302)
      ..cubicTo(333, 302, 410, 317, 452, 344)
      ..cubicTo(472, 357, 471, 371, 450, 383)
      ..cubicTo(408, 407, 331, 421, 256, 421)
      ..cubicTo(181, 421, 104, 407, 62, 383)
      ..cubicTo(41, 371, 40, 357, 60, 344)
      ..close();
    final crown = Path()
      ..moveTo(174, 216)
      ..cubicTo(174, 196, 211, 183, 256, 183)
      ..cubicTo(301, 183, 338, 196, 338, 216)
      ..lineTo(352, 348)
      ..lineTo(160, 348)
      ..close();
    final crownTop = Path()
      ..moveTo(174, 216)
      ..cubicTo(174, 196, 211, 183, 256, 183)
      ..cubicTo(301, 183, 338, 196, 338, 216)
      ..cubicTo(338, 236, 301, 249, 256, 249)
      ..cubicTo(211, 249, 174, 236, 174, 216)
      ..close();
    final zMark = Path()
      ..moveTo(220, 220)
      ..lineTo(292, 220)
      ..lineTo(292, 242)
      ..lineTo(250, 276)
      ..lineTo(292, 276)
      ..lineTo(292, 300)
      ..lineTo(220, 300)
      ..lineTo(220, 277)
      ..lineTo(262, 243)
      ..lineTo(220, 243)
      ..close();
    final lowerShadow = Path()
      ..moveTo(73, 374)
      ..cubicTo(123, 394, 189, 404, 256, 404)
      ..cubicTo(323, 404, 389, 394, 439, 374)
      ..cubicTo(409, 404, 333, 422, 256, 422)
      ..cubicTo(179, 422, 103, 404, 73, 374)
      ..close();

    final insetColor = Color.lerp(color, Colors.white, 0.08)!;
    final crownColor = Color.lerp(color, Colors.black, 0.08)!;
    final crownTopColor = Color.lerp(color, Colors.white, 0.14)!;
    final zColor = color.computeLuminance() > 0.55
        ? Colors.black.withValues(alpha: 0.72)
        : Colors.white.withValues(alpha: 0.72);
    final basePaint = Paint()..color = color;
    canvas
      ..drawPath(brim, basePaint)
      ..drawPath(brimInset, Paint()..color = insetColor)
      ..drawPath(crown, Paint()..color = crownColor)
      ..drawPath(crownTop, Paint()..color = crownTopColor)
      ..drawPath(zMark, Paint()..color = zColor)
      ..drawPath(
        lowerShadow,
        Paint()..color = Colors.black.withValues(alpha: 0.22),
      );

    canvas.restore();
  }

  @override
  bool shouldRepaint(_ZorroHatPainter oldDelegate) =>
      color != oldDelegate.color || strikeAmount != oldDelegate.strikeAmount;
}
