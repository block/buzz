import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';

void main() {
  group('themePairs', () {
    test('every pair member exists in the catalog', () {
      for (final entry in themePairs.entries) {
        expect(
          findTheme(entry.key),
          isNotNull,
          reason: '${entry.key} is not in themeCatalog',
        );
        expect(
          findTheme(entry.value),
          isNotNull,
          reason: '${entry.value} is not in themeCatalog',
        );
      }
    });

    test('pairs map a light theme to a dark one', () {
      for (final entry in themePairs.entries) {
        expect(
          findTheme(entry.key)!.isDark,
          isFalse,
          reason: '${entry.key} should be light',
        );
        expect(
          findTheme(entry.value)!.isDark,
          isTrue,
          reason: '${entry.value} should be dark',
        );
      }
    });

    test('resolves in both directions', () {
      expect(themePairFor('github-light'), 'github-dark');
      expect(themePairFor('github-dark'), 'github-light');
      expect(themePairFor('andromeeda'), isNull);
      expect(isPairedTheme('catppuccin-latte'), isTrue);
      expect(isPairedTheme('andromeeda'), isFalse);
    });
  });

  group('themeGroups', () {
    test('light and dark partition the whole catalog', () {
      final groups = themeGroups();

      expect(groups.light.length + groups.dark.length, themeCatalog.length);
      expect(groups.light.every((t) => !t.isDark), isTrue);
      expect(groups.dark.every((t) => t.isDark), isTrue);
    });

    test('paired holds only light members of pairs', () {
      final groups = themeGroups();

      expect(groups.paired.length, themePairs.length);
      for (final theme in groups.paired) {
        expect(theme.isDark, isFalse);
        expect(themePairs.containsKey(theme.name), isTrue);
      }
    });

    test('Buzz is first and remaining themes are sorted by display name', () {
      final groups = themeGroups();

      expect(groups.paired.first.name, buzzThemeName);
      expect(groups.light.first.name, buzzThemeName);
      expect(groups.dark.first.name, buzzDarkThemeName);

      for (final list in [groups.paired, groups.light, groups.dark]) {
        final names = list.skip(1).map((t) => t.displayName).toList();
        expect(names, orderedEquals(List.of(names)..sort()));
      }
    });
  });

  group('pairedThemeLabel', () {
    test('strips brightness tokens', () {
      expect(pairedThemeLabel('github-light'), 'Github');
      expect(pairedThemeLabel('github-light-default'), 'Github Default');
      expect(pairedThemeLabel('gruvbox-light-soft'), 'Gruvbox Soft');
      expect(pairedThemeLabel('material-theme-lighter'), 'Material Theme');
      expect(pairedThemeLabel('catppuccin-latte'), 'Catppuccin');
    });

    test('falls back to the raw name when stripping empties it', () {
      expect(pairedThemeLabel('light-plus'), 'Light Plus');
    });

    test('never produces a blank label for a real pair', () {
      for (final lightName in themePairs.keys) {
        expect(pairedThemeLabel(lightName), isNotEmpty);
      }
    });
  });

  group('schemeForAppearanceMode', () {
    test('system mode replaces an unpaired selection with a paired theme', () {
      expect(
        schemeForAppearanceMode('snazzy-light', ThemeMode.system),
        themeGroups().paired.first.name,
      );
      expect(
        schemeForAppearanceMode('nord', ThemeMode.system),
        themeGroups().paired.first.name,
      );
    });

    test('system mode preserves either half of a paired selection', () {
      expect(
        schemeForAppearanceMode('github-light', ThemeMode.system),
        'github-light',
      );
      expect(
        schemeForAppearanceMode('github-dark', ThemeMode.system),
        'github-dark',
      );
    });

    test('pinned modes preserve an unpaired selection', () {
      expect(
        schemeForAppearanceMode('snazzy-light', ThemeMode.light),
        'snazzy-light',
      );
      expect(schemeForAppearanceMode('nord', ThemeMode.dark), 'nord');
    });
  });

  group('effectiveTheme', () {
    test('system mode keeps the stored selection', () {
      expect(
        effectiveTheme('github-dark', ThemeMode.system)?.name,
        'github-dark',
      );
    });

    test('pinned modes report the coerced theme', () {
      expect(
        effectiveTheme('github-dark', ThemeMode.light)?.name,
        'github-light',
      );
      expect(
        effectiveTheme('github-light', ThemeMode.dark)?.name,
        'github-dark',
      );
    });

    test('a null selection resolves to the default theme', () {
      expect(effectiveTheme(null, ThemeMode.light)?.name, defaultSchemeName);
    });
  });
}
