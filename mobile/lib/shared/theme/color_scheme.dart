import 'package:flutter/material.dart';

import 'accent_colors.dart';

// Catppuccin Latte (mauve accent) — matches Buzz desktop light theme
const lightColorScheme = ColorScheme(
  brightness: Brightness.light,
  primary: Color(0xFF8839EF), // Latte Mauve
  onPrimary: Color(0xFFEFF1F5), // Latte Base
  primaryContainer: Color(0xFFE6E9EF), // Latte Mantle
  onPrimaryContainer: Color(0xFF4C4F69), // Latte Text
  secondary: Color(0xFF6C6F85), // Latte Subtext0
  onSecondary: Color(0xFFFFFFFF),
  secondaryContainer: Color(0xFFCCD0DA), // Latte Surface1
  onSecondaryContainer: Color(0xFF4C4F69), // Latte Text
  tertiary: Color(0xFF1E66F5), // Latte Blue
  onTertiary: Color(0xFFFFFFFF),
  tertiaryContainer: Color(0xFFBCC0CC), // Latte Surface2
  onTertiaryContainer: Color(0xFF1E66F5), // Latte Blue
  error: Color(0xFFD20F39), // Latte Red
  onError: Color(0xFFFFFFFF),
  errorContainer: Color(0xFFFDD8E0),
  onErrorContainer: Color(0xFFD20F39),
  surface: Color(0xFFFFFFFF),
  onSurface: Color(0xFF4C4F69), // Latte Text
  onSurfaceVariant: Color(0xFF6C6F85), // Latte Subtext0
  outline: Color(0xFFBCC0CC), // Latte Surface2
  outlineVariant: Color(0xFFCCD0DA), // Latte Surface1
  inverseSurface: Color(0xFF4C4F69), // Latte Text
  onInverseSurface: Color(0xFFEFF1F5), // Latte Base
  inversePrimary: Color(0xFFA875F5), // Macchiato Mauve (saturated)
  shadow: Color(0xFF000000),
  scrim: Color(0xFF000000),
  surfaceTint: Color(0xFF8839EF), // Latte Mauve
  surfaceContainerHighest: Color(0xFFFFFFFF),
);

// Catppuccin Macchiato (mauve accent) — matches Buzz desktop dark theme
const darkColorScheme = ColorScheme(
  brightness: Brightness.dark,
  primary: Color(0xFFA875F5), // Macchiato Mauve (saturated)
  onPrimary: Color(0xFF24273A), // Macchiato Base
  primaryContainer: Color(0xFF363A4F), // Macchiato Surface0
  onPrimaryContainer: Color(0xFFCAD3F5), // Macchiato Text
  secondary: Color(0xFFA5ADCB), // Macchiato Subtext0
  onSecondary: Color(0xFF24273A), // Macchiato Base
  secondaryContainer: Color(0xFF494D64), // Macchiato Surface1
  onSecondaryContainer: Color(0xFFCAD3F5), // Macchiato Text
  tertiary: Color(0xFF8AADF4), // Macchiato Blue
  onTertiary: Color(0xFF24273A), // Macchiato Base
  tertiaryContainer: Color(0xFF363A4F), // Macchiato Surface0
  onTertiaryContainer: Color(0xFF8AADF4), // Macchiato Blue
  error: Color(0xFFED8796), // Macchiato Red
  onError: Color(0xFF24273A), // Macchiato Base
  errorContainer: Color(0xFF3D2030),
  onErrorContainer: Color(0xFFED8796),
  surface: Color(0xFF24273A), // Macchiato Base
  onSurface: Color(0xFFCAD3F5), // Macchiato Text
  onSurfaceVariant: Color(0xFFA5ADCB), // Macchiato Subtext0
  outline: Color(0xFF494D64), // Macchiato Surface1
  outlineVariant: Color(0xFF363A4F), // Macchiato Surface0
  inverseSurface: Color(0xFFCAD3F5), // Macchiato Text
  onInverseSurface: Color(0xFF24273A), // Macchiato Base
  inversePrimary: Color(0xFF8839EF), // Latte Mauve
  shadow: Color(0xFF000000),
  scrim: Color(0xFF000000),
  surfaceTint: Color(0xFFA875F5), // Macchiato Mauve (saturated)
  surfaceContainerHighest: Color(0xFF1E2030), // Macchiato Mantle
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
