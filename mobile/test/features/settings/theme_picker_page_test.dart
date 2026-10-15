import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
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
    final green = chatColorChoices.indexWhere((color) => color.name == 'Green');
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
      for (final name in ['Blue', 'Green', 'Neutral']) {
        await tester.tap(
          find.byKey(const ValueKey('theme-preview-accent-action-button')),
        );
        await tester.pumpAndSettle();
        final index = chatColorChoices.indexWhere(
          (color) => color.name == name,
        );
        final option = find.byKey(ValueKey('accent-option-$index'));
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
      expect(fills.toSet(), hasLength(3));
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
