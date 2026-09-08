import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('title row and reported height grow with accessible text', (
    tester,
  ) async {
    const titleStyle = TextStyle(fontSize: 22, height: 1.3);
    late double reportedHeight;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(2)),
          child: Builder(
            builder: (context) {
              reportedHeight = frostedAppBarHeight(
                context,
                titleStyle: titleStyle,
              );
              return const Stack(
                children: [
                  FrostedAppBar(title: Text('Search'), titleStyle: titleStyle),
                ],
              );
            },
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final clip = find.descendant(
      of: find.byType(FrostedAppBar),
      matching: find.byType(ClipRect),
    );
    expect(reportedHeight, greaterThan(48));
    expect(tester.getSize(clip).height, closeTo(reportedHeight, 0.01));
    expect(tester.getSize(find.text('Search')).height, greaterThan(48));
    expect(tester.takeException(), isNull);
  });

  testWidgets('custom multi-line title height matches reported height', (
    tester,
  ) async {
    const titleContentHeight = 64.0;
    late double reportedHeight;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) {
            reportedHeight = frostedAppBarHeight(
              context,
              titleContentHeight: titleContentHeight,
            );
            return const Stack(
              children: [
                FrostedAppBar(
                  titleContentHeight: titleContentHeight,
                  title: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [Text('Title'), Text('Subtitle')],
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
    await tester.pumpAndSettle();

    final clip = find.descendant(
      of: find.byType(FrostedAppBar),
      matching: find.byType(ClipRect),
    );
    expect(tester.getSize(clip).height, closeTo(reportedHeight, 0.01));
    expect(reportedHeight, closeTo(titleContentHeight + Grid.xs + 1, 0.01));
    expect(tester.takeException(), isNull);
  });

  testWidgets('centers a title between asymmetric navigation controls', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const Stack(
          children: [
            FrostedAppBar(
              centerTitle: true,
              leading: SizedBox(width: 48, height: 48),
              title: Text('Profile', key: ValueKey('centered-title')),
              actions: [SizedBox(width: 96, height: 48)],
            ),
          ],
        ),
      ),
    );

    final titleRect = tester.getRect(
      find.byKey(const ValueKey('centered-title')),
    );
    expect(
      titleRect.center.dx,
      closeTo(
        tester.view.physicalSize.width / tester.view.devicePixelRatio / 2,
        0.01,
      ),
    );
  });

  testWidgets('uses the native glass back control on iOS', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => const Stack(
                  children: [
                    FrostedAppBar(
                      title: Text(
                        'Destination',
                        key: ValueKey('destination-title'),
                      ),
                    ),
                  ],
                ),
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();

    final nativeBack = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(nativeBack.viewType, 'buzz/navigation_glass');
    expect(nativeBack.creationParams, containsPair('icon', 'back'));
    expect(
      nativeBack.creationParams,
      containsPair('accessibilityLabel', 'Back'),
    );
    expect(
      nativeBack.creationParams,
      containsPair('buttonCenterX', iosGlassChannelHeaderButtonCenterX),
    );
    expect(
      nativeBack.creationParams,
      containsPair('hitTargetWidth', iosGlassChannelHeaderLeadingWidth),
    );
    expect(nativeBack.creationParams, containsPair('hitTargetHeight', 48.0));
    final backRect = tester.getRect(find.byType(IosGlassNavigationButton));
    final titleRect = tester.getRect(
      find.byKey(const ValueKey('destination-title')),
    );
    expect(titleRect.left - backRect.right, iosGlassChannelHeaderTitleSpacing);
    expect(find.byTooltip('Back'), findsOneWidget);
    expect(
      tester.widget<Tooltip>(find.byTooltip('Back')).excludeFromSemantics,
      isTrue,
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('uses the theme primary color for automatic navigation glyphs', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => const Stack(
                  children: [FrostedAppBar(title: Text('Destination'))],
                ),
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();

    final backButton = tester.widget<IconButton>(
      find.byWidgetPredicate(
        (widget) => widget is IconButton && widget.tooltip == 'Back',
      ),
    );
    expect(backButton.color, AppTheme.light().colorScheme.primary);
  });

  testWidgets('replaces native glass while a Flutter backdrop is active', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final suppressNativeView = ValueNotifier(false);
    addTearDown(suppressNativeView.dispose);
    var pressCount = 0;

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: IosGlassNavigationButton(
            icon: IosGlassNavigationIcon.back,
            semanticLabel: 'Back',
            onPressed: () => pressCount++,
            nativeViewSuppressed: suppressNativeView,
          ),
        ),
      ),
    );

    expect(find.byType(UiKitView), findsOneWidget);
    expect(
      find.byKey(const ValueKey('ios-glass-navigation-flutter-fallback')),
      findsNothing,
    );
    expect(find.bySemanticsLabel('Back'), findsNothing);

    suppressNativeView.value = true;
    await tester.pump();

    expect(find.byType(UiKitView), findsNothing);
    expect(
      find.byKey(const ValueKey('ios-glass-navigation-flutter-fallback')),
      findsOneWidget,
    );
    final fallbackFinder = find.bySemanticsLabel('Back');
    expect(fallbackFinder, findsOneWidget);
    final fallbackSemantics = tester.getSemantics(fallbackFinder);
    expect(fallbackSemantics.flagsCollection.isButton, isTrue);
    expect(
      fallbackSemantics.flagsCollection.isEnabled.toString(),
      'Tristate.isTrue',
    );
    expect(
      fallbackSemantics.getSemanticsData().hasAction(SemanticsAction.tap),
      isTrue,
    );
    tester.binding.performSemanticsAction(
      SemanticsActionEvent(
        type: SemanticsAction.tap,
        viewId: tester.view.viewId,
        nodeId: fallbackSemantics.id,
      ),
    );
    await tester.pump();
    expect(pressCount, 1);

    suppressNativeView.value = false;
    await tester.pump();

    expect(find.byType(UiKitView), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });
}
