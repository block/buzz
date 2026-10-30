part of '../pairing_page.dart';

// Flutter adaptation of Buzz 2.0's glass-primary-interactive and ghost button.
// Keep the material together: glass-2 fill, blur-md, directional rim, glass-3
// interaction. Flutter's buttons retain focus, semantics, and disabled behavior.
extension _OnboardingActionStyles on BuildContext {
  ButtonStyle get _onboardingGlassButtonStyle =>
      _onboardingButtonStyle.copyWith(
        backgroundColor: const WidgetStatePropertyAll(Colors.transparent),
        foregroundColor: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.disabled)
              ? _onboardingInk.withValues(alpha: 0.45)
              : _onboardingInk,
        ),
        overlayColor: const WidgetStatePropertyAll(Colors.transparent),
        side: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.focused)
              ? BorderSide(color: _onboardingInk, width: 2)
              : BorderSide.none,
        ),
        backgroundBuilder: (context, states, child) {
          final active =
              states.contains(WidgetState.hovered) ||
              states.contains(WidgetState.pressed);
          final dark = context._onboardingIsDark;
          final fill = dark
              ? (active ? const Color(0xA81C1C1C) : const Color(0x851C1C1C))
              : (active ? const Color(0x9EFFFFFF) : const Color(0x85FFFFFF));
          return ClipRRect(
            borderRadius: BorderRadius.circular(999),
            child: BackdropFilter(
              filter: ImageFilter.blur(sigmaX: 16, sigmaY: 16),
              child: CustomPaint(
                foregroundPainter: _GlassButtonRim(dark: dark),
                child: ColoredBox(color: fill, child: child),
              ),
            ),
          );
        },
      );

  ButtonStyle get _onboardingGhostButtonStyle =>
      _onboardingSecondaryButtonStyle.copyWith(
        backgroundColor: const WidgetStatePropertyAll(Colors.transparent),
        side: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.focused)
              ? BorderSide(color: _onboardingInk, width: 2)
              : BorderSide.none,
        ),
      );
}

class _GlassButtonRim extends CustomPainter {
  final bool dark;
  const _GlassButtonRim({required this.dark});

  @override
  void paint(Canvas canvas, Size size) {
    final rect = (Offset.zero & size).deflate(0.5);
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..shader = LinearGradient(
        begin: Alignment.topCenter,
        end: Alignment.bottomCenter,
        colors: dark
            ? const [Color(0x2EFFFFFF), Color(0x0FFFFFFF)]
            : const [Color(0x73FFFFFF), Color(0x1FFFFFFF)],
      ).createShader(rect);
    canvas.drawRRect(
      RRect.fromRectAndRadius(rect, Radius.circular(size.height / 2)),
      paint,
    );
  }

  @override
  bool shouldRepaint(_GlassButtonRim oldDelegate) => oldDelegate.dark != dark;
}
