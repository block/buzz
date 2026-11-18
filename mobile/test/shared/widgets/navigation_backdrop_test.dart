import 'dart:ui' as ui;

import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/navigation_backdrop.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('Android headers share Home blur and surface without a divider', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const Stack(
          children: [
            FrostedAppBar(title: Text('Settings'), alwaysFrosted: true),
          ],
        ),
      ),
    );
    final backdrop = find.byType(NavigationBackdrop);
    final blur = tester.widget<BackdropFilter>(
      find.descendant(of: backdrop, matching: find.byType(BackdropFilter)),
    );
    expect(blur.enabled, isTrue);
    expect(blur.filter, ui.ImageFilter.blur(sigmaX: 20, sigmaY: 20));
    final tint = tester.widget<ColoredBox>(
      find.descendant(of: backdrop, matching: find.byType(ColoredBox)),
    );
    expect(
      tint.color,
      AppTheme.light().colorScheme.surface.withValues(alpha: 0.45),
    );
    expect(find.byType(UiKitView), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'custom iOS header keeps controls over a decorative native edge',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      var taps = 0;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Stack(
              children: [
                FrostedAppBar(
                  useNativeNavigation: false,
                  automaticallyImplyLeading: false,
                  alwaysFrosted: true,
                  title: const Text('Messages'),
                  actions: [
                    TextButton(
                      onPressed: () => taps++,
                      child: const Text('Search'),
                    ),
                  ],
                  bottomHeight: 40,
                  bottom: const Text('Filters'),
                ),
              ],
            ),
          ),
        ),
      );
      final native = find.byType(UiKitView);
      final view = tester.widget<UiKitView>(native);
      expect(view.creationParams, containsPair('backdropOnly', true));
      expect(view.creationParams, containsPair('alwaysFrosted', true));
      expect(
        find.ancestor(of: native, matching: find.byType(ExcludeSemantics)),
        findsWidgets,
      );
      expect(
        find.ancestor(of: native, matching: find.byType(IgnorePointer)),
        findsWidgets,
      );
      expect(find.text('Messages'), findsOneWidget);
      expect(find.text('Filters'), findsOneWidget);
      await tester.tap(find.text('Search'));
      expect(taps, 1);
      expect(find.byType(BackdropFilter), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );
}
