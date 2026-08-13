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
}
