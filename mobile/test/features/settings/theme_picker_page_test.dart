import 'dart:convert';

import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';
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
          final wide = size.width - 76 >= 640 && size.width > size.height;
          final controls = tester.getRect(
            find.byKey(
              ValueKey(
                wide ? 'theme-preview-bottom-row' : 'theme-preview-rail-bottom',
              ),
            ),
          );
          final top = tester.getRect(
            find.byKey(const ValueKey('duo-toolbar-Set')),
          );
          expect(controls.top, greaterThan(top.bottom));
          expect(controls.bottom, lessThanOrEqualTo(size.height));
          if (wide) {
            expect(controls.bottom, size.height - 21);
            final colorButton = tester.getCenter(
              find.byKey(const ValueKey('theme-preview-accent-action-button')),
            );
            final appearanceButton = tester.getCenter(
              find.byKey(
                const ValueKey('theme-preview-appearance-action-button'),
              ),
            );
            expect(colorButton.dy, appearanceButton.dy);
            expect(controls.height, 56);
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
            final grid = find.byKey(const ValueKey('accent-selection-grid'));
            expect(tester.getSize(grid).width, lessThanOrEqualTo(320));
            Navigator.of(tester.element(grid)).pop();
            await tester.pumpAndSettle();
          } else {
            expect(controls.center.dx, closeTo(top.center.dx, 1));
          }
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
        expect(instance.getString(appearanceModeKey), ThemeMode.light.name);
        expect(notifier.lastPreference?.mode, defaultCommunityTheme.mode);
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
    final prefs = await _pumpPicker(tester, themeNotifier: () => notifier);
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
    // Mode stays on this device; the synced preference keeps its own theme
    // and followSystem and only takes the new chat color.
    expect(prefs.getString(appearanceModeKey), ThemeMode.dark.name);
    expect(notifier.lastPreference?.theme, defaultCommunityTheme.theme);
    expect(
      notifier.lastPreference?.followSystem,
      defaultCommunityTheme.followSystem,
    );
    expect(notifier.lastPreference?.accent, chatColorChoices[green].wireValue);
  });

  testWidgets('picker uses the device mode and keeps the synced one', (
    tester,
  ) async {
    // Desktop's legacy Cyan only maps to the Jade swatch; a mode-only Set
    // must not republish it as Jade's Green to other devices.
    const synced = CommunityThemePreference(
      theme: buzzDarkThemeName,
      accent: '#06b6d4',
      followSystem: false,
    );
    final prefs = await _pumpPicker(
      tester,
      prefs: {appearanceModeKey: ThemeMode.light.name},
      themeNotifier: () => _SeededCommunityThemeNotifier(synced),
    );
    expect(find.text('Light'), findsOneWidget);
    // Light -> Dark -> System.
    for (var i = 0; i < 2; i++) {
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pumpAndSettle();
    }
    await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
    await tester.pumpAndSettle();

    expect(prefs.getString(appearanceModeKey), ThemeMode.system.name);
    // The synced preference survives a mode change untouched, through the
    // real setPreference path.
    final container = ProviderScope.containerOf(
      tester.element(find.byType(MaterialApp)),
    );
    expect(container.read(communityThemeProvider), synced);
  });

  group('a legacy chat color', () {
    // Desktop's legacy Cyan and Indigo have no swatch of their own: Cyan
    // shows as Jade and Indigo as Cobalt. These run the real notifier over
    // a fake relay, so Set goes through the real save and publish path.
    const cyan = '#06b6d4';
    const indigo = '#6366f1';
    String swatch(String name) =>
        chatColorChoices.firstWhere((color) => color.name == name).wireValue;
    CommunityThemePreference preference(String accent) =>
        CommunityThemePreference(
          theme: buzzDarkThemeName,
          accent: accent,
          followSystem: false,
        );

    late FakeRelay relay;
    late ProviderContainer container;
    var revision = 1;

    NostrEvent synced(String accent) => relay.sign(
      kind: EventKind.readState,
      tags: const [
        ['d', communityThemeDTag],
        ['t', communityThemeDTag],
      ],
      content: jsonEncode(preference(accent).toJson()),
      createdAt: revision++,
      author: relay.keys,
    );

    Future<void> pumpPicker(WidgetTester tester, String accent) async {
      relay = FakeRelay();
      revision = 1;
      relay.seed([synced(accent)]);
      final prefs = await _prefs({});
      // Open the picker once the synced color has loaded, as a user would.
      final open = ValueNotifier(false);
      addTearDown(open.dispose);
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: ValueListenableBuilder(
            valueListenable: open,
            builder: (_, isOpen, _) =>
                isOpen ? const ThemePickerPage() : const SizedBox.shrink(),
          ),
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            ...relay.overrides,
            communityThemeCryptoProvider.overrideWithValue(
              CommunityThemeCrypto(encrypt: (s) => s, decrypt: (s) => s),
            ),
          ],
        ),
      );
      container = ProviderScope.containerOf(
        tester.element(find.byType(MaterialApp)),
      );
      // The app's theme keeps this provider listened in production.
      container.listen(communityThemeProvider, (_, _) {});
      await tester.pumpAndSettle();
      expect(container.read(communityThemeProvider).accent, accent);
      open.value = true;
      await tester.pumpAndSettle();
    }

    Future<void> receive(WidgetTester tester, String accent) async {
      relay.deliver(synced(accent));
      await tester.pumpAndSettle();
      expect(container.read(communityThemeProvider).accent, accent);
    }

    int shownSwatch(WidgetTester tester) {
      final bubble =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(const ValueKey('theme-chat-outgoing-bubble')),
                  )
                  .decoration
              as BoxDecoration;
      // System mode follows the test platform's light appearance, and one
      // mode tap would make it Light: the preview is light either way.
      return chatColorChoices.indexWhere(
        (choice) =>
            mobileAppearanceTheme(
              Brightness.light,
              choice.wireValue,
            ).extension<ChatColors>()!.outgoing ==
            bubble.color,
      );
    }

    int index(String name) =>
        chatColorChoices.indexWhere((color) => color.name == name);

    Future<void> tapSwatch(WidgetTester tester, String name) async {
      final option = find.byKey(ValueKey('accent-option-${index(name)}'));
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
      );
      await tester.pumpAndSettle();
      await tester.ensureVisible(option);
      await tester.pumpAndSettle();
      await tester.tap(option);
      await tester.pumpAndSettle();
      Navigator.of(tester.element(option)).pop();
      await tester.pumpAndSettle();
    }

    Future<void> cycleModeAndSet(WidgetTester tester) async {
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      // Past the sync manager's publish debounce.
      await tester.pumpAndSettle(const Duration(seconds: 3));
    }

    List<String> publishedAccents() => [
      for (final event in relay.published)
        if (event.getTagValue('d') == communityThemeDTag)
          (jsonDecode(event.content) as Map<String, dynamic>)['accent']
              as String,
    ];

    testWidgets('publishes the swatch it maps to when the user taps it', (
      tester,
    ) async {
      await pumpPicker(tester, cyan);
      await tapSwatch(tester, 'Jade');
      await cycleModeAndSet(tester);
      expect(container.read(communityThemeProvider).accent, swatch('Jade'));
      expect(publishedAccents(), [swatch('Jade')]);
    });

    testWidgets('keeps a tapped swatch when a sync lands under the picker', (
      tester,
    ) async {
      await pumpPicker(tester, cyan);
      await tapSwatch(tester, 'Cobalt');
      await receive(tester, indigo);
      await cycleModeAndSet(tester);
      expect(container.read(communityThemeProvider).accent, swatch('Cobalt'));
      expect(publishedAccents(), [swatch('Cobalt')]);
    });

    testWidgets('keeps a synced color on a mode-only Set', (tester) async {
      await pumpPicker(tester, cyan);
      await receive(tester, indigo);
      await cycleModeAndSet(tester);
      expect(container.read(communityThemeProvider).accent, indigo);
      expect(publishedAccents(), isEmpty);
    });

    testWidgets('the preview follows a sync until the user taps a swatch', (
      tester,
    ) async {
      await pumpPicker(tester, cyan);
      expect(shownSwatch(tester), index('Jade'));
      await receive(tester, indigo);
      expect(shownSwatch(tester), index('Cobalt'));
      await tapSwatch(tester, 'Gray');
      await receive(tester, cyan);
      expect(shownSwatch(tester), index('Gray'));
    });

    testWidgets('the open color grid follows a sync until the user taps', (
      tester,
    ) async {
      final semantics = tester.ensureSemantics();
      await pumpPicker(tester, cyan);
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
      );
      await tester.pumpAndSettle();
      Finder option(String name) =>
          find.byKey(ValueKey('accent-option-${index(name)}'));
      void expectMarked(String name) => expect(
        tester.getSemantics(option(name)),
        isSemantics(isSelected: true),
      );
      expectMarked('Jade');
      await receive(tester, indigo);
      expectMarked('Cobalt');
      await tester.ensureVisible(option('Gray'));
      await tester.pumpAndSettle();
      await tester.tap(option('Gray'));
      await tester.pumpAndSettle();
      await receive(tester, cyan);
      expectMarked('Gray');
      semantics.dispose();
    });
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
    final prefs = await _pumpPicker(tester, themeNotifier: () => notifier);
    await tester.tap(
      find.byKey(const ValueKey('theme-preview-appearance-action-button')),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('theme-preview-close')));
    await tester.pumpAndSettle();
    expect(notifier.preferenceWrites, 0);
    expect(prefs.getString(appearanceModeKey), isNull);
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

/// Real notifier, seeded with a synced preference.
class _SeededCommunityThemeNotifier extends CommunityThemeNotifier {
  _SeededCommunityThemeNotifier(this.initial);

  final CommunityThemePreference initial;

  @override
  CommunityThemePreference build() => initial;
}
