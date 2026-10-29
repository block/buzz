import 'package:flutter/services.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/frosted_scroll_under_scope.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('Duo backdrop spans the rail while title stays in content', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    try {
      for (final left in [false, true]) {
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              theme: AppTheme.light(),
              home: DuoToolbarScope(
                active: true,
                strip: FoldStrip(
                  side: left ? FoldStripSide.left : FoldStripSide.right,
                  rect: Rect.fromLTWH(left ? 0 : 700, 0, 100, 600),
                  topClearance: 170,
                  bottomClearance: 0,
                  isClearanceEstimated: true,
                ),
                contentInsets: EdgeInsets.only(
                  left: left ? 100 : 0,
                  right: left ? 0 : 100,
                ),
                child: const FrostedScaffold(
                  appBar: FrostedAppBar(
                    nativeTitle: 'Solo',
                    nativeLargeTitle: true,
                  ),
                  body: SizedBox.expand(),
                ),
              ),
            ),
          ),
        );
        await tester.pump();
        final backdrop = tester.getRect(
          find.byKey(const ValueKey('duo-navigation-backdrop')),
        );
        final header = tester.getRect(find.byType(IosNavigationBar));
        expect(backdrop.left, 0);
        expect(backdrop.right, 800);
        expect(header.left, left ? 100 : 0);
        expect(header.right, left ? 800 : 700);
      }
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('Duo keeps list actions in header and profile on aligned rail', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final active = ValueNotifier(true);
    final hasStrip = ValueNotifier(true);
    final listHeader = ValueNotifier(false);
    addTearDown(listHeader.dispose);
    addTearDown(active.dispose);
    addTearDown(hasStrip.dispose);
    var presses = 0;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          builder: (context, child) => IosNavigationMetricsHost(
            child: MediaQuery(
              // The detail pane can be narrower than its window.
              data: MediaQuery.of(context).copyWith(size: const Size(400, 600)),
              child: child!,
            ),
          ),
          theme: AppTheme.light(),
          home: ListenableBuilder(
            listenable: Listenable.merge([active, hasStrip, listHeader]),
            builder: (context, _) => DuoToolbarScope(
              active: active.value,
              strip: hasStrip.value
                  ? const FoldStrip(
                      side: FoldStripSide.right,
                      rect: Rect.fromLTWH(700, 0, 100, 600),
                      topClearance: 170,
                      bottomClearance: 0,
                      isClearanceEstimated: true,
                    )
                  : null,
              child: Stack(
                children: [
                  FrostedAppBar(
                    nativeLargeTitle: listHeader.value,
                    nativeLeading: IosNavigationAction(
                      label: 'Community',
                      keepInHeaderOnDuo: !listHeader.value,
                      onPressed: () => presses++,
                    ),
                    nativeActions: [
                      IosNavigationAction(
                        label: 'Search',
                        keepInHeaderOnDuo: true,
                        onPressed: () => presses++,
                      ),
                      IosNavigationAction(
                        label: 'Inbox',
                        keepInHeaderOnDuo: true,
                        onPressed: () => presses++,
                      ),
                      IosNavigationAction(
                        label: 'Profile',
                        onPressed: () => presses++,
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    for (final (index, label) in ['Profile'].indexed) {
      final control = find.byKey(ValueKey('duo-toolbar-$label'));
      expect(tester.getTopLeft(control), Offset(694, 110 + index * 60));
      expect(tester.getSize(control).height, 60);
      final view = tester.widget<UiKitView>(
        find.descendant(of: control, matching: find.byType(UiKitView)),
      );
      expect(view.creationParams, containsPair('standaloneControl', true));
      await _nativeAction(tester, control, 910 + index, 'leading');
    }
    expect(presses, 1);
    final header = tester
        .widgetList<IosNavigationBar>(find.byType(IosNavigationBar))
        .singleWhere((bar) => !bar.standaloneControl);
    expect(header.horizontalControls, isTrue);

    expect(header.alwaysFrosted, isTrue);
    expect(
      find.byKey(const ValueKey('duo-navigation-backdrop')),
      findsOneWidget,
    );
    expect(tester.getSize(find.byWidget(header)).height, 68);
    expect(frostedAppBarHeight(tester.element(find.byWidget(header))), 68);
    expect(header.centerTitleInPane, isTrue);
    expect(header.leading!.label, 'Community');
    expect(header.actions.map((action) => action.label), ['Search', 'Inbox']);
    listHeader.value = true;
    await tester.pump();
    final listNavigation = tester.widget<IosNavigationBar>(
      find.byType(IosNavigationBar).first,
    );
    expect(listNavigation.horizontalControls, isTrue);
    expect(listNavigation.largeTitle, isTrue);
    expect(listNavigation.leading, isNull);
    listHeader.value = false;
    await tester.pump();

    final toolbarContext = tester.element(find.byType(FrostedAppBar));
    final view = View.of(toolbarContext);
    final windowSize = view.physicalSize / view.devicePixelRatio;
    IosNavigationMetrics.updateRail(toolbarContext, {
      'centerX': 740,
      'windowWidth': windowSize.width,
      'windowHeight': windowSize.height,
    });
    await tester.pump();
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('duo-toolbar-Profile'))).dx,
      684,
    );
    // A measurement from another pose must not move the current controls.
    IosNavigationMetrics.updateRail(toolbarContext, {
      'centerX': 500,
      'windowWidth': windowSize.height,
      'windowHeight': windowSize.width,
    });
    await tester.pump();
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('duo-toolbar-Profile'))).dx,
      694,
    );
    active.value = false;
    await tester.pump();
    expect(find.byKey(const ValueKey('duo-toolbar-Profile')), findsNothing);
    active.value = true;
    hasStrip.value = false;
    await tester.pump();
    expect(find.byType(IosNavigationBar), findsOneWidget);
    final restored = tester.widget<IosNavigationBar>(
      find.byType(IosNavigationBar),
    );
    expect(restored.centerTitleInPane, isFalse);
    expect(find.byKey(const ValueKey('duo-navigation-backdrop')), findsNothing);
    expect(restored.leading!.label, 'Community');
    expect(restored.actions.map((action) => action.label), [
      'Search',
      'Inbox',
      'Profile',
    ]);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('Duo Back follows pushed routes and action menus remain usable', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final navigator = GlobalKey<NavigatorState>();
    var chosen = false;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          navigatorKey: navigator,
          theme: AppTheme.light(),
          builder: (context, child) => DuoToolbarScope(
            strip: const FoldStrip(
              side: FoldStripSide.right,
              rect: Rect.fromLTWH(700, 0, 100, 600),
              topClearance: 170,
              bottomClearance: 0,
              isClearanceEstimated: true,
            ),
            active: true,
            child: child!,
          ),
          home: const Stack(children: [FrostedAppBar(nativeTitle: 'Home')]),
        ),
      ),
    );
    navigator.currentState!.push(
      MaterialPageRoute<void>(
        builder: (_) => Stack(
          children: [
            FrostedAppBar(
              nativeTitle: 'Conversation',
              nativeActions: [
                IosNavigationAction(
                  label: 'Options',
                  children: [
                    IosNavigationAction(
                      label: 'Choose assistant',
                      onPressed: () => chosen = true,
                    ),
                  ],
                ),
              ],
            ),
          ],
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('duo-toolbar-Back'))).dy,
      110,
    );
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('duo-toolbar-Options'))).dy,
      170,
    );
    await _nativeAction(
      tester,
      find.byKey(const ValueKey('duo-toolbar-Options')),
      920,
      'leading.0',
    );
    expect(chosen, isTrue);
    await _nativeAction(
      tester,
      find.byKey(const ValueKey('duo-toolbar-Back')),
      921,
      'leading',
    );
    await tester.pumpAndSettle();
    expect(navigator.currentState!.canPop(), isFalse);
    expect(find.byKey(const ValueKey('duo-toolbar-Back')), findsNothing);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('iOS conversations request a backdrop before scrolling', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const Stack(
            children: [
              FrostedAppBar(
                title: Text('Alice'),
                nativeSubtitle: 'Online',
                alwaysFrosted: true,
              ),
            ],
          ),
        ),
      ),
    );
    final view = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(view.creationParams, containsPair('alwaysFrosted', true));
    expect(view.creationParams, containsPair('subtitle', 'Online'));
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'conversation backdrop stays mounted across scroll and keyboard changes',
    (tester) async {
      final scrolled = ValueNotifier(false);
      addTearDown(scrolled.dispose);
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: ValueListenableBuilder<bool>(
            valueListenable: scrolled,
            builder: (_, value, _) => FrostedScrollUnderScope(
              isScrolledUnder: value,
              child: const Stack(
                children: [
                  FrostedAppBar(
                    title: Text('Conversation'),
                    alwaysFrosted: true,
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      final backdrop = tester.element(find.byType(BackdropFilter));
      final initial = tester
          .widget<Container>(
            find.byKey(const ValueKey('frosted-app-bar-background')),
          )
          .decoration;
      addTearDown(tester.view.reset);
      for (final under in [true, false, true, false]) {
        scrolled.value = under;
        tester.view.viewInsets = FakeViewPadding(bottom: under ? 300 : 0);
        await tester.pump();
        expect(tester.element(find.byType(BackdropFilter)), same(backdrop));
        expect(
          tester
              .widget<Container>(
                find.byKey(const ValueKey('frosted-app-bar-background')),
              )
              .decoration,
          initial,
        );
      }
    },
  );

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

  testWidgets('uses the UIKit navigation bar back action on iOS', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);

    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
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
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();

    final nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(nativeView.viewType, IosNavigationBar.viewType);
    final navigation = tester.widget<IosNavigationBar>(
      find.byType(IosNavigationBar),
    );
    expect(navigation.title, 'Destination');
    expect(navigation.onBack, isNotNull);
    navigation.onBack!();
    await tester.pumpAndSettle();
    expect(find.text('Open'), findsOneWidget);
    expect(find.byType(IosNavigationBar), findsNothing);
    expect(tester.takeException(), isNull);
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

Future<void> _nativeAction(
  WidgetTester tester,
  Finder control,
  int id,
  String action,
) async {
  final channel = MethodChannel('buzz/ios_navigation_bar/$id');
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    channel,
    (_) async => null,
  );
  tester
      .widget<UiKitView>(
        find.descendant(of: control, matching: find.byType(UiKitView)),
      )
      .onPlatformViewCreated!(id);
  await tester.pump();
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    channel.name,
    const StandardMethodCodec().encodeMethodCall(MethodCall('action', action)),
    (_) {},
  );
  await tester.pump();
}
