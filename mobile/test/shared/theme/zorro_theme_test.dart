import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';

void main() {
  group('Zorro theme catalog entries', () {
    test('use the product palette and form the default pair', () {
      final light = findTheme(zorroThemeName)!;
      final dark = findTheme(zorroDarkThemeName)!;

      expect(defaultSchemeName, zorroThemeName);
      expect(light.bg, const Color(0xFFFFF8ED));
      expect(light.fg, const Color(0xFF7A0103));
      expect(light.comment, const Color(0xFFA50104));
      expect(dark.bg, const Color(0xFF320002));
      expect(dark.fg, const Color(0xFFFFFFFF));
      expect(dark.comment, const Color(0xFFD4D4D4));
      expect(light.isDark, isFalse);
      expect(dark.isDark, isTrue);
      expect(themePairFor(zorroThemeName), zorroDarkThemeName);
      expect(themePairFor(zorroDarkThemeName), zorroThemeName);
    });

    test('appear as one System-mode option labelled Zorro', () {
      final paired = themeGroups().paired.map((theme) => theme.name);
      expect(paired, contains(zorroThemeName));
      expect(paired, isNot(contains(zorroDarkThemeName)));
      expect(pairedThemeLabel(zorroThemeName), 'Zorro');
      expect(themeSelectionLabel(zorroThemeName, ThemeMode.system), 'Zorro');
      expect(
        themeSelectionLabel(zorroDarkThemeName, ThemeMode.system),
        'Zorro',
      );
      expect(
        themeSelectionLabel(zorroThemeName, ThemeMode.light),
        'Zorro Light',
      );
      expect(
        themeSelectionLabel(zorroDarkThemeName, ThemeMode.dark),
        'Zorro Dark',
      );
    });

    test('resolve across system, light, and dark appearances', () {
      final resolved = resolveSchemes(zorroThemeName, ThemeMode.system);
      expect(resolved.forcedMode, isNull);
      expect(resolved.lightTheme?.name, zorroThemeName);
      expect(resolved.darkTheme?.name, zorroDarkThemeName);
      expect(
        effectiveTheme(zorroThemeName, ThemeMode.dark)?.name,
        zorroDarkThemeName,
      );
      expect(
        effectiveTheme(zorroDarkThemeName, ThemeMode.light)?.name,
        zorroThemeName,
      );
    });

    test('forces neutral rendering without changing the stored accent', () {
      const storedAccent = '#ef4444';

      expect(
        effectiveAccentIndex(zorroThemeName, storedAccent),
        neutralAccentIndex,
      );
      expect(
        effectiveAccentIndex(zorroDarkThemeName, storedAccent),
        neutralAccentIndex,
      );
      expect(
        effectiveAccentIndex('github-light', storedAccent),
        accentIndexForWireValue(storedAccent),
      );
      expect(storedAccent, '#ef4444');
    });

    test('fallbacks expose the effective Zorro theme for gradients', () {
      for (final name in ['nord', 'not-a-theme']) {
        final resolved = resolveSchemes(name, ThemeMode.light);
        expect(resolved.lightTheme?.name, zorroThemeName);
        expect(
          zorroTopSectionGradient(
            resolved.lightTheme!.name,
            resolved.light.brightness,
          ),
          isNotNull,
        );
      }
    });
  });

  group('zorroTopSectionGradient', () {
    test('is limited to the Zorro pair', () {
      expect(zorroTopSectionGradient('github-light', Brightness.light), isNull);
      expect(isZorroTheme(zorroThemeName), isTrue);
      expect(isZorroTheme(zorroDarkThemeName), isTrue);
      expect(isZorroTheme('github-light'), isFalse);
    });

    test('uses the product palette for each brightness', () {
      final light = zorroTopSectionGradient(zorroThemeName, Brightness.light)!;
      final dark = zorroTopSectionGradient(zorroThemeName, Brightness.dark)!;

      expect(light.colors, const [Color(0xFFFFFAF2), Color(0xFFFFDFC2)]);
      expect(dark.colors, const [Color(0xFF260001), Color(0xFF260001)]);
    });
  });

  testWidgets('AppTheme carries the Zorro gradient to the top section', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(
          topSectionGradient: zorroTopSectionGradient(
            zorroThemeName,
            Brightness.light,
          ),
        ),
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
      ),
    );

    final container = tester
        .widgetList<Container>(
          find.descendant(
            of: find.byType(FrostedAppBar),
            matching: find.byType(Container),
          ),
        )
        .first;
    final decoration = container.decoration! as BoxDecoration;
    expect(decoration.gradient, isNotNull);
    expect(decoration.color, isNull);
  });

  testWidgets('non-Zorro themes keep the frosted surface fill', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const Stack(children: [FrostedAppBar(title: Text('Home'))]),
      ),
    );

    final container = tester
        .widgetList<Container>(
          find.descendant(
            of: find.byType(FrostedAppBar),
            matching: find.byType(Container),
          ),
        )
        .first;
    final decoration = container.decoration! as BoxDecoration;
    expect(decoration.gradient, isNull);
    expect(decoration.color, isNotNull);
  });

  testWidgets('Zorro navigation roles use neutral foregrounds', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(
          topSectionGradient: zorroTopSectionGradient(
            zorroThemeName,
            Brightness.light,
          ),
        ),
        home: const Scaffold(body: Text('Home')),
      ),
    );

    final context = tester.element(find.text('Home'));
    expect(navigationPrimaryForeground(context), Colors.black);
    expect(
      navigationSecondaryForeground(context),
      Colors.black.withValues(alpha: 0.4),
    );
    expect(
      navigationSectionForeground(context),
      Colors.black.withValues(alpha: 0.8),
    );
    expect(
      navigationSearchSurface(context),
      Colors.black.withValues(alpha: 0.04),
    );
  });

  testWidgets('navigation roles inherit non-Zorro theme tokens', (
    tester,
  ) async {
    const primaryForeground = Color(0xFF123456);
    const secondaryForeground = Color(0xFF789ABC);
    const searchSurface = Color(0xFFDEF012);
    final theme = ThemeData(
      colorScheme: ColorScheme.fromSeed(seedColor: Colors.purple).copyWith(
        onSurface: primaryForeground,
        onSurfaceVariant: secondaryForeground,
        surfaceContainerHighest: searchSurface,
      ),
    );

    await tester.pumpWidget(
      MaterialApp(
        theme: theme,
        home: const Scaffold(body: SizedBox()),
      ),
    );

    final context = tester.element(find.byType(SizedBox));
    expect(navigationPrimaryForeground(context), primaryForeground);
    expect(navigationSecondaryForeground(context), secondaryForeground);
    expect(navigationSectionForeground(context), secondaryForeground);
    expect(navigationSearchSurface(context), searchSurface);
    expect(
      navigationDivider(context, 0.15),
      primaryForeground.withValues(alpha: 0.15),
    );
  });
}
