import 'package:flutter/material.dart';

import 'accent_colors.dart';
import 'app_colors.dart';

/// Legacy storage name for the first-party Zorro light theme. Its branded
/// gradient mirrors the desktop sidebar canvas.
const zorroThemeName = 'zorro';

/// Legacy storage name for the Zorro dark counterpart. Paired with
/// [zorroThemeName], the two behave as a single "Zorro" System-mode choice.
const zorroDarkThemeName = 'zorro-dark';

/// Whether [themeName] is either half of the Zorro pair. Both halves enable the
/// gradient so System mode keeps it on across an OS light/dark switch.
bool isZorroTheme(String themeName) =>
    themeName == zorroThemeName || themeName == zorroDarkThemeName;

/// Whether the current widget tree is using the first-party Zorro treatment.
bool isZorroThemeContext(BuildContext context) =>
    Theme.of(context).extension<AppColors>()?.topSectionGradient != null;

/// Primary foreground for the mobile top navigation.
///
/// Every theme uses its own [ColorScheme.onSurface]. Zorro is the exception:
/// its branded top gradient needs a neutral black or white foreground rather
/// than the accent-derived color scheme foreground.
Color navigationPrimaryForeground(BuildContext context) {
  final scheme = Theme.of(context).colorScheme;
  if (!isZorroThemeContext(context)) return scheme.onSurface;
  return scheme.brightness == Brightness.dark ? Colors.white : Colors.black;
}

/// Secondary label and placeholder foreground for the mobile top navigation.
Color navigationSecondaryForeground(BuildContext context) {
  final scheme = Theme.of(context).colorScheme;
  if (!isZorroThemeContext(context)) return scheme.onSurfaceVariant;
  return navigationPrimaryForeground(context).withValues(alpha: 0.4);
}

/// Channel-section label and icon foreground for the mobile side navigation.
Color navigationSectionForeground(BuildContext context) {
  final scheme = Theme.of(context).colorScheme;
  if (!isZorroThemeContext(context)) return scheme.onSurfaceVariant;
  return navigationPrimaryForeground(context).withValues(alpha: 0.8);
}

/// Search-field surface for the mobile top navigation.
Color navigationSearchSurface(BuildContext context) {
  final scheme = Theme.of(context).colorScheme;
  if (!isZorroThemeContext(context)) return scheme.surfaceContainerHighest;
  return navigationPrimaryForeground(context).withValues(alpha: 0.04);
}

/// A low-contrast navigation divider derived from the active theme foreground.
Color navigationDivider(BuildContext context, double opacity) =>
    navigationPrimaryForeground(context).withValues(alpha: opacity);

/// Zorro renders with a neutral foreground while preserving the stored accent.
int effectiveAccentIndex(String themeName, String storedAccent) {
  if (isZorroTheme(themeName)) return neutralAccentIndex;
  return accentIndexForWireValue(storedAccent) ?? defaultAccentIndex;
}

/// Gradient stops, matching desktop's Zorro custom properties.
const _lightTop = Color(0xFFFFFAF2);
const _lightBottom = Color(0xFFFFDFC2);
const _darkTop = Color(0xFF260001);
const _darkBottom = Color(0xFF260001);

/// The Zorro gradient for the app's top section, or null when [themeName] is
/// not a Zorro theme — in which case the section keeps its default frosted fill.
///
/// The stops are fully opaque: under Zorro the color replaces the frosted
/// treatment rather than tinting it, matching desktop's solid sidebar canvas.
///
/// [brightness] comes from the applied color scheme rather than the theme name,
/// so System mode picks the right stops as the OS switches.
LinearGradient? zorroTopSectionGradient(
  String themeName,
  Brightness brightness,
) {
  if (!isZorroTheme(themeName)) return null;

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
