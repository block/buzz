// Adaptive Theme Engine
//
// Derives a Material 3 [ColorScheme] from a syntax theme's key colors
// (bg, fg, comment, git). Detects light vs dark from background luminance
// and adjusts accordingly.
//
// Ported from desktop/src/shared/theme/adaptive-theme.ts.
import 'dart:math' as math;
import 'package:flutter/material.dart';

import 'color_scheme.dart' show contrastForeground;
import 'theme_catalog.dart';

double _luminance(Color c) => c.computeLuminance();

int _to255(double v) => (v * 255.0).round().clamp(0, 255);

Color _mix(Color c1, Color c2, double factor) {
  final r = c1.r + (c2.r - c1.r) * factor;
  final g = c1.g + (c2.g - c1.g) * factor;
  final b = c1.b + (c2.b - c1.b) * factor;
  return Color.fromARGB(255, _to255(r), _to255(g), _to255(b));
}

Color _adjust(Color c, double amount) {
  final target = amount > 0 ? const Color(0xFFFFFFFF) : const Color(0xFF000000);
  return _mix(c, target, amount.abs());
}

const _contrastValue = 0.035;
const _contrastOffset = 0.0135;

double _calculateLumDiff(double bgLum) {
  return _contrastValue * math.log(1 + (bgLum + _contrastOffset) * 10);
}

Color _findColorWithLuminance(Color base, double targetLum) {
  final baseLum = _luminance(base);
  if ((baseLum - targetLum).abs() < 0.001) return base;

  final target = targetLum < baseLum
      ? const Color(0xFF000000)
      : const Color(0xFFFFFFFF);
  var lo = 0.0;
  var hi = 1.0;

  for (var i = 0; i < 20; i++) {
    final mid = (lo + hi) / 2;
    final testLum = _luminance(_mix(base, target, mid));
    final diff = testLum - targetLum;

    if (diff.abs() < 0.001) break;

    if (target == const Color(0xFF000000)) {
      if (testLum > targetLum) {
        lo = mid;
      } else {
        hi = mid;
      }
    } else {
      if (testLum < targetLum) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
  }
  return _mix(base, target, (lo + hi) / 2);
}

({Color chrome, Color primary}) _calculateChromeColors(Color syntaxBg) {
  final bgLum = _luminance(syntaxBg);
  final lumDiff = _calculateLumDiff(bgLum);
  final targetChromeLum = bgLum - lumDiff;

  if (targetChromeLum >= 0) {
    return (
      chrome: _findColorWithLuminance(syntaxBg, targetChromeLum),
      primary: syntaxBg,
    );
  }

  return (
    chrome: _findColorWithLuminance(syntaxBg, 0),
    primary: _findColorWithLuminance(syntaxBg, lumDiff),
  );
}

/// Generate a full Material 3 [ColorScheme] from syntax theme colors.
///
/// Maps the desktop's CSS variable system to Material 3 semantic slots:
///   --background     → surface
///   --foreground     → onSurface
///   --muted          → surfaceContainerHighest
///   --muted-fg       → onSurfaceVariant
///   --border         → outline
///   --popover        → surfaceContainerHigh (elevated surfaces)
///   --destructive    → error
///   --sidebar-bg     → surfaceContainerLowest (chrome)
ColorScheme generateColorScheme(ThemeColors theme) {
  final isDark = theme.isDark;
  final syntaxBg = theme.bg;
  final syntaxFg = theme.fg;
  final syntaxComment = theme.comment;

  final (:chrome, :primary) = _calculateChromeColors(syntaxBg);

  final dir = isDark ? 1.0 : -1.0;
  Color elevate(double amount) => _adjust(primary, dir * amount);

  // Fallback git/accent colors
  final fallbackGreen = isDark
      ? const Color(0xFF3FB950)
      : const Color(0xFF1A7F37);
  final fallbackRed = isDark
      ? const Color(0xFFF85149)
      : const Color(0xFFCF222E);

  final accentGreen = theme.added ?? fallbackGreen;
  final accentRed = theme.deleted ?? fallbackRed;

  // Derived surfaces
  final borderColor = _mix(primary, syntaxFg, isDark ? 0.15 : 0.12);
  final hoverBg = elevate(0.06);
  final popoverBg = elevate(0.08);

  return ColorScheme(
    brightness: isDark ? Brightness.dark : Brightness.light,

    // Primary — use the theme fg as a muted "primary" to keep the theme
    // feeling cohesive. Accent color override happens in applyAccent().
    primary: syntaxFg,
    onPrimary: contrastForeground(syntaxFg),
    primaryContainer: hoverBg,
    onPrimaryContainer: syntaxFg,

    // Secondary
    secondary: syntaxComment,
    onSecondary: contrastForeground(syntaxComment),
    secondaryContainer: hoverBg,
    onSecondaryContainer: syntaxFg,

    // Tertiary
    tertiary: accentGreen,
    onTertiary: contrastForeground(accentGreen),
    tertiaryContainer: hoverBg,
    onTertiaryContainer: accentGreen,

    // Error / destructive
    error: accentRed,
    onError: contrastForeground(accentRed),
    errorContainer: _mix(primary, accentRed, 0.15),
    onErrorContainer: accentRed,

    // Surfaces
    surface: primary,
    onSurface: syntaxFg,
    onSurfaceVariant: syntaxComment,

    // Outline / borders
    outline: borderColor,
    outlineVariant: _mix(primary, borderColor, 0.5),

    // Inverse
    inverseSurface: syntaxFg,
    onInverseSurface: primary,
    inversePrimary: syntaxComment,

    // Shadow / scrim
    shadow: const Color(0xFF000000),
    scrim: const Color(0xFF000000),
    surfaceTint: syntaxFg,

    // Container hierarchy
    surfaceContainerLowest: chrome,
    surfaceContainerLow: _mix(chrome, primary, 0.5),
    surfaceContainer: primary,
    surfaceContainerHigh: popoverBg,
    surfaceContainerHighest: elevate(0.04),
  );
}
