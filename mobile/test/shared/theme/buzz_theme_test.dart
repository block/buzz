import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/navigation_backdrop.dart';

const _buzzGradient = LinearGradient(
  begin: Alignment.topCenter,
  end: Alignment.bottomCenter,
  colors: [Color(0xFFE6E6B6), Color(0xFFC4D0DA)],
);

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
    });

    test('resolve across brightnesses like any other pair', () {
      expect(
        effectiveTheme(buzzThemeName, ThemeMode.dark)?.name,
        buzzDarkThemeName,
      );
      expect(
        effectiveTheme(buzzDarkThemeName, ThemeMode.light)?.name,
        buzzThemeName,
      );
    });
  });

  group('theme threading', () {
    Widget harness(ThemeData theme) => MaterialApp(
      theme: theme,
      home: Builder(
        builder: (context) => FrostedScaffold(
          backgroundGradient: context.appColors.topSectionGradient,
          appBar: const FrostedAppBar(title: Text('Home'), alwaysFrosted: true),
          body: const SizedBox.expand(),
        ),
      ),
    );

    testWidgets('AppTheme carries the gradient to the top section', (
      tester,
    ) async {
      await tester.pumpWidget(
        harness(AppTheme.light(topSectionGradient: _buzzGradient)),
      );

      final background = tester.widget<DecoratedBox>(
        find.byKey(const ValueKey('frosted-scaffold-pinned-gradient')),
      );
      expect((background.decoration as BoxDecoration).gradient, isNotNull);
      expect(find.byType(NavigationBackdrop), findsOneWidget);
    });

    testWidgets('non-Buzz themes keep the frosted surface fill', (
      tester,
    ) async {
      await tester.pumpWidget(harness(AppTheme.light()));

      expect(
        find.byKey(const ValueKey('frosted-scaffold-pinned-gradient')),
        findsNothing,
      );
      expect(find.byType(NavigationBackdrop), findsOneWidget);
    });

    testWidgets('Buzz section labels use 80% neutral foreground', (
      tester,
    ) async {
      await tester.pumpWidget(
        harness(AppTheme.light(topSectionGradient: _buzzGradient)),
      );

      final context = tester.element(find.text('Home'));
      expect(
        navigationSectionForeground(context),
        Colors.black.withValues(alpha: 0.8),
      );
    });

    testWidgets('navigation roles inherit non-Buzz theme tokens', (
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
      expect(navigationSearchSurface(context), const Color(0xFFF0F0F0));
      expect(
        navigationDivider(context, 0.15),
        primaryForeground.withValues(alpha: 0.15),
      );
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
