import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'accent_colors.dart';
import 'adaptive_theme.dart';
import 'buzz_theme.dart';
import 'color_scheme.dart';
import 'theme_catalog.dart';
import 'theme_pairs.dart';

const _themeModeKey = 'buzz_theme_mode';
const _accentKey = 'buzz_accent_color';
const _schemeKey = 'buzz_color_scheme';

/// Buzz ships as the default: the first-party pair, so a fresh install gets the
/// branded top-section gradient without picking a theme first.
const defaultSchemeName = buzzThemeName;
const defaultSchemeDisplayName = 'Buzz';

/// Pre-loaded SharedPreferences instance, overridden in main().
final savedPrefsProvider = Provider<SharedPreferences>(
  (_) => throw UnimplementedError('Must be overridden'),
);

/// Tracks the appearance mode: follow the OS, or pin light/dark.
class ThemeNotifier extends Notifier<ThemeMode> {
  @override
  ThemeMode build() {
    final prefs = ref.read(savedPrefsProvider);
    final stored = prefs.getString(_themeModeKey);
    if (stored != null) {
      return ThemeMode.values.where((m) => m.name == stored).firstOrNull ??
          ThemeMode.system;
    }
    return ThemeMode.system;
  }

  void setMode(ThemeMode mode) {
    state = mode;
    ref.read(savedPrefsProvider).setString(_themeModeKey, mode.name);
  }
}

final themeProvider = NotifierProvider<ThemeNotifier, ThemeMode>(
  ThemeNotifier.new,
);

/// Tracks the selected accent color index.
class AccentNotifier extends Notifier<int> {
  @override
  int build() {
    final prefs = ref.read(savedPrefsProvider);
    final stored = prefs.getInt(_accentKey);
    if (stored == legacyDefaultAccentIndex) {
      prefs.setInt(_accentKey, defaultAccentIndex);
      return defaultAccentIndex;
    }
    if (stored == null || stored < 0 || stored >= accentColors.length) {
      return defaultAccentIndex;
    }
    return stored;
  }

  void setAccent(int index) {
    state = index;
    ref.read(savedPrefsProvider).setInt(_accentKey, index);
  }
}

final accentProvider = NotifierProvider<AccentNotifier, int>(
  AccentNotifier.new,
);

/// Tracks the selected color scheme name.
/// null means "use default" ([defaultSchemeDisplayName]).
class SchemeNotifier extends Notifier<String?> {
  @override
  String? build() {
    final prefs = ref.read(savedPrefsProvider);
    final stored = prefs.getString(_schemeKey);
    final compatible = schemeForAppearanceMode(
      stored,
      ref.watch(themeProvider),
    );

    if (compatible != stored) {
      if (compatible == null) {
        prefs.remove(_schemeKey);
      } else {
        prefs.setString(_schemeKey, compatible);
      }
    }

    return compatible;
  }

  void setScheme(String? name) {
    state = name;
    final prefs = ref.read(savedPrefsProvider);
    if (name == null) {
      prefs.remove(_schemeKey);
    } else {
      prefs.setString(_schemeKey, name);
    }
  }
}

final schemeProvider = NotifierProvider<SchemeNotifier, String?>(
  SchemeNotifier.new,
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

/// Resolves the selected scheme and appearance [mode] into light and dark
/// [ColorScheme]s, mirroring desktop's System / Light / Dark behaviour.
///
/// In [ThemeMode.system] a paired theme is rendered with its pair
/// ([themePairFor]) so Flutter swaps them with the OS. An unpaired theme is
/// pinned to its own brightness. In [ThemeMode.light] and [ThemeMode.dark] the
/// theme is coerced to that brightness and the mode is forced, so the OS setting
/// is ignored.
({
  ColorScheme light,
  ColorScheme dark,
  ThemeColors? lightTheme,
  ThemeColors? darkTheme,
  ThemeMode? forcedMode,
})
resolveSchemes(String? schemeName, ThemeMode mode) {
  final selected =
      findTheme(schemeName ?? defaultSchemeName) ??
      findTheme(defaultSchemeName);
  if (selected == null) {
    return (
      light: lightColorScheme,
      dark: darkColorScheme,
      lightTheme: null,
      darkTheme: null,
      forcedMode: null,
    );
  }

  switch (mode) {
    case ThemeMode.system:
      final pairName = themePairFor(selected.name);
      if (pairName == null) {
        final scheme = generateColorScheme(selected);
        return (
          light: scheme,
          dark: scheme,
          lightTheme: selected,
          darkTheme: selected,
          forcedMode: selected.isDark ? ThemeMode.dark : ThemeMode.light,
        );
      }
      final counterpart = findTheme(pairName) ?? selected;
      final lightTheme = selected.isDark ? counterpart : selected;
      final darkTheme = selected.isDark ? selected : counterpart;
      return (
        light: generateColorScheme(lightTheme),
        dark: generateColorScheme(darkTheme),
        lightTheme: lightTheme,
        darkTheme: darkTheme,
        forcedMode: null,
      );
    case ThemeMode.light:
      final theme = _themeForBrightness(selected, wantDark: false);
      final scheme = generateColorScheme(theme);
      return (
        light: scheme,
        dark: scheme,
        lightTheme: theme,
        darkTheme: theme,
        forcedMode: ThemeMode.light,
      );
    case ThemeMode.dark:
      final theme = _themeForBrightness(selected, wantDark: true);
      final scheme = generateColorScheme(theme);
      return (
        light: scheme,
        dark: scheme,
        lightTheme: theme,
        darkTheme: theme,
        forcedMode: ThemeMode.dark,
      );
  }
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

/// Label for the current theme selection. System mode names the pair as a whole
/// ("Github" rather than "Github Light"), since both halves are in play.
String themeSelectionLabel(String? schemeName, ThemeMode mode) {
  final theme = effectiveTheme(schemeName, mode);
  if (theme == null) return defaultSchemeDisplayName;
  if (mode != ThemeMode.system) return theme.displayName;

  final lightMember = themePairs.containsKey(theme.name)
      ? theme.name
      : themePairFor(theme.name);
  return lightMember == null
      ? theme.displayName
      : pairedThemeLabel(lightMember);
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
