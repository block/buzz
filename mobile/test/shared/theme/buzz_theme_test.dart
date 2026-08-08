import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';

void main() {
  group('Buzz theme catalog entries', () {
    test('both halves are in the catalog', () {
      expect(findTheme(buzzThemeName), isNotNull);
      expect(findTheme(buzzDarkThemeName), isNotNull);
    });

    test('borrow the GitHub palettes', () {
      final buzz = findTheme(buzzThemeName)!;
      final github = findTheme('github-light')!;
      expect(buzz.bg, github.bg);
      expect(buzz.fg, github.fg);
      expect(buzz.comment, github.comment);

      final buzzDark = findTheme(buzzDarkThemeName)!;
      final githubDark = findTheme('github-dark')!;
      expect(buzzDark.bg, githubDark.bg);
      expect(buzzDark.fg, githubDark.fg);
      expect(buzzDark.comment, githubDark.comment);
    });

    test('are a light/dark pair', () {
      expect(findTheme(buzzThemeName)!.isDark, isFalse);
      expect(findTheme(buzzDarkThemeName)!.isDark, isTrue);
      expect(themePairFor(buzzThemeName), buzzDarkThemeName);
      expect(themePairFor(buzzDarkThemeName), buzzThemeName);
    });

    test('appear as a single System-mode option labelled "Buzz"', () {
      final paired = themeGroups().paired.map((t) => t.name);
      expect(paired, contains(buzzThemeName));
      expect(paired, isNot(contains(buzzDarkThemeName)));
      expect(pairedThemeLabel(buzzThemeName), 'Buzz');
      expect(themeSelectionLabel(buzzThemeName, ThemeMode.system), 'Buzz');
      expect(themeSelectionLabel(buzzDarkThemeName, ThemeMode.system), 'Buzz');
    });

    test('resolve across brightnesses like any other pair', () {
      final resolved = resolveSchemes(buzzThemeName, ThemeMode.system);
      expect(resolved.forcedMode, isNull);
      expect(resolved.light.brightness, Brightness.light);
      expect(resolved.dark.brightness, Brightness.dark);
      expect(resolved.lightTheme?.name, buzzThemeName);
      expect(resolved.darkTheme?.name, buzzDarkThemeName);

      expect(
        effectiveTheme(buzzThemeName, ThemeMode.dark)?.name,
        buzzDarkThemeName,
      );
      expect(
        effectiveTheme(buzzDarkThemeName, ThemeMode.light)?.name,
        buzzThemeName,
      );
    });

    test(
      'fallbacks expose the effective Buzz theme for gradient selection',
      () {
        final coerced = resolveSchemes('nord', ThemeMode.light);
        expect(coerced.lightTheme?.name, buzzThemeName);
        expect(
          buzzTopSectionGradient(
            coerced.lightTheme!.name,
            coerced.light.brightness,
          ),
          isNotNull,
        );

        final unknown = resolveSchemes('not-a-theme', ThemeMode.light);
        expect(unknown.lightTheme?.name, buzzThemeName);
        expect(
          buzzTopSectionGradient(
            unknown.lightTheme!.name,
            unknown.light.brightness,
          ),
          isNotNull,
        );
      },
    );
  });

  group('buzzTopSectionGradient', () {
    test('is null for non-Buzz themes', () {
      expect(buzzTopSectionGradient('github-light', Brightness.light), isNull);
      expect(buzzTopSectionGradient('nord', Brightness.dark), isNull);
    });

    test('paints top to bottom for both halves of the pair', () {
      for (final name in [buzzThemeName, buzzDarkThemeName]) {
        final gradient = buzzTopSectionGradient(name, Brightness.light);
        expect(gradient, isNotNull, reason: '$name should be gradient-backed');
        expect(gradient!.begin, Alignment.topCenter);
        expect(gradient.end, Alignment.bottomCenter);
        expect(gradient.colors, hasLength(2));
      }
    });

    test('brightness selects the stops, not the theme name', () {
      // Both halves enable the gradient, so System mode keeps it on across an
      // OS switch — the applied brightness alone decides which stops are used.
      final light = buzzTopSectionGradient(buzzThemeName, Brightness.light)!;
      final dark = buzzTopSectionGradient(buzzThemeName, Brightness.dark)!;

      expect(light.colors, isNot(dark.colors));
      expect(
        buzzTopSectionGradient(buzzDarkThemeName, Brightness.dark)!.colors,
        dark.colors,
      );
      expect(
        buzzTopSectionGradient(buzzDarkThemeName, Brightness.light)!.colors,
        light.colors,
      );
    });

    test('is opaque so the color replaces the frosted fill', () {
      for (final brightness in Brightness.values) {
        final gradient = buzzTopSectionGradient(buzzThemeName, brightness)!;
        for (final color in gradient.colors) {
          expect(color.a, 1.0);
        }
      }
    });
  });

  group('theme threading', () {
    BoxDecoration barDecoration(WidgetTester tester) {
      final container = tester
          .widgetList<Container>(
            find.descendant(
              of: find.byType(FrostedAppBar),
              matching: find.byType(Container),
            ),
          )
          .first;
      return container.decoration! as BoxDecoration;
    }

    Widget harness(ThemeData theme) => MaterialApp(
      theme: theme,
      home: Builder(
        builder: (context) => Stack(
          children: [
            FrostedAppBar(
              gradient: context.appColors.topSectionGradient,
              title: const Text('Home'),
            ),
          ],
        ),
      ),
    );

    testWidgets('AppTheme carries the gradient to the top section', (
      tester,
    ) async {
      await tester.pumpWidget(
        harness(
          AppTheme.light(
            topSectionGradient: buzzTopSectionGradient(
              buzzThemeName,
              Brightness.light,
            ),
          ),
        ),
      );

      final decoration = barDecoration(tester);
      expect(decoration.gradient, isNotNull);
      // A BoxDecoration cannot paint a color and a gradient at once.
      expect(decoration.color, isNull);
    });

    testWidgets('non-Buzz themes keep the frosted surface fill', (
      tester,
    ) async {
      await tester.pumpWidget(harness(AppTheme.light()));

      final decoration = barDecoration(tester);
      expect(decoration.gradient, isNull);
      expect(decoration.color, isNotNull);
    });
  });

  group('isBuzzTheme', () {
    test('matches only the Buzz pair', () {
      expect(isBuzzTheme(buzzThemeName), isTrue);
      expect(isBuzzTheme(buzzDarkThemeName), isTrue);
      expect(isBuzzTheme('github-light'), isFalse);
      expect(isBuzzTheme(''), isFalse);
    });
  });
}
