import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'buzz_theme.dart';
import 'theme_catalog.dart';
import 'theme_pairs.dart';

/// Buzz ships as the default: the first-party pair, so a fresh install gets the
/// branded top-section gradient without picking a theme first.
const defaultSchemeName = buzzThemeName;

/// Pre-loaded SharedPreferences instance, overridden in main().
final savedPrefsProvider = Provider<SharedPreferences>(
  (_) => throw UnimplementedError('Must be overridden'),
);

/// Returns a scheme that can honor [mode] without silently pinning brightness.
///
/// System mode only offers paired themes because they have both light and dark
/// variants. Switching to it from an unpaired or unknown selection therefore
/// falls back to the first paired theme, matching the picker ordering.
String? schemeForAppearanceMode(String? schemeName, ThemeMode mode) {
  if (mode != ThemeMode.system) return schemeName;

  final selected = findTheme(schemeName ?? defaultSchemeName);
  if (selected != null && isPairedTheme(selected.name)) return schemeName;

  return themeGroups().paired.firstOrNull?.name ?? schemeName;
}

/// The theme whose colors are actually applied for [schemeName] under [mode].
///
/// In [ThemeMode.system] that is the selected theme itself — its pair covers the
/// other brightness. In the pinned modes it is the theme coerced to the pinned
/// brightness, which is what the picker highlights so the checkmark always
/// tracks what is on screen.
ThemeColors? effectiveTheme(String? schemeName, ThemeMode mode) {
  final selected =
      findTheme(schemeName ?? defaultSchemeName) ??
      findTheme(defaultSchemeName);
  if (selected == null) return null;

  return switch (mode) {
    ThemeMode.system => selected,
    ThemeMode.light => _themeForBrightness(selected, wantDark: false),
    ThemeMode.dark => _themeForBrightness(selected, wantDark: true),
  };
}

/// Coerces [selected] to the requested brightness: itself when it already
/// matches, otherwise its pair, otherwise the first catalog theme of that
/// brightness. Keeps a stored light theme usable after switching to Dark mode
/// without rewriting the user's saved selection.
ThemeColors _themeForBrightness(
  ThemeColors selected, {
  required bool wantDark,
}) {
  if (selected.isDark == wantDark) return selected;

  final pairName = themePairFor(selected.name);
  final paired = pairName == null ? null : findTheme(pairName);
  if (paired != null && paired.isDark == wantDark) return paired;

  // Match desktop's paired-first picker ordering: an unpaired selection falls
  // back through the first-party default pair, not whichever borrowed theme
  // sorts first in the full brightness group.
  final defaultTheme = findTheme(defaultSchemeName);
  if (defaultTheme != null) {
    if (defaultTheme.isDark == wantDark) return defaultTheme;

    final defaultPairName = themePairFor(defaultTheme.name);
    final defaultPair = defaultPairName == null
        ? null
        : findTheme(defaultPairName);
    if (defaultPair != null && defaultPair.isDark == wantDark) {
      return defaultPair;
    }
  }

  final groups = themeGroups();
  final fallback = wantDark ? groups.dark : groups.light;
  return fallback.isEmpty ? selected : fallback.first;
}
