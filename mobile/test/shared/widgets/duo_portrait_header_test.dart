import 'package:buzz/shared/widgets/duo_toolbar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/ios_navigation_metrics.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('Home row shares the status region without narrowing filters', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.physicalSize = const Size(669, 951);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    late BuildContext metricsContext;
    var viewportHeight = 951.0;
    late StateSetter resizeViewport;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: IosNavigationMetricsHost(
            child: Builder(
              builder: (context) {
                metricsContext = context;
                return StatefulBuilder(
                  builder: (context, setState) {
                    resizeViewport = setState;
                    return MediaQuery(
                      data: MediaQueryData(
                        size: Size(669, viewportHeight),
                        padding: const EdgeInsets.only(top: 82),
                      ),
                      child: FrostedScaffold(
                        appBar: FrostedAppBar(
                          useNativeNavigation: false,
                          alignWithStatusRegion: true,
                          centerTitle: false,
                          title: const Text('Messages'),
                          actions: [
                            SizedBox(
                              key: const Key('action'),
                              width: 44,
                              height: 44,
                            ),
                          ],
                          bottomHeight: 48,
                          bottom: const SizedBox(
                            key: Key('filters'),
                            height: 48,
                          ),
                        ),
                        body: const SizedBox.expand(),
                      ),
                    );
                  },
                );
              },
            ),
          ),
        ),
      ),
    );
    IosNavigationMetrics.updatePortraitHeader(metricsContext, [
      669,
      951,
      0,
      20,
      535,
      56,
    ]);
    await tester.pump();
    expect(tester.getCenter(find.byKey(const Key('action'))).dy, 48);
    expect(
      tester.getRect(find.byKey(const Key('action'))).right,
      lessThanOrEqualTo(535),
    );
    expect(tester.getSize(find.byKey(const Key('filters'))).width, 669);
    // A keyboard or pane can resize MediaQuery without resizing the window.
    resizeViewport(() => viewportHeight = 600);
    await tester.pump();
    expect(tester.getCenter(find.byKey(const Key('action'))).dy, 48);
    final metrics = IosNavigationMetrics.of(metricsContext);
    expect(metrics.portraitHeaderFor(const Size(951, 669)), isNull);
    IosNavigationMetrics.updatePortraitHeader(metricsContext, [
      669,
      951,
      0,
      0,
      0,
      0,
    ]);
    await tester.pump();
    expect(tester.getCenter(find.byKey(const Key('action'))).dy, 110);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'portrait utility title stays on the screen midpoint while scrolling',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      tester.view.physicalSize = const Size(669, 951);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      late BuildContext metrics;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: IosNavigationMetricsHost(
              child: Builder(
                builder: (context) {
                  metrics = context;
                  return FrostedScaffold(
                    appBar: const FrostedAppBar(
                      title: Text('Settings'),
                      centerTitle: true,
                      useNativeNavigation: false,
                      alignWithStatusRegion: true,
                      actions: [SizedBox(width: 44)],
                    ),
                    body: ListView(children: const [SizedBox(height: 2000)]),
                  );
                },
              ),
            ),
          ),
        ),
      );
      IosNavigationMetrics.updatePortraitHeader(metrics, [
        669,
        951,
        0,
        20,
        535,
        56,
      ]);
      await tester.pumpAndSettle();
      await tester.drag(find.byType(ListView), const Offset(0, -250));
      await tester.pumpAndSettle();
      expect(tester.getCenter(find.text('Settings')), const Offset(334.5, 48));
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('two-panel Messages matches the portrait title spacing', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    Future<(double, double)> measure({required bool landscape}) async {
      await tester.pumpWidget(const SizedBox());
      final size = landscape ? const Size(951, 669) : const Size(669, 951);
      tester.view.physicalSize = size;
      late BuildContext metrics;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: IosNavigationMetricsHost(
              child: Builder(
                builder: (context) {
                  metrics = context;
                  return MediaQuery(
                    data: MediaQuery.of(context).copyWith(
                      padding: EdgeInsets.only(top: landscape ? 0 : 82),
                    ),
                    child: DuoToolbarScope(
                      strip: landscape
                          ? const FoldStrip(
                              side: FoldStripSide.right,
                              rect: Rect.fromLTWH(875, 0, 76, 669),
                              topClearance: 110,
                              bottomClearance: 0,
                              isClearanceEstimated: true,
                            )
                          : null,
                      active: true,
                      child: Builder(
                        builder: (context) => FrostedScaffold(
                          appBar: const FrostedAppBar(
                            nativeTitle: 'Messages',
                            nativeLargeTitle: true,
                            nativeLargeTitleWithActions: true,
                            alignWithStatusRegion: true,
                            leading: SizedBox(
                              key: Key('menu'),
                              width: 44,
                              height: 44,
                            ),
                          ),
                          body: Padding(
                            padding: EdgeInsets.only(
                              top: frostedAppBarHeight(
                                context,
                                nativeLargeTitle: true,
                                alignWithStatusRegion: true,
                              ),
                            ),
                            child: const Align(
                              alignment: Alignment.topLeft,
                              child: Text('First row'),
                            ),
                          ),
                        ),
                      ),
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      );
      if (!landscape) {
        IosNavigationMetrics.updatePortraitHeader(metrics, [
          669,
          951,
          0,
          20,
          535,
          56,
        ]);
      }
      await tester.pumpAndSettle();
      final title = tester.getRect(
        find.byKey(const ValueKey('expanded-navigation-title')),
      );
      return (
        title.top - tester.getBottomLeft(find.byKey(const Key('menu'))).dy,
        tester.getTopLeft(find.text('First row')).dy - title.bottom,
      );
    }

    final portrait = await measure(landscape: false);
    expect(await measure(landscape: true), portrait);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('Messages expands at the top and collapses in the status row', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.physicalSize = const Size(669, 951);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final offset = ValueNotifier(0.0);
    addTearDown(offset.dispose);
    late BuildContext metricsContext;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: IosNavigationMetricsHost(
            child: Builder(
              builder: (context) {
                metricsContext = context;
                return FrostedScaffold(
                  navigationScrollOffset: offset,
                  appBar: const FrostedAppBar(
                    nativeTitle: 'Messages',
                    nativeLargeTitle: true,
                    nativeLargeTitleWithActions: true,
                    alignWithStatusRegion: true,
                    actions: [
                      SizedBox(key: Key('action'), width: 44, height: 44),
                    ],
                  ),
                  body: const SizedBox.expand(),
                );
              },
            ),
          ),
        ),
      ),
    );
    IosNavigationMetrics.updatePortraitHeader(metricsContext, [
      669,
      951,
      0,
      20,
      535,
      56,
    ]);
    await tester.pumpAndSettle();
    final expanded = find.byKey(const ValueKey('expanded-navigation-title'));
    expect(expanded, findsOneWidget);
    expect(tester.getCenter(find.byKey(const Key('action'))).dy, 48);
    offset.value = 100;
    await tester.pumpAndSettle();
    expect(expanded, findsNothing);
    final bar = find.byType(UiKitView);
    final params = tester.widget<UiKitView>(bar).creationParams! as Map;
    expect(params['alignCollapsedTitleToLeading'], false);
    expect(tester.getSize(bar).width, 669);
    expect(params['titleInsetLeft'], 0);
    expect(params['titleInsetRight'], 134);
    expect(params['leadingTitleRowTop'], 26);
    offset.value = 0;
    await tester.pumpAndSettle();
    expect(expanded, findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });
}
