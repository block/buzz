import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme_extensions.dart';

/// Crisp mobile rendering of desktop BuzzLogoAnimation's v8 loading morph.
/// Uses its 880ms keyframes and easing, without the desktop noise filter.
class BeeLoadingMark extends HookWidget {
  const BeeLoadingMark({super.key});

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: const Duration(milliseconds: 880),
    );
    useEffect(() {
      if (reduced) {
        controller
          ..stop()
          ..value = 1;
      } else {
        controller.repeat(reverse: true);
      }
      return controller.stop;
    }, [controller, reduced]);
    return Semantics(
      label: 'Loading',
      liveRegion: true,
      child: ExcludeSemantics(
        child: RepaintBoundary(
          child: CustomPaint(
            size: const Size(80, 80 * 309 / 466),
            painter: _BeeMorphPainter(controller, context.colors.onSurface),
          ),
        ),
      ),
    );
  }
}

class _BeeMorphPainter extends CustomPainter {
  _BeeMorphPainter(this.animation, this.color) : super(repaint: animation);
  final Animation<double> animation;
  final Color color;
  static const ease = Cubic(.16, 1, .3, 1);

  double value(List<double> times, List<double> values) {
    final t = animation.value;
    for (var i = 1; i < times.length; i++) {
      if (t <= times[i]) {
        final progress = ((t - times[i - 1]) / (times[i] - times[i - 1])).clamp(
          0.0,
          1.0,
        );
        return values[i - 1] +
            (values[i] - values[i - 1]) * ease.transform(progress);
      }
    }
    return values.last;
  }

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.scale(size.width / 466);
    canvas.saveLayer(const Rect.fromLTWH(-20, -20, 506, 349), Paint());
    final wingOpacity = value([0, .54, .55, 1], [0, 0, 1, 1]);
    final wingPaint = Paint()
      ..color = color.withValues(alpha: color.a * wingOpacity);
    for (final xs in const <List<double>>[
      [233.0, 233, 76, 95, 91.7],
      [233.0, 233, 390, 371, 374.3],
    ]) {
      canvas.drawCircle(
        Offset(value([0, .54, .74, .9, 1], xs), 154.5),
        91.7,
        wingPaint,
      );
    }
    const bodyTimes = [0.0, .08, .16, .23, .35, .47, .6, 1.0];
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(
          value(bodyTimes, [233.4, 200, 186, 186, 120, 132, 127, 128]),
          value(bodyTimes, [154.5, 121, 108, 108, 42, -12, 3, 0]),
          value(bodyTimes, [0, 66, 93, 93, 226, 204, 212, 210]),
          value(bodyTimes, [0, 66, 93, 93, 226, 326, 304, 309]),
        ),
        Radius.circular(value(bodyTimes, [0, 33, 46.5, 14, 40, 37, 33, 34])),
      ),
      Paint()
        ..color = color.withValues(
          alpha: color.a * value([0, .03, .08, 1], [0, 0, 1, 1]),
        ),
    );
    final eyePaint = Paint()
      ..blendMode = BlendMode.dstOut
      ..color = Colors.black.withValues(
        alpha: value([0, .18, .2, 1], [0, 0, 1, 1]),
      );
    for (final xs in const <List<double>>[
      [233.4, 233.4, 185, 196, 193.3, 193.3],
      [233.4, 233.4, 283, 273, 276, 276],
    ]) {
      canvas.drawOval(
        Rect.fromCenter(
          center: Offset(
            value([0, .23, .45, .58, .68, 1], xs),
            value(
              [0, .23, .45, .58, .68, 1],
              [154.5, 154.5, 76, 87, 84.4, 84.4],
            ),
          ),
          width: 54,
          height: 2 * value([0, .76, .84, .92, 1], [27, 27, 2, 27, 27]),
        ),
        eyePaint,
      );
    }
    for (final lower in [false, true]) {
      final times = lower
          ? [0.0, .54, .72, .84, .94, 1.0]
          : [0.0, .42, .6, .72, .82, 1.0];
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(
            value(times, [
              234.8,
              234.8,
              158,
              169,
              lower ? 166.9 : 166.3,
              lower ? 166.9 : 166.3,
            ]),
            lower ? 235.1 : 157.2,
            value(times, [
              0,
              0,
              153,
              132,
              lower ? 136.2 : 136.9,
              lower ? 136.2 : 136.9,
            ]),
            lower ? 37.6 : 38.3,
          ),
          const Radius.circular(5),
        ),
        Paint()
          ..blendMode = BlendMode.dstOut
          ..color = Colors.black.withValues(
            alpha: value(lower ? [0, .54, .55, 1] : [0, .42, .43, 1], [
              0,
              0,
              1,
              1,
            ]),
          ),
      );
    }
    canvas.restore();
    canvas.restore();
  }

  @override
  bool shouldRepaint(_BeeMorphPainter oldDelegate) =>
      oldDelegate.color != color || oldDelegate.animation != animation;
}
