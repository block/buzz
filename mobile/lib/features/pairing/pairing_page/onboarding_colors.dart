part of '../pairing_page.dart';

// Foregrounds for the fixed Buzz shell artwork, independent of accent choice.
extension _OnboardingColors on BuildContext {
  bool get _onboardingIsDark => Theme.of(this).brightness == Brightness.dark;
  Color get _onboardingInk =>
      _onboardingIsDark ? const Color(0xFFE6EDF0) : const Color(0xFF111111);
  Color get _onboardingMutedInk =>
      _onboardingIsDark ? const Color(0xFFAAB8C0) : const Color(0xB3111111);
  Color get _onboardingCtaLabel =>
      _onboardingIsDark ? const Color(0xFF172229) : const Color(0xFFD7E6F0);
  Color get _onboardingErrorInk => colors.error;
  Color get _onboardingInputSurface => _onboardingIsDark
      ? const Color(0xFF233039)
      : Colors.white.withValues(alpha: 0.7);

  OutlineInputBorder get _inputBorder => OutlineInputBorder(
    borderRadius: BorderRadius.circular(Radii.md),
    borderSide: BorderSide(color: _onboardingInk.withValues(alpha: 0.18)),
  );

  ButtonStyle get _onboardingButtonStyle => FilledButton.styleFrom(
    minimumSize: const Size(0, 48),
    padding: const EdgeInsets.symmetric(
      horizontal: Grid.lg,
      vertical: Grid.twelve,
    ),
    backgroundColor: _onboardingInk,
    foregroundColor: _onboardingCtaLabel,
    disabledBackgroundColor: _onboardingInk.withValues(alpha: 0.38),
    disabledForegroundColor: _onboardingCtaLabel.withValues(alpha: 0.7),
    shape: const StadiumBorder(),
  );

  ButtonStyle get _onboardingSecondaryButtonStyle => TextButton.styleFrom(
    minimumSize: const Size(0, 44),
    padding: const EdgeInsets.symmetric(
      horizontal: Grid.md,
      vertical: Grid.xxs,
    ),
    backgroundColor: _onboardingInk.withValues(alpha: 0.1),
    foregroundColor: _onboardingInk,
    disabledBackgroundColor: _onboardingInk.withValues(alpha: 0.05),
    disabledForegroundColor: _onboardingInk.withValues(alpha: 0.45),
    shape: const StadiumBorder(),
  );
}
