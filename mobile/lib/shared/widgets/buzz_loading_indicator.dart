import 'dart:math' show pi;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';

/// The shared mobile loading indicator, matching the desktop arc spinner.
class BuzzLoadingIndicator extends HookConsumerWidget {
  /// The spinner diameter.
  final double size;

  /// An optional spinner color. Defaults to the active accent color.
  final Color? color;

  /// The accessibility announcement for this loading state.
  final String semanticLabel;

  /// Creates a looping arc loading indicator.
  const BuzzLoadingIndicator({
    this.size = 40,
    this.color,
    this.semanticLabel = 'Loading',
    super.key,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 500),
    );

    useEffect(() {
      if (reducedMotion) {
        animation
          ..stop()
          ..value = 0;
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, reducedMotion]);

    final spinnerColor = color ?? context.colors.primary;
    final strokeWidth = (size / 6).clamp(2.0, 4.0);

    return Semantics(
      liveRegion: true,
      label: semanticLabel,
      child: ExcludeSemantics(
        child: RotationTransition(
          key: const ValueKey('buzz-loading-indicator-spinner'),
          turns: animation,
          child: CustomPaint(
            size: Size.square(size),
            painter: _ArcSpinnerPainter(
              color: spinnerColor,
              strokeWidth: strokeWidth,
            ),
          ),
        ),
      ),
    );
  }
}

class _ArcSpinnerPainter extends CustomPainter {
  final Color color;
  final double strokeWidth;

  const _ArcSpinnerPainter({required this.color, required this.strokeWidth});

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final radius = (size.shortestSide - strokeWidth) / 2;
    final bounds = Rect.fromCircle(center: center, radius: radius);
    final trackPaint = Paint()
      ..color = color.withValues(alpha: color.a * 0.1)
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth;
    final arcPaint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth;

    canvas
      ..drawCircle(center, radius, trackPaint)
      ..drawArc(bounds, -pi / 2, pi / 2, false, arcPaint);
  }

  @override
  bool shouldRepaint(_ArcSpinnerPainter oldDelegate) {
    return color != oldDelegate.color || strokeWidth != oldDelegate.strokeWidth;
  }
}
