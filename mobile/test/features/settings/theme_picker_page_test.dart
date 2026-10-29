import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

Future<SharedPreferences> _prefs(Map<String, Object> initial) async {
  SharedPreferences.setMockInitialValues(initial);
  return SharedPreferences.getInstance();
}

Future<SharedPreferences> _pumpPicker(
  WidgetTester tester, {
  Map<String, Object> prefs = const {},
  bool reduceMotion = false,
  CommunityThemeNotifier Function()? themeNotifier,
}) async {
  final instance = await _prefs(prefs);
  await tester.pumpWidget(
    WidgetHelpers.testable(
      child: MediaQuery(
        data: MediaQueryData(disableAnimations: reduceMotion),
        child: const ThemePickerPage(),
      ),
      overrides: [
        savedPrefsProvider.overrideWithValue(instance),
        if (themeNotifier != null)
          communityThemeProvider.overrideWith(themeNotifier),
      ],
    ),
  );
  await tester.pumpAndSettle();
  return instance;
}

void main() {
  testWidgets(
    'Appearance preserves its draft inside Duo content bounds across folds',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final instance = await _prefs({});
      final notifier = _RecordingCommunityThemeNotifier();
      try {
        tester.view.physicalSize = const Size(466, 678);
        tester.view.viewPadding = const FakeViewPadding(right: 76, bottom: 21);
        tester.view.padding = const FakeViewPadding(right: 76, bottom: 21);
        await tester.pumpWidget(
          WidgetHelpers.testable(
            child: const ThemePickerPage(),
            overrides: [
              savedPrefsProvider.overrideWithValue(instance),
              communityThemeProvider.overrideWith(() => notifier),
            ],
          ),
        );
        await tester.pumpAndSettle();
        // The OS button is the only mocked boundary.
        IosNavigationAction railAction(String label) => tester
            .widget<IosNavigationBar>(
              find.descendant(
                of: find.byKey(ValueKey('duo-toolbar-$label')),
                matching: find.byType(IosNavigationBar),
              ),
            )
            .leading!;
        IosGlassNavigationButton modeButton() =>
            tester.widget<IosGlassNavigationButton>(
              find.byKey(
                const ValueKey('theme-preview-appearance-action-button'),
              ),
            );
        modeButton().onPressed!();
        await tester.pumpAndSettle();
        tester
            .widget<IosGlassNavigationButton>(
              find.descendant(
                of: find.byKey(
                  const ValueKey('theme-preview-accent-action-button'),
                ),
                matching: find.byType(IosGlassNavigationButton),
              ),
            )
            .onPressed!();
        await tester.pumpAndSettle();
        final jade = chatColorChoices.indexWhere(
          (color) => color.name == 'Jade',
        );
        final option = find.byKey(ValueKey('accent-option-$jade'));
        await tester.ensureVisible(option);
        await tester.tap(option);
        await tester.pumpAndSettle();
        Navigator.of(tester.element(option)).pop();
        await tester.pumpAndSettle();
        for (final (size, left) in [
          (const Size(466, 678), false),
          (const Size(951, 669), false),
          (const Size(669, 951), false),
          (const Size(678, 466), true),
        ]) {
          tester.view.physicalSize = size;
          final padding = FakeViewPadding(
            left: left ? 76 : 0,
            right: left ? 0 : 76,
            bottom: 21,
          );
          tester.view.viewPadding = padding;
          tester.view.padding = padding;
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          expect(modeButton().icon, IosGlassNavigationIcon.sun);
          expect(notifier.preferenceWrites, 0);
          final preview = tester.getRect(
            find.byKey(const ValueKey('theme-preview-page-$buzzThemeName')),
          );
          expect(preview.left, greaterThanOrEqualTo(left ? 76 : 0));
          expect(
            preview.right,
            lessThanOrEqualTo(size.width - (left ? 0 : 76)),
          );
          final images = tester.getCenter(
            find.byKey(
              const ValueKey('theme-device-pair-preview-$buzzThemeName'),
            ),
          );
          expect(images.dy, closeTo(size.height / 2, 1));
          expect(images.dx, closeTo((size.width + (left ? 76 : -76)) / 2, 1));
          final controls = tester.getRect(
            find.byKey(const ValueKey('theme-preview-rail-bottom')),
          );
          final top = tester.getRect(
            find.byKey(const ValueKey('duo-toolbar-Set')),
          );
          expect(controls.top, greaterThan(top.bottom));
          expect(controls.bottom, lessThanOrEqualTo(size.height));
          expect(controls.center.dx, closeTo(top.center.dx, 1));
          final swatch = tester.widget<IosGlassNavigationButton>(
            find.descendant(
              of: find.byKey(
                const ValueKey('theme-preview-accent-action-button'),
              ),
              matching: find.byType(IosGlassNavigationButton),
            ),
          );
          expect(swatch.icon, IosGlassNavigationIcon.colorSwatch);
          expect(
            swatch.swatchColor,
            chatColorForScheme(lightColorScheme, jade),
          );
          expect(
            find.byKey(const ValueKey('theme-preview-name-$buzzThemeName')),
            findsNothing,
          );
        }
        railAction('Set').onPressed!();
        await tester.pumpAndSettle();
        expect(notifier.preferenceWrites, 1);
        expect(notifier.lastPreference?.mode, ThemeMode.light);
        expect(
          notifier.lastPreference?.accent,
          chatColorChoices[jade].wireValue,
        );
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('one Home and Chat pair replaces the theme carousel', (
    tester,
  ) async {
    await _pumpPicker(tester, prefs: {'buzz_color_scheme': 'dracula'});
    expect(find.byType(PageView), findsNothing);
    expect(find.bySemanticsLabel('Home preview'), findsOneWidget);
    expect(find.bySemanticsLabel('Chat preview'), findsOneWidget);
    expect(
      find.byKey(const ValueKey('theme-preview-accent-action-button')),
      findsOneWidget,
    );
    expect(find.text('Dracula'), findsNothing);
  });

  testWidgets('mode and chat color remain drafts until Set', (tester) async {
    final notifier = _RecordingCommunityThemeNotifier();
    await _pumpPicker(tester, themeNotifier: () => notifier);
    // System -> Light -> Dark.
    for (var i = 0; i < 2; i++) {
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pumpAndSettle();
    }
    expect(find.text('Dark'), findsOneWidget);
    await tester.tap(
      find.byKey(const ValueKey('theme-preview-accent-action-button')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Chat color'), findsOneWidget);
    final green = chatColorChoices.indexWhere((color) => color.name == 'Jade');
    await tester.ensureVisible(find.byKey(ValueKey('accent-option-$green')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(ValueKey('accent-option-$green')));
    await tester.pumpAndSettle();
    expect(notifier.preferenceWrites, 0);
    Navigator.of(
      tester.element(find.byKey(ValueKey('accent-option-$green'))),
    ).pop();
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
    await tester.pumpAndSettle();
    expect(notifier.preferenceWrites, 1);
    expect(notifier.lastPreference?.mode, ThemeMode.dark);
    expect(notifier.lastPreference?.theme, buzzDarkThemeName);
    expect(notifier.lastPreference?.accent, chatColorChoices[green].wireValue);
  });

  for (final brightness in Brightness.values) {
    testWidgets('${brightness.name} chat preview follows draft colors only', (
      tester,
    ) async {
      final notifier = _RecordingCommunityThemeNotifier();
      await _pumpPicker(tester, themeNotifier: () => notifier);
      // System starts with the test platform's light appearance.
      if (brightness == Brightness.dark) {
        for (var i = 0; i < 2; i++) {
          await tester.tap(
            find.byKey(
              const ValueKey('theme-preview-appearance-action-button'),
            ),
          );
          await tester.pumpAndSettle();
        }
      }
      Color bubbleColor() =>
          (tester
                      .widget<DecoratedBox>(
                        find.byKey(
                          const ValueKey('theme-chat-outgoing-bubble'),
                        ),
                      )
                      .decoration
                  as BoxDecoration)
              .color!;
      Color foregroundColor() =>
          (tester
                      .widget<DecoratedBox>(
                        find.byKey(
                          const ValueKey('theme-chat-outgoing-line-206.0'),
                        ),
                      )
                      .decoration
                  as BoxDecoration)
              .color!;
      Color surfaceColor() => tester
          .widget<ColoredBox>(
            find.byKey(const ValueKey('theme-chat-preview-surface')),
          )
          .color;
      final surface = surfaceColor();
      final fills = <Color>[];
      for (final name in ['Cobalt', 'Jade', 'Chartreuse', 'Gray']) {
        await tester.tap(
          find.byKey(const ValueKey('theme-preview-accent-action-button')),
        );
        await tester.pumpAndSettle();
        final index = chatColorChoices.indexWhere(
          (color) => color.name == name,
        );
        final option = find.byKey(ValueKey('accent-option-$index'));
        await tester.ensureVisible(option);
        await tester.pumpAndSettle();
        await tester.tap(option);
        await tester.pumpAndSettle();
        Navigator.of(tester.element(option)).pop();
        await tester.pumpAndSettle();
        final theme = mobileAppearanceTheme(
          brightness,
          chatColorChoices[index].wireValue,
        );
        final chat = theme.extension<ChatColors>()!;
        expect(bubbleColor(), chat.outgoing);
        expect(foregroundColor(), chat.foreground);
        expect(surfaceColor(), surface);
        expect(notifier.preferenceWrites, 0);
        fills.add(bubbleColor());
      }
      expect(fills.toSet(), hasLength(4));
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('Close discards mode changes', (tester) async {
    final notifier = _RecordingCommunityThemeNotifier();
    await _pumpPicker(tester, themeNotifier: () => notifier);
    await tester.tap(
      find.byKey(const ValueKey('theme-preview-appearance-action-button')),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('theme-preview-close')));
    await tester.pumpAndSettle();
    expect(notifier.preferenceWrites, 0);
  });

  testWidgets(
    'appearance cycles through Light Dark System with reduced motion',
    (tester) async {
      await _pumpPicker(tester, reduceMotion: true);
      for (final label in ['Light', 'Dark', 'System']) {
        await tester.tap(
          find.byKey(const ValueKey('theme-preview-appearance-action-button')),
        );
        await tester.pumpAndSettle();
        expect(find.text(label), findsOneWidget);
        expect(tester.takeException(), isNull);
      }
    },
  );
}

class _RecordingCommunityThemeNotifier extends CommunityThemeNotifier {
  int preferenceWrites = 0;
  CommunityThemePreference? lastPreference;

  @override
  CommunityThemePreference build() => defaultCommunityTheme;

  @override
  void setPreference(CommunityThemePreference preference) {
    preferenceWrites++;
    lastPreference = preference;
    state = preference;
  }
}
