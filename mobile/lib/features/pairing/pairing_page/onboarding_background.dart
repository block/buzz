part of '../pairing_page.dart';

class _OnboardingBackground extends StatelessWidget {
  final Widget child;

  const _OnboardingBackground({required this.child});

  @override
  Widget build(BuildContext context) {
    final isDark = context._onboardingIsDark;
    return DecoratedBox(
      key: const Key('pairing-onboarding-background'),
      decoration: BoxDecoration(
        color: isDark ? const Color(0xFF11181D) : const Color(0xFFE7F0EF),
        image: isDark
            ? null
            : const DecorationImage(
                // Same supplied artwork and full-image stretch as Buzz 2.0.
                image: AssetImage('assets/images/shell-gradient.png'),
                fit: BoxFit.fill,
              ),
      ),
      child: CustomPaint(
        painter: _OnboardingShellPainter(isDark: isDark),
        child: child,
      ),
    );
  }
}

class _OnboardingShellPainter extends CustomPainter {
  final bool isDark;

  const _OnboardingShellPainter({required this.isDark});

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    if (isDark) {
      // Match the desktop's layered CSS farthest-corner ellipses.
      _paintGlow(
        canvas,
        size,
        const Offset(0.9, 1),
        0.7,
        const Color(0xFF39304B),
      );
      _paintGlow(
        canvas,
        size,
        const Offset(0.15, 0),
        0.65,
        const Color(0xFF203D53),
      );
    }
  }

  void _paintGlow(
    Canvas canvas,
    Size size,
    Offset position,
    double stop,
    Color color,
  ) {
    final radiusX =
        (position.dx > 0.5 ? position.dx : 1 - position.dx) *
        size.width *
        sqrt2 *
        stop;
    final radiusY =
        (position.dy > 0.5 ? position.dy : 1 - position.dy) *
        size.height *
        sqrt2 *
        stop;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    canvas.translate(position.dx * size.width, position.dy * size.height);
    canvas.scale(radiusX, radiusY);
    canvas.drawCircle(
      Offset.zero,
      1,
      Paint()
        ..shader = RadialGradient(
          colors: [color, color.withValues(alpha: 0)],
        ).createShader(const Rect.fromLTWH(-1, -1, 2, 2)),
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(_OnboardingShellPainter oldDelegate) =>
      oldDelegate.isDark != isDark;
}
