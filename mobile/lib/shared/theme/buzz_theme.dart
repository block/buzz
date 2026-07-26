import 'package:flutter/material.dart';

/// Name of the first-party Buzz theme. Buzz reuses the GitHub Light palette for
/// every base color; the one thing that sets it apart is a branded gradient
/// painted across the app's top section. Mirrors desktop, where the same
/// gradient fills the sidebar canvas — see `data-buzz-sidebar` in
/// `desktop/src/shared/styles/globals/theme.css`.
const buzzThemeName = 'buzz';

/// Name of the dark counterpart, which reuses the GitHub Dark palette and the
/// dark-tuned gradient stops. Paired with [buzzThemeName] in `themePairs`, so
/// the two behave as a single "Buzz" choice under System mode.
const buzzDarkThemeName = 'buzz-dark';

/// Whether [themeName] is either half of the Buzz pair. Both halves enable the
/// gradient so System mode keeps it on across an OS light/dark switch.
bool isBuzzTheme(String themeName) =>
    themeName == buzzThemeName || themeName == buzzDarkThemeName;

/// Gradient stops, matching desktop's `--buzz-gradient-*` custom properties.
const _lightTop = Color(0xFFE6E6B6);
const _lightBottom = Color(0xFFC4D0DA);
const _darkTop = Color(0xFF4A4616);
const _darkBottom = Color(0xFF0A1423);

/// The Buzz gradient for the app's top section, or null when [themeName] is not
/// a Buzz theme — in which case the section keeps its default frosted fill.
///
/// The stops are fully opaque: under Buzz the color replaces the frosted
/// treatment rather than tinting it, matching desktop's solid sidebar canvas.
///
/// [brightness] comes from the applied color scheme rather than the theme name,
/// so System mode picks the right stops as the OS switches.
LinearGradient? buzzTopSectionGradient(
  String themeName,
  Brightness brightness,
) {
  if (!isBuzzTheme(themeName)) return null;

  final isDark = brightness == Brightness.dark;
  return LinearGradient(
    begin: Alignment.topCenter,
    end: Alignment.bottomCenter,
    colors: [
      isDark ? _darkTop : _lightTop,
      isDark ? _darkBottom : _lightBottom,
    ],
  );
}
