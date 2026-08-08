import 'theme_catalog.dart';

/// Light → dark theme counterparts, ported from the desktop app's `THEME_PAIRS`
/// (`desktop/src/shared/theme/theme-loader.ts`) so both clients offer the same
/// System-mode pairings.
///
/// Buzz leads the map the way it leads desktop's, so the first-party pair sorts
/// ahead of the borrowed syntax themes wherever insertion order is preserved.
const themePairs = <String, String>{
  'buzz': 'buzz-dark',
  'catppuccin-latte': 'catppuccin-mocha',
  'everforest-light': 'everforest-dark',
  'github-light': 'github-dark',
  'github-light-default': 'github-dark-default',
  'github-light-high-contrast': 'github-dark-high-contrast',
  'gruvbox-light-hard': 'gruvbox-dark-hard',
  'gruvbox-light-medium': 'gruvbox-dark-medium',
  'gruvbox-light-soft': 'gruvbox-dark-soft',
  'kanagawa-lotus': 'kanagawa-wave',
  'light-plus': 'dark-plus',
  'material-theme-lighter': 'material-theme',
  'min-light': 'min-dark',
  'one-light': 'one-dark-pro',
  'rose-pine-dawn': 'rose-pine',
  'slack-ochin': 'slack-dark',
  'solarized-light': 'solarized-dark',
  'vitesse-light': 'vitesse-dark',
};

final Map<String, String> _darkToLight = {
  for (final entry in themePairs.entries) entry.value: entry.key,
};

/// The counterpart of [name] in the opposite brightness, or null when the theme
/// has no pair. Resolves in both directions.
String? themePairFor(String name) => themePairs[name] ?? _darkToLight[name];

/// Whether [name] participates in a light/dark pair, and so can follow the OS.
bool isPairedTheme(String name) => themePairFor(name) != null;

/// [themeCatalog] partitioned the way the appearance picker offers it.
class ThemeGroups {
  /// Light members of light/dark pairs — the System-mode options. Each stands
  /// in for both halves of its pair.
  final List<ThemeColors> paired;

  /// Every light theme, paired or not — the Light-mode options.
  final List<ThemeColors> light;

  /// Every dark theme, paired or not — the Dark-mode options.
  final List<ThemeColors> dark;

  const ThemeGroups({
    required this.paired,
    required this.light,
    required this.dark,
  });
}

ThemeGroups? _groups;

/// [themeCatalog] grouped for the appearance picker, each list sorted by
/// display name. Computed once — the catalog is a compile-time constant.
ThemeGroups themeGroups() {
  final cached = _groups;
  if (cached != null) return cached;

  final paired = <ThemeColors>[];
  final light = <ThemeColors>[];
  final dark = <ThemeColors>[];

  for (final theme in themeCatalog) {
    if (theme.isDark) {
      dark.add(theme);
      continue;
    }
    light.add(theme);
    if (themePairs.containsKey(theme.name)) paired.add(theme);
  }

  int byDisplayName(ThemeColors a, ThemeColors b) =>
      a.displayName.compareTo(b.displayName);
  paired.sort(byDisplayName);
  light.sort(byDisplayName);
  dark.sort(byDisplayName);

  return _groups = ThemeGroups(paired: paired, light: light, dark: dark);
}

/// Tokens that only describe a theme's brightness, stripped from paired labels.
const _modeTokens = <String>{
  'light',
  'latte',
  'dawn',
  'lotus',
  'ochin',
  'lighter',
  'plus',
};

/// Display label for a pair, derived from its light member: strips the
/// mode-specific tokens so `github-light` reads as "Github" and stands for both
/// halves. Mirrors desktop's `pairedThemeLabel`.
String pairedThemeLabel(String lightName) {
  final stripped = lightName
      .split('-')
      .where((token) => !_modeTokens.contains(token))
      .toList();
  // Stripping can remove everything (e.g. "light-plus") — fall back to the raw
  // name so the row is never blank.
  final parts = stripped.isEmpty ? lightName.split('-') : stripped;
  return parts
      .map((w) => w.isEmpty ? w : '${w[0].toUpperCase()}${w.substring(1)}')
      .join(' ');
}
