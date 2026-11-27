part of 'profile_details.dart';

List<Color> _skyColors(DateTime local) {
  // Local clock phases approximate daylight when latitude is unknown.
  const phases = [
    (4.0, Color(0xff10132f), Color(0xff36315f)),
    (6.5, Color(0xff806dad), Color(0xffffad7d)),
    (9.0, Color(0xff518ace), Color(0xffbce5ed)),
    (16.0, Color(0xff518ace), Color(0xffbce5ed)),
    (18.0, Color(0xff88548f), Color(0xfff99a69)),
    (20.0, Color(0xff332655), Color(0xff85578c)),
    (22.0, Color(0xff10132f), Color(0xff36315f)),
  ];
  final hour = local.hour + local.minute / 60;
  final end = phases.indexWhere((phase) => phase.$1 >= hour);
  final a = end > 0 ? phases[end - 1] : phases.first;
  final b = end > 0 ? phases[end] : phases.first;
  final t = end > 0 ? (hour - a.$1) / (b.$1 - a.$1) : 0.0;
  return [Color.lerp(a.$2, b.$2, t)!, Color.lerp(a.$3, b.$3, t)!];
}

class _SunPath extends CustomPainter {
  const _SunPath(this.hour);
  final double hour;

  @override
  void paint(Canvas canvas, Size size) {
    const left = 0.0;
    final width = size.width;
    if (width <= 0) return;
    final horizon = size.height / 2;
    final amplitude = size.height * .32;
    Offset point(double fraction) => Offset(
      left + width * fraction,
      horizon + amplitude * math.cos(2 * math.pi * fraction),
    );
    final path = Path()..moveTo(left, point(0).dy);
    for (var i = 1; i <= 64; i++) {
      final position = point(i / 64);
      path.lineTo(position.dx, position.dy);
    }
    final stroke = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..color = const Color(0x55000000);
    canvas.drawPath(path, stroke);
    canvas.save();
    canvas.clipRect(Rect.fromLTWH(left - 1, 0, width + 2, horizon));
    canvas.drawPath(path, stroke..color = const Color(0xAAFFFFFF));
    canvas.restore();
    canvas.drawLine(
      Offset(left, horizon),
      Offset(left + width, horizon),
      Paint()
        ..color = const Color(0x88FFFFFF)
        ..strokeWidth = 1,
    );
    final sun = point((6 + (width - 12) * hour / 24) / width);
    final elevation = -math.cos(2 * math.pi * hour / 24);
    const amber = Color(0xffffb264);
    final color = elevation >= 0
        ? Color.lerp(amber, const Color(0xfffff8de), elevation)!
        : Color.lerp(amber, const Color(0xffc8bde6), -elevation)!;
    final flare = ((elevation + .18) / .55).clamp(0.0, 1.0);
    if (flare > 0) {
      canvas.drawCircle(
        sun,
        22,
        Paint()
          ..shader = ui.Gradient.radial(sun, 22, [
            color.withValues(alpha: .48 * flare),
            color.withValues(alpha: 0),
          ]),
      );
      final direction = sun.dx < width / 2 ? 1.0 : -1.0;
      final reflection = Offset.lerp(sun, size.center(Offset.zero), .16)!;
      canvas.drawCircle(
        reflection + Offset(direction * 3 * (1 - elevation), 0),
        2.5,
        Paint()..color = color.withValues(alpha: .30 * flare),
      );
    }
    canvas.drawCircle(sun, 6, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_SunPath old) => hour != old.hour;
}

class _LocalSky extends HookWidget {
  const _LocalSky({required this.local});
  final DateTime local;

  @override
  Widget build(BuildContext context) {
    final hour = local.hour + local.minute / 60;
    final colors = _skyColors(local);
    final stars = hour < 6
        ? ((6 - hour) / 2).clamp(0.0, 1.0)
        : ((hour - 19) / 3).clamp(0.0, 1.0);
    final pulse = useAnimationController(duration: const Duration(seconds: 4));
    final animate =
        stars > 0 &&
        !MediaQuery.disableAnimationsOf(context) &&
        TickerMode.valuesOf(context).enabled;
    useEffect(() {
      if (animate) pulse.repeat(reverse: true);
      return pulse.stop;
    }, [animate, pulse]);
    return ExcludeSemantics(
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: colors,
          ),
        ),
        child: Stack(
          fit: StackFit.expand,
          children: [
            CustomPaint(painter: _SunPath(hour)),
            if (stars > 0)
              AnimatedBuilder(
                animation: pulse,
                builder: (context, _) => Stack(
                  children: [
                    for (final star in const [
                      (Alignment(.38, -.58), 2.5),
                      (Alignment(.73, -.22), 2.0),
                      (Alignment(-.25, -.7), 1.5),
                    ])
                      Align(
                        alignment: star.$1,
                        child: Container(
                          width: star.$2,
                          height: star.$2,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: Colors.white.withValues(
                              alpha:
                                  stars *
                                  (.6 +
                                      .3 *
                                          (star.$2 == 2
                                              ? pulse.value
                                              : 1 - pulse.value)),
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }
}
