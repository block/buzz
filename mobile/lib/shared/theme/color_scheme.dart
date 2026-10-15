import 'package:flutter/material.dart';

import 'accent_colors.dart';

// Buzz desktop design-system tokens: neutral surfaces/text and Radix status roles.
// Source: buzz-app/src/shared/design-system/styles/tokens.css.
const lightColorScheme = ColorScheme(
  brightness: Brightness.light,
  primary: Color(0xFF262626),
  onPrimary: Color(0xFFFFFFFF),
  primaryContainer: Color(0xFFF0F0F0),
  onPrimaryContainer: Color(0xFF0A0A0A),
  secondary: Color(0xFF646464),
  onSecondary: Color(0xFFFFFFFF),
  secondaryContainer: Color(0xFFF0F0F0),
  onSecondaryContainer: Color(0xFF0A0A0A),
  tertiary: Color(0xFF0D74CE),
  onTertiary: Color(0xFFFFFFFF),
  tertiaryContainer: Color(0xFFE6F4FE),
  onTertiaryContainer: Color(0xFF113264),
  error: Color(0xFFCE2C31),
  onError: Color(0xFFFFFFFF),
  errorContainer: Color(0xFFFEEBEC),
  onErrorContainer: Color(0xFF641723),
  surface: Color(0xFFFFFFFF),
  onSurface: Color(0xFF0A0A0A),
  onSurfaceVariant: Color(0xFF646464),
  outline: Color(0xFFE8E8E8),
  outlineVariant: Color(0xFFE8E8E8),
  inverseSurface: Color(0xFF262626),
  onInverseSurface: Color(0xFFFFFFFF),
  inversePrimary: Color(0xFFE5E5E5),
  shadow: Color(0xFF000000),
  scrim: Color(0xFF000000),
  surfaceContainerLowest: Color(0xFFFFFFFF),
  surfaceContainerLow: Color(0xFFF5F5F6),
  surfaceContainer: Color(0xFFF0F0F0),
  surfaceContainerHigh: Color(0xFFE8E8E8),
  surfaceContainerHighest: Color(0xFFF5F5F6),
  surfaceTint: Colors.transparent,
);

const darkColorScheme = ColorScheme(
  brightness: Brightness.dark,
  primary: Color(0xFFE5E5E5),
  onPrimary: Color(0xFF0F0F0F),
  primaryContainer: Color(0xFF2A2A2A),
  onPrimaryContainer: Color(0xFFF5F5F5),
  secondary: Color(0xFFC1C1C1),
  onSecondary: Color(0xFF0F0F0F),
  secondaryContainer: Color(0xFF2A2A2A),
  onSecondaryContainer: Color(0xFFF5F5F5),
  tertiary: Color(0xFF70B8FF),
  onTertiary: Color(0xFF0F0F0F),
  tertiaryContainer: Color(0xFF0D2847),
  onTertiaryContainer: Color(0xFFC2E6FF),
  error: Color(0xFFFF9592),
  onError: Color(0xFF191111),
  errorContainer: Color(0xFF3B1219),
  onErrorContainer: Color(0xFFFFD1D9),
  surface: Color(0xFF1C1C1C),
  onSurface: Color(0xFFF5F5F5),
  onSurfaceVariant: Color(0xFFC1C1C1),
  outline: Color(0xFF333333),
  outlineVariant: Color(0xFF333333),
  inverseSurface: Color(0xFFE5E5E5),
  onInverseSurface: Color(0xFF0F0F0F),
  inversePrimary: Color(0xFF262626),
  shadow: Color(0xFF000000),
  scrim: Color(0xFF000000),
  surfaceContainerLowest: Color(0xFF0F0F0F),
  surfaceContainerLow: Color(0xFF161616),
  surfaceContainer: Color(0xFF232323),
  surfaceContainerHigh: Color(0xFF2A2A2A),
  surfaceContainerHighest: Color(0xFF2A2A2A),
  surfaceTint: Colors.transparent,
);

/// Compute a contrast-safe foreground color for a given background.
/// Uses WCAG contrast ratio (higher ratio wins) instead of a simple luminance
/// cutoff, so colors like Blue (#3B82F6) correctly get black text (5.7:1)
/// rather than white (3.7:1).
Color contrastForeground(Color bg) {
  final lum = bg.computeLuminance();
  // WCAG contrast ratio: (L1 + 0.05) / (L2 + 0.05), L1 >= L2
  final contrastWithBlack = (lum + 0.05) / 0.05; // black luminance = 0
  final contrastWithWhite = 1.05 / (lum + 0.05); // white luminance = 1
  return contrastWithBlack >= contrastWithWhite
      ? const Color(0xFF000000)
      : const Color(0xFFFFFFFF);
}

/// Returns a [ColorScheme] with the given accent applied as primary.
ColorScheme applyAccent(ColorScheme base, int accentIndex) {
  if (accentIndex < 0 || accentIndex >= accentColors.length) {
    return base;
  }
  final color = accentColorForScheme(base, accentIndex);
  final onColor = contrastForeground(color);

  return base.copyWith(primary: color, onPrimary: onColor, surfaceTint: color);
}
