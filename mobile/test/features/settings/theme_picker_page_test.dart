import 'package:buzz/features/settings/theme_picker_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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

Future<void> _swipeToNextTheme(WidgetTester tester) async {
  await tester.fling(
    find.byKey(const ValueKey('theme-preview-pages')),
    const Offset(-600, 0),
    1200,
  );
  await tester.pumpAndSettle();
}

void main() {
  group('ThemePickerPage', () {
    testWidgets(
      'is the direct preview page with the name inside its container',
      (tester) async {
        tester.view.physicalSize = const Size(390, 844);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);

        await _pumpPicker(tester);

        expect(
          find.byKey(const ValueKey('theme-preview-pages')),
          findsOneWidget,
        );
        expect(find.byKey(const ValueKey('theme-preview-set')), findsOneWidget);
        expect(
          find.byKey(const ValueKey('theme-preview-close')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('theme-selection-grid')),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('theme-preview-name-buzz')),
          findsOneWidget,
        );

        final nameBounds = tester.getRect(
          find.byKey(const ValueKey('theme-preview-name-buzz')),
        );
        final previewBounds = tester.getRect(
          find.byKey(const ValueKey('theme-device-pair-preview-buzz')),
        );
        expect(nameBounds.bottom, lessThan(previewBounds.bottom));
        expect(previewBounds.top - nameBounds.bottom, closeTo(Grid.md, 1));
        expect(
          tester
              .widget<FittedBox>(
                find.byKey(const ValueKey('theme-full-preview-buzz')),
              )
              .alignment,
          Alignment.center,
        );
        expect(
          tester
              .getSize(find.byKey(const ValueKey('theme-preview-close')))
              .height,
          44,
        );
        expect(
          tester
              .getSize(find.byKey(const ValueKey('theme-preview-set')))
              .height,
          44,
        );
      },
    );

    testWidgets('keeps the theme name inside the card in landscape', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(844, 390);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await _pumpPicker(tester);

      final nameBounds = tester.getRect(
        find.byKey(const ValueKey('theme-preview-name-buzz')),
      );
      final cardBounds = tester.getRect(
        find.byKey(const ValueKey('theme-preview-page-buzz')),
      );
      expect(cardBounds.contains(nameBounds.topLeft), isTrue);
      expect(cardBounds.contains(nameBounds.bottomRight), isTrue);
    });

    testWidgets(
      'keeps accurate Home and Chat frames in one preview container',
      (tester) async {
        await _pumpPicker(tester);

        expect(
          find.byKey(const ValueKey('theme-preview-surface')),
          findsWidgets,
        );
        expect(
          find.byKey(const ValueKey('theme-chat-timeline-message-1')),
          findsWidgets,
        );
        expect(
          find.byKey(const ValueKey('theme-preview-status-time')),
          findsNothing,
        );

        final chatFrame = tester.widget<Container>(
          find
              .descendant(
                of: find.byKey(const ValueKey('theme-full-chat-buzz')),
                matching: find.byType(Container),
              )
              .first,
        );
        final decoration = chatFrame.decoration as BoxDecoration;
        final border = decoration.border! as Border;
        expect(chatFrame.clipBehavior, Clip.none);
        expect(border.top.strokeAlign, BorderSide.strokeAlignOutside);
      },
    );

    testWidgets('exposes only image labels for decorative previews', (
      tester,
    ) async {
      final semantics = tester.ensureSemantics();
      try {
        await _pumpPicker(tester);

        expect(find.bySemanticsLabel(RegExp('Theme preview')), findsNothing);
        expect(find.bySemanticsLabel(RegExp('Home preview')), findsWidgets);
        expect(find.bySemanticsLabel(RegExp('Chat preview')), findsWidgets);
        expect(find.bySemanticsLabel(RegExp('Community')), findsNothing);
      } finally {
        semantics.dispose();
      }
    });

    testWidgets('theme name trails the preview during a swipe', (tester) async {
      await _pumpPicker(tester);

      final pages = find.byKey(const ValueKey('theme-preview-pages'));
      final gesture = await tester.startGesture(tester.getCenter(pages));
      await gesture.moveBy(const Offset(-120, 0));
      await tester.pump();

      final titleMotion = tester.widget<Transform>(
        find.byKey(const ValueKey('theme-preview-name-motion-buzz')),
      );
      expect(titleMotion.transform.getTranslation().x, lessThan(0));

      await gesture.up();
      await tester.pumpAndSettle();
    });

    testWidgets('theme name does not trail with Reduce Motion', (tester) async {
      await _pumpPicker(tester, reduceMotion: true);

      final pages = find.byKey(const ValueKey('theme-preview-pages'));
      final gesture = await tester.startGesture(tester.getCenter(pages));
      await gesture.moveBy(const Offset(-120, 0));
      await tester.pump();

      final titleMotion = tester.widget<Transform>(
        find.byKey(const ValueKey('theme-preview-name-motion-buzz')),
      );
      expect(titleMotion.transform.getTranslation().x, 0);

      await gesture.up();
      await tester.pumpAndSettle();
    });

    testWidgets('matches the updated Figma Home skeleton', (tester) async {
      await _pumpPicker(tester);

      const rowTops = [
        116.0,
        152.0,
        188.0,
        224.0,
        266.0,
        302.0,
        338.0,
        374.0,
        416.0,
        452.0,
        488.0,
        524.0,
      ];
      const widths = [69.0, 69.0, 107.0, 91.0];
      for (var index = 0; index < rowTops.length; index++) {
        final isSection = index % 4 == 0;
        final row = find
            .byKey(
              isSection
                  ? ValueKey('theme-preview-section-${index ~/ 4}')
                  : ValueKey('theme-preview-channel-$index'),
            )
            .first;
        final capsule = find
            .byKey(
              isSection
                  ? ValueKey('theme-preview-section-${index ~/ 4}-capsule')
                  : ValueKey('theme-preview-channel-$index-capsule'),
            )
            .first;
        expect(row, findsOneWidget);
        final position = tester.widget<Positioned>(
          find.ancestor(of: row, matching: find.byType(Positioned)).first,
        );
        expect(position.top, rowTops[index]);
        expect(position.left, isSection ? 22 : 38);
        expect(tester.getSize(capsule).height, 11);
        expect(tester.getSize(capsule).width, widths[index % widths.length]);

        final capsuleWidget = tester.widget<Container>(capsule);
        final capsuleContext = tester.element(capsule);
        expect(
          (capsuleWidget.decoration! as BoxDecoration).color,
          isSection
              ? navigationSectionForeground(
                  capsuleContext,
                ).withValues(alpha: 0.52)
              : navigationPrimaryForeground(
                  capsuleContext,
                ).withValues(alpha: 0.38),
        );
      }
      expect(
        find.byKey(const ValueKey('theme-preview-section-3')),
        findsNothing,
      );

      final identity = find
          .byKey(const ValueKey('theme-preview-community-identity'))
          .first;
      final home = tester.getRect(
        find.byKey(const ValueKey('theme-full-home-buzz')),
      );
      final identityBounds = tester.getRect(identity);
      expect(
        (identityBounds.top - home.top) / home.height * 852,
        closeTo(38, 4),
      );
      expect(
        find.byKey(const ValueKey('theme-preview-community-icon')),
        findsWidgets,
      );
      expect(
        find.byKey(const ValueKey('theme-preview-community-name')),
        findsWidgets,
      );
    });

    testWidgets('keeps the chat header left aligned and accents only Send', (
      tester,
    ) async {
      await _pumpPicker(tester);

      expect(
        find.byKey(const ValueKey('theme-chat-header-avatar')),
        findsNothing,
      );
      final title = tester.getRect(
        find.byKey(const ValueKey('theme-chat-header-title')).first,
      );
      final chatFrame = tester.getRect(
        find.byKey(const ValueKey('theme-full-chat-buzz')),
      );
      expect(
        (title.left - chatFrame.left) / chatFrame.width * 393,
        closeTo(70, 4),
      );
      expect(title.center.dx, lessThan(chatFrame.center.dx));

      final send = tester.widget<DecoratedBox>(
        find
            .descendant(
              of: find.byKey(const ValueKey('theme-chat-send-action')).first,
              matching: find.byType(DecoratedBox),
            )
            .first,
      );
      final sendDecoration = send.decoration as BoxDecoration;
      final previewContext = tester.element(
        find.byKey(const ValueKey('theme-chat-send-action')).first,
      );
      expect(
        sendDecoration.color,
        Theme.of(previewContext).colorScheme.primary,
      );
    });

    testWidgets('uses native liquid-glass pagination on iOS', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);

      await _pumpPicker(tester);

      final nativePagination = tester.widget<UiKitView>(
        find.byWidgetPredicate(
          (widget) =>
              widget is UiKitView &&
              widget.viewType == 'buzz/theme_pagination_glass',
        ),
      );
      final params = nativePagination.creationParams! as Map<String, Object>;
      expect(params['count'], greaterThan(1));
      expect(params['selected'], 0);
      expect(params['animateChanges'], isTrue);
      expect(
        tester
            .getSize(
              find.byWidgetPredicate(
                (widget) =>
                    widget is UiKitView &&
                    widget.viewType == 'buzz/theme_pagination_glass',
              ),
            )
            .width,
        116,
      );

      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('uses matching native glass bottom actions on iOS', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);

      await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'light',
          'buzz_color_scheme': 'github-light',
        },
      );

      final nativeActions = tester
          .widgetList<UiKitView>(find.byType(UiKitView))
          .where((view) => view.viewType == 'buzz/navigation_glass');
      final accentAction = nativeActions.firstWhere(
        (view) =>
            (view.creationParams! as Map<String, Object>)['icon'] ==
            'colorSwatch',
      );
      final appearanceAction = nativeActions.firstWhere(
        (view) =>
            (view.creationParams! as Map<String, Object>)['icon'] == 'sun',
      );
      final accentParams = accentAction.creationParams! as Map<String, Object>;
      final appearanceParams =
          appearanceAction.creationParams! as Map<String, Object>;
      expect(accentParams['controlSize'], 54);
      expect(appearanceParams['controlSize'], 54);
      expect(accentParams['swatchColor'], isNotNull);

      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('swipes between drafts and saves only when Set is pressed', (
      tester,
    ) async {
      final instance = await _pumpPicker(
        tester,
        prefs: {'buzz_theme_mode': 'system', 'buzz_color_scheme': 'buzz'},
      );

      await _swipeToNextTheme(tester);
      expect(instance.getString('buzz_color_scheme'), 'buzz');
      expect(
        find.byKey(const ValueKey('theme-preview-name-buzz')),
        findsNothing,
      );

      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      await tester.pumpAndSettle();
      expect(instance.getString('buzz_color_scheme'), isNot('buzz'));
    });

    testWidgets('Set submits one complete theme preference', (tester) async {
      final notifier = _RecordingCommunityThemeNotifier();
      await _pumpPicker(tester, themeNotifier: () => notifier);

      await _swipeToNextTheme(tester);
      expect(notifier.preferenceWrites, 0);

      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      await tester.pumpAndSettle();

      expect(notifier.preferenceWrites, 1);
      expect(notifier.lastPreference?.theme, isNot('buzz'));
      expect(notifier.lastPreference?.followSystem, isTrue);
      expect(notifier.lastPreference?.accent, defaultCommunityTheme.accent);
    });

    testWidgets('preserves the stored accent when leaving Buzz', (
      tester,
    ) async {
      final green = accentColors.indexWhere((accent) => accent.name == 'Green');
      final legacyGreen = green - 1;
      final instance = await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'system',
          'buzz_color_scheme': 'buzz',
          'buzz_accent_color': legacyGreen,
        },
      );

      await _swipeToNextTheme(tester);
      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      await tester.pumpAndSettle();

      expect(instance.getString('buzz_color_scheme'), isNot('buzz'));
      expect(instance.getInt('buzz_accent_color'), green);
    });

    testWidgets('accent action scales and fades with theme availability', (
      tester,
    ) async {
      await _pumpPicker(
        tester,
        prefs: {'buzz_theme_mode': 'system', 'buzz_color_scheme': 'buzz'},
      );

      expect(
        find.byKey(const ValueKey('theme-preview-accent-unavailable')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
        findsNothing,
      );

      final pageView = tester.widget<PageView>(
        find.byKey(const ValueKey('theme-preview-pages')),
      );
      pageView.controller!.jumpToPage(1);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 80));

      expect(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
        findsOneWidget,
      );
      final scaleValues = tester
          .widgetList<ScaleTransition>(
            find.descendant(
              of: find.byKey(
                const ValueKey('theme-preview-accent-availability'),
              ),
              matching: find.byType(ScaleTransition),
            ),
          )
          .map((transition) => transition.scale.value);
      expect(scaleValues.any((value) => value > 0.78 && value < 0.95), isTrue);

      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('theme-preview-accent-unavailable')),
        findsNothing,
      );
    });

    testWidgets('scrubs directly across the pagination control', (
      tester,
    ) async {
      final instance = await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'dark',
          'buzz_color_scheme': 'catppuccin-macchiato',
        },
      );
      final pageView = tester.widget<PageView>(
        find.byKey(const ValueKey('theme-preview-pages')),
      );
      final scrubber = tester.getRect(
        find.byKey(const ValueKey('theme-preview-scrubber')),
      );

      await tester.tapAt(
        Offset(scrubber.left + scrubber.width * 0.85, scrubber.center.dy),
      );
      await tester.pumpAndSettle();

      expect(pageView.controller!.page, greaterThan(2));
      expect(instance.getString('buzz_color_scheme'), 'catppuccin-macchiato');
    });

    testWidgets('cycles appearance inline with scale and opacity motion', (
      tester,
    ) async {
      final instance = await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'light',
          'buzz_color_scheme': 'github-light',
        },
      );

      expect(
        find.byKey(const ValueKey('theme-appearance-light')),
        findsOneWidget,
      );
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pump();

      expect(find.byKey(const ValueKey('appearance-mode-dark')), findsNothing);
      expect(
        find.byKey(const ValueKey('theme-appearance-dark')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('theme-preview-appearance-switcher')),
          matching: find.byType(FadeTransition),
        ),
        findsWidgets,
      );

      await tester.pump(const Duration(milliseconds: 75));
      final opacityValues = tester
          .widgetList<FadeTransition>(
            find.descendant(
              of: find.byKey(
                const ValueKey('theme-preview-appearance-switcher'),
              ),
              matching: find.byType(FadeTransition),
            ),
          )
          .map((transition) => transition.opacity.value);
      expect(opacityValues.any((value) => value > 0 && value < 1), isTrue);

      await tester.pumpAndSettle();
      expect(instance.getString('buzz_theme_mode'), 'light');
      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      await tester.pumpAndSettle();
      expect(instance.getString('buzz_theme_mode'), 'dark');
    });

    testWidgets('appearance changes replace pagination state without motion', (
      tester,
    ) async {
      await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'light',
          'buzz_color_scheme': 'github-light',
        },
      );

      Duration paginationDuration() => tester
          .widgetList<AnimatedPositioned>(
            find.descendant(
              of: find.byKey(const ValueKey('theme-preview-scrubber')),
              matching: find.byType(AnimatedPositioned),
            ),
          )
          .first
          .duration;

      expect(paginationDuration(), const Duration(milliseconds: 150));
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pump();
      expect(paginationDuration(), Duration.zero);
    });

    testWidgets('appearance cycle becomes instant under Reduce Motion', (
      tester,
    ) async {
      await _pumpPicker(
        tester,
        prefs: {'buzz_theme_mode': 'light'},
        reduceMotion: true,
      );
      final switcher = tester.widget<AnimatedSwitcher>(
        find.byKey(const ValueKey('theme-preview-appearance-switcher')),
      );
      expect(switcher.duration, Duration.zero);

      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pump();
      expect(
        find.byKey(const ValueKey('theme-appearance-dark')),
        findsOneWidget,
      );
    });

    testWidgets('emits one selection haptic for appearance and scrub choices', (
      tester,
    ) async {
      await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'dark',
          'buzz_color_scheme': 'catppuccin-macchiato',
        },
      );
      var selectionHaptics = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (call) async {
            if (call.method == 'HapticFeedback.vibrate' &&
                call.arguments == 'HapticFeedbackType.selectionClick') {
              selectionHaptics++;
            }
            return null;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(SystemChannels.platform, null),
      );

      await tester.tap(
        find.byKey(const ValueKey('theme-preview-appearance-action-button')),
      );
      await tester.pumpAndSettle();
      expect(selectionHaptics, 1);

      final scrubber = tester.getRect(
        find.byKey(const ValueKey('theme-preview-scrubber')),
      );
      await tester.tapAt(
        Offset(scrubber.left + scrubber.width * 0.75, scrubber.center.dy),
      );
      await tester.pumpAndSettle();
      expect(selectionHaptics, 2);
    });

    testWidgets('accent remains a sheet and is committed with Set', (
      tester,
    ) async {
      final instance = await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'light',
          'buzz_color_scheme': 'github-light',
          'buzz_accent_color': 0,
        },
      );
      final storedBefore = instance.getInt('buzz_accent_color');
      await tester.tap(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
      );
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('accent-selection-grid')),
        findsOneWidget,
      );
      final green = accentColors.indexWhere((accent) => accent.name == 'Green');
      expect(find.text('Green'), findsNothing);
      expect(
        tester.getSemantics(find.byKey(ValueKey('accent-option-$green'))).label,
        contains('Green accent color'),
      );
      await tester.tap(find.byKey(ValueKey('accent-option-$green')));
      await tester.pumpAndSettle();
      expect(instance.getInt('buzz_accent_color'), storedBefore);

      Navigator.of(
        tester.element(find.byKey(ValueKey('accent-option-$green'))),
      ).pop();
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('theme-preview-set')));
      await tester.pumpAndSettle();
      expect(instance.getInt('buzz_accent_color'), green);
    });

    testWidgets('accent swatch is optically smaller than its tap target', (
      tester,
    ) async {
      await _pumpPicker(
        tester,
        prefs: {
          'buzz_theme_mode': 'light',
          'buzz_color_scheme': 'github-light',
        },
      );

      expect(
        tester.getSize(
          find.byKey(const ValueKey('theme-preview-accent-action-button')),
        ),
        const Size(64, 54),
      );
      expect(
        tester.getSize(
          find.byKey(const ValueKey('theme-preview-accent-swatch')),
        ),
        const Size.square(46),
      );
    });

    testWidgets('Buzz keeps the accent action unavailable', (tester) async {
      await _pumpPicker(tester, prefs: {'buzz_color_scheme': 'buzz'});
      expect(
        find.byKey(const ValueKey('theme-preview-accent-action-button')),
        findsNothing,
      );
    });
  });
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
