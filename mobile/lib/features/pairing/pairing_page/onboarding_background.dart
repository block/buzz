part of '../pairing_page.dart';

class _OnboardingBackground extends StatelessWidget {
  final Widget child;

  const _OnboardingBackground({required this.child});

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: const Key('pairing-onboarding-background'),
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [_onboardingChartreuse, _onboardingShellBottom],
        ),
      ),
      child: CustomPaint(painter: const _DotGridPainter(), child: child),
    );
  }
}

class _DotGridPainter extends CustomPainter {
  const _DotGridPainter();

  @override
  void paint(Canvas canvas, Size size) {
    final dotPaint = Paint()..color = _onboardingInk.withValues(alpha: 0.08);
    const spacing = 24.0;

    for (var x = 0.0; x <= size.width; x += spacing) {
      for (var y = 0.0; y <= size.height; y += spacing) {
        canvas.drawCircle(Offset(x, y), 1, dotPaint);
      }
    }
  }

  @override
  bool shouldRepaint(_DotGridPainter oldDelegate) => false;
}
