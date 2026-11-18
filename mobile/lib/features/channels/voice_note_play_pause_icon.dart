import 'dart:ui' show lerpDouble;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Animated icon that morphs between voice-note play and pause glyphs.
class VoiceNotePlayPauseIcon extends HookWidget {
  const VoiceNotePlayPauseIcon({
    super.key,
    required this.isPlaying,
    this.color,
    this.size = 23,
  });

  final bool isPlaying;
  final Color? color;
  final double size;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 160),
      initialValue: isPlaying ? 1 : 0,
    );
    useEffect(() {
      final target = isPlaying ? 1.0 : 0.0;
      if (reducedMotion) {
        controller.value = target;
      } else {
        controller.animateTo(
          target,
          duration: const Duration(milliseconds: 160),
          curve: const Cubic(0.77, 0, 0.175, 1),
        );
      }
      return null;
    }, [isPlaying, reducedMotion]);

    return ExcludeSemantics(
      child: SizedBox.square(
        dimension: size,
        child: AnimatedBuilder(
          animation: controller,
          builder: (context, _) => CustomPaint(
            key: ValueKey(
              isPlaying
                  ? 'voice-note-play-pause-icon-pause'
                  : 'voice-note-play-pause-icon-play',
            ),
            painter: _VoiceNotePlayPausePainter(
              progress: controller.value,
              color: color ?? IconTheme.of(context).color!,
            ),
          ),
        ),
      ),
    );
  }
}

class _VoiceNotePlayPausePainter extends CustomPainter {
  const _VoiceNotePlayPausePainter({
    required this.progress,
    required this.color,
  });

  final double progress;
  final Color color;

  static const _playPrimary = <Offset>[
    Offset(8, 4.75),
    Offset(7, 4.15),
    Offset(7, 5.35),
    Offset(7, 18.65),
    Offset(7, 19.85),
    Offset(8, 19.25),
    Offset(18.1, 12.75),
    Offset(19.3, 12),
    Offset(18.1, 11.25),
    Offset(8, 4.75),
    Offset(8, 4.75),
    Offset(8, 4.75),
  ];
  static const _playSecondary = <Offset>[
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
    Offset(12, 12),
  ];
  static const _pausePrimary = <Offset>[
    Offset(7.5, 5),
    Offset(6.5, 5),
    Offset(6.5, 6),
    Offset(6.5, 18),
    Offset(6.5, 19),
    Offset(7.5, 19),
    Offset(9.5, 19),
    Offset(10.5, 19),
    Offset(10.5, 18),
    Offset(10.5, 6),
    Offset(10.5, 5),
    Offset(9.5, 5),
  ];
  static const _pauseSecondary = <Offset>[
    Offset(14.5, 5),
    Offset(13.5, 5),
    Offset(13.5, 6),
    Offset(13.5, 18),
    Offset(13.5, 19),
    Offset(14.5, 19),
    Offset(16.5, 19),
    Offset(17.5, 19),
    Offset(17.5, 18),
    Offset(17.5, 6),
    Offset(17.5, 5),
    Offset(16.5, 5),
  ];

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.shortestSide / 24;
    canvas.save();
    canvas.scale(scale, scale);
    final fillPaint = Paint()
      ..color = color
      ..style = PaintingStyle.fill;
    final strokePaint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 0.8
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    final primary = _morphPath(_playPrimary, _pausePrimary);
    final secondary = _morphPath(_playSecondary, _pauseSecondary);
    canvas
      ..drawPath(primary, fillPaint)
      ..drawPath(primary, strokePaint)
      ..drawPath(secondary, fillPaint)
      ..drawPath(secondary, strokePaint);
    canvas.restore();
  }

  Path _morphPath(List<Offset> from, List<Offset> to) {
    final points = [
      for (var index = 0; index < from.length; index++)
        Offset(
          lerpDouble(from[index].dx, to[index].dx, progress)!,
          lerpDouble(from[index].dy, to[index].dy, progress)!,
        ),
    ];
    return Path()
      ..moveTo(points[0].dx, points[0].dy)
      ..quadraticBezierTo(
        points[1].dx,
        points[1].dy,
        points[2].dx,
        points[2].dy,
      )
      ..lineTo(points[3].dx, points[3].dy)
      ..quadraticBezierTo(
        points[4].dx,
        points[4].dy,
        points[5].dx,
        points[5].dy,
      )
      ..lineTo(points[6].dx, points[6].dy)
      ..quadraticBezierTo(
        points[7].dx,
        points[7].dy,
        points[8].dx,
        points[8].dy,
      )
      ..lineTo(points[9].dx, points[9].dy)
      ..quadraticBezierTo(
        points[10].dx,
        points[10].dy,
        points[11].dx,
        points[11].dy,
      )
      ..close();
  }

  @override
  bool shouldRepaint(_VoiceNotePlayPausePainter oldDelegate) =>
      oldDelegate.progress != progress || oldDelegate.color != color;
}
