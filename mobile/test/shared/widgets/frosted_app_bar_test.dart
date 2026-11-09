import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scroll_under_scope.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets(
    'opt-in headers align collapsed filters and restore phone geometry',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const titleKey = ValueKey('layout-title');
      const filtersKey = ValueKey('layout-filters');
      const open = NavigationHeaderLayout(
        hardEdge: true,
        paneControls: true,
        topSpacing: 24,
        centerInVisibleHeight: true,
        alignCollapsedBottom: true,
      );
      for (final safeTop in [0.0, 59.0]) {
        final cover = NavigationHeaderLayout(
          hardEdge: true,
          paneControls: true,
          topSpacing: 24,
          controlCenterY: safeTop + 45,
          centerInVisibleHeight: true,
          alignCollapsedBottom: true,
          nativeEdgeClearance: 16,
        );
        for (final layout in [
          const NavigationHeaderLayout(),
          open,
          cover,
          open,
          const NavigationHeaderLayout(),
        ]) {
          for (final collapsed in [false, true]) {
            late double clearance;
            await tester.pumpWidget(
              ProviderScope(
                child: MaterialApp(
                  theme: AppTheme.light(),
                  home: MediaQuery(
                    data: MediaQueryData(
                      padding: EdgeInsets.only(top: safeTop),
                      viewPadding: EdgeInsets.only(top: safeTop),
                    ),
                    child: NavigationHeaderScope(
                      layout: layout,
                      child: Builder(
                        builder: (context) {
                          clearance = frostedAppBarEdgeClearance(context);
                          return Stack(
                            children: [
                              FrostedAppBar(
                                useNativeNavigation: false,
                                title: const SizedBox(
                                  key: titleKey,
                                  height: 44,
                                  child: Text('Messages'),
                                ),
                                bottom: const SizedBox(
                                  key: filtersKey,
                                  height: 72,
                                  child: Text('All   Solo   Team'),
                                ),
                                bottomHeight: collapsed ? 0 : 72,
                                bottomOverlap: collapsed ? 72 : 0,
                              ),
                            ],
                          );
                        },
                      ),
                    ),
                  ),
                ),
              ),
            );
            await tester.pump();
            final title = tester.getRect(find.byKey(titleKey));
            final filters = tester.getRect(find.byKey(filtersKey));
            expect(filters.height, 72);
            expect(title.height, 44);
            expect(clearance, layout.hardEdge ? layout.nativeEdgeClearance : 0);
            if (layout.alignCollapsedBottom && collapsed) {
              expect(filters.center.dy, closeTo(title.center.dy, 0.01));
              expect(
                title.center.dy,
                closeTo(layout.controlCenterY ?? safeTop + 38, 0.01),
              );
            } else if (!layout.hardEdge) {
              expect(title.center.dy, safeTop + 28);
              expect(filters.center.dy, safeTop + (collapsed ? 20 : 92));
            }
            if (layout.alignCollapsedBottom && collapsed) {
              expect(title.top, greaterThanOrEqualTo(safeTop));
            }
            expect(tester.takeException(), isNull);
          }
        }
      }
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'explicit axes keep native controls inside shared header bounds',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final messenger = tester.binding.defaultBinaryMessenger;
      final channels = <MethodChannel>[];
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
        call,
      ) async {
        if (call.method == 'create') {
          final id = (call.arguments as Map)['id'];
          final channel = MethodChannel('${IosNavigationBar.viewType}/$id');
          channels.add(channel);
          messenger.setMockMethodCallHandler(channel, (_) async => null);
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
        for (final channel in channels) {
          messenger.setMockMethodCallHandler(channel, null);
        }
      });
      for (final axis in [119.0, 179.0]) {
        for (final native in [false, true]) {
          late double bodyTop;
          late double stickyTop;
          await tester.pumpWidget(
            ProviderScope(
              child: MaterialApp(
                theme: AppTheme.light(),
                home: MediaQuery(
                  data: const MediaQueryData(
                    padding: EdgeInsets.only(top: 59),
                    viewPadding: EdgeInsets.only(top: 59),
                  ),
                  child: NavigationHeaderScope(
                    layout: NavigationHeaderLayout(
                      hardEdge: true,
                      paneControls: true,
                      topSpacing: 24,
                      centerInVisibleHeight: true,
                      controlCenterY: axis,
                      nativeEdgeClearance: 16,
                    ),
                    child: Builder(
                      builder: (context) {
                        bodyTop = frostedAppBarHeight(
                          context,
                          useNativeNavigation: native,
                        );
                        stickyTop =
                            bodyTop + frostedAppBarEdgeClearance(context);
                        return Stack(
                          children: [
                            FrostedAppBar(
                              useNativeNavigation: native,
                              title: const SizedBox(
                                key: ValueKey('axis-title'),
                                height: 44,
                                child: Text('Messages'),
                              ),
                            ),
                          ],
                        );
                      },
                    ),
                  ),
                ),
              ),
            ),
          );
          await tester.pump();
          await tester.pump();
          expect(bodyTop, axis + 34);
          expect(stickyTop, axis + 50);
          final bounds = native
              ? tester.getRect(find.byType(UiKitView))
              : tester.getRect(find.byType(FrostedAppBar));
          expect(bounds.bottom, bodyTop);
          final lowerControlPoint = Offset(bounds.center.dx, axis + 21);
          expect(bounds.contains(lowerControlPoint), isTrue);
          if (native) {
            final view = tester.renderObject<RenderBox>(find.byType(UiKitView));
            final hit = BoxHitTestResult();
            expect(
              view.hitTest(
                hit,
                position: view.globalToLocal(lowerControlPoint),
              ),
              isTrue,
            );
          } else {
            final title = tester.getRect(
              find.byKey(const ValueKey('axis-title')),
            );
            expect(title.center.dy, axis);
            expect(title.bottom, lessThanOrEqualTo(bounds.bottom));
          }
          expect(tester.takeException(), isNull);
        }
      }
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

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
    expect(reportedHeight, closeTo(titleContentHeight + Grid.xs, 0.01));
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
