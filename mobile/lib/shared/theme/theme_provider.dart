import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'accent_colors.dart';
import 'adaptive_theme.dart';
import 'color_scheme.dart';
import 'theme_catalog.dart';

const _themeModeKey = 'buzz_theme_mode';
const _accentKey = 'buzz_accent_color';
const _schemeKey = 'buzz_color_scheme';

const defaultSchemeName = 'github-light';
const defaultSchemeDisplayName = 'GitHub Light';

/// Pre-loaded SharedPreferences instance, overridden in main().
final savedPrefsProvider = Provider<SharedPreferences>(
  (_) => throw UnimplementedError('Must be overridden'),
);

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
    final scheme = prefs.getString(_schemeKey);

    // One-time migration: if no scheme has been chosen and the user had a
    // non-system themeMode persisted (from the old Light/System/Dark toggle),
    // reset it because default schemes now control their own brightness.
    if (scheme == null) {
      final storedMode = prefs.getString(_themeModeKey);
      if (storedMode != null && storedMode != ThemeMode.system.name) {
        prefs.setString(_themeModeKey, ThemeMode.system.name);
      }
    }

    return scheme;
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

/// Resolves the current scheme selection into light and dark [ColorScheme]s.
/// When a named scheme is selected, generates it via the adaptive engine.
/// When null (default), resolves to [defaultSchemeName].
({ColorScheme light, ColorScheme dark, ThemeMode? forcedMode}) resolveSchemes(
  String? schemeName,
) {
  final theme =
      findTheme(schemeName ?? defaultSchemeName) ??
      findTheme(defaultSchemeName);
  if (theme == null) {
    return (light: lightColorScheme, dark: darkColorScheme, forcedMode: null);
  }

  final scheme = generateColorScheme(theme);

  // A named scheme is inherently light or dark — force the mode.
  return (
    light: scheme,
    dark: scheme,
    forcedMode: theme.isDark ? ThemeMode.dark : ThemeMode.light,
  );
}
