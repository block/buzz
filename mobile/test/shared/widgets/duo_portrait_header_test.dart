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
}
