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

    test('groups are sorted by display name', () {
      final groups = themeGroups();

      for (final list in [groups.paired, groups.light, groups.dark]) {
        final names = list.map((t) => t.displayName).toList();
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

  group('resolveSchemes', () {
    test('system mode pairs the selection across brightnesses', () {
      final resolved = resolveSchemes('github-light', ThemeMode.system);

      // Null forcedMode leaves MaterialApp following the OS.
      expect(resolved.forcedMode, isNull);
      expect(resolved.light.brightness, Brightness.light);
      expect(resolved.dark.brightness, Brightness.dark);
    });

    test('system mode works from the dark half of a pair too', () {
      final resolved = resolveSchemes('github-dark', ThemeMode.system);

      expect(resolved.forcedMode, isNull);
      expect(resolved.light.brightness, Brightness.light);
      expect(resolved.dark.brightness, Brightness.dark);
    });

    test('system mode pins an unpaired theme to its own brightness', () {
      final resolved = resolveSchemes('andromeeda', ThemeMode.system);

      // No counterpart exists, so both sides render the selected theme rather
      // than jumping to an unrelated one.
      expect(resolved.forcedMode, ThemeMode.dark);
      expect(resolved.light, resolved.dark);
      expect(resolved.dark.brightness, Brightness.dark);
    });

    test('system mode pins an unpaired light theme to light mode', () {
      final resolved = resolveSchemes('snazzy-light', ThemeMode.system);

      expect(resolved.forcedMode, ThemeMode.light);
      expect(resolved.light, resolved.dark);
      expect(resolved.light.brightness, Brightness.light);
    });

    test('light mode forces light and swaps a dark selection for its pair', () {
      final resolved = resolveSchemes('github-dark', ThemeMode.light);

      expect(resolved.forcedMode, ThemeMode.light);
      expect(resolved.light, resolved.dark);
      expect(resolved.light.brightness, Brightness.light);
      expect(resolved.light, generateColorScheme(findTheme('github-light')!));
    });

    test('dark mode forces dark and swaps a light selection for its pair', () {
      final resolved = resolveSchemes('github-light', ThemeMode.dark);

      expect(resolved.forcedMode, ThemeMode.dark);
      expect(resolved.dark.brightness, Brightness.dark);
      expect(resolved.dark, generateColorScheme(findTheme('github-dark')!));
    });

    test('dark mode falls back to the default pair when pick is unpaired', () {
      // 'snazzy-light' is light and has no dark counterpart.
      final resolved = resolveSchemes('snazzy-light', ThemeMode.dark);

      expect(resolved.forcedMode, ThemeMode.dark);
      expect(resolved.dark.brightness, Brightness.dark);
      expect(resolved.darkTheme?.name, buzzDarkThemeName);
      expect(resolved.dark, generateColorScheme(findTheme(buzzDarkThemeName)!));
    });

    test('light mode falls back to the default pair when pick is unpaired', () {
      final resolved = resolveSchemes('andromeeda', ThemeMode.light);

      expect(resolved.forcedMode, ThemeMode.light);
      expect(resolved.light.brightness, Brightness.light);
      expect(resolved.lightTheme?.name, buzzThemeName);
      expect(resolved.light, generateColorScheme(findTheme(buzzThemeName)!));
    });

    test('an unknown scheme name falls back to the default theme', () {
      final resolved = resolveSchemes('not-a-theme', ThemeMode.light);

      expect(
        resolved.light,
        generateColorScheme(findTheme(defaultSchemeName)!),
      );
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

  group('themeSelectionLabel', () {
    test('names the pair as a whole in system mode', () {
      expect(themeSelectionLabel('github-light', ThemeMode.system), 'Github');
      expect(themeSelectionLabel('github-dark', ThemeMode.system), 'Github');
    });

    test('names the concrete theme in pinned modes', () {
      expect(
        themeSelectionLabel('github-light', ThemeMode.light),
        'Github Light',
      );
      expect(
        themeSelectionLabel('github-light', ThemeMode.dark),
        'Github Dark',
      );
    });

    test('falls back to the theme display name when unpaired', () {
      expect(themeSelectionLabel('andromeeda', ThemeMode.system), 'Andromeeda');
    });
  });
}
