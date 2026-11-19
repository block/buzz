import 'package:buzz/shared/widgets/bottom_up_page_route.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_action.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (_) async => null,
    );
  });
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      null,
    );
  });

  for (final bottomUp in [true, false]) {
    testWidgets(
      'glass controls respect covering route composition: bottomUp=$bottomUp',
      (tester) async {
        final navigator = GlobalKey<NavigatorState>();
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              navigatorKey: navigator,
              home: Scaffold(
                body: Row(
                  key: const ValueKey('underlying-controls'),
                  children: [
                    IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.more,
                      label: 'All',
                      semanticLabel: 'All',
                      onPressed: () {},
                    ),
                    IosGlassNavigationAction(label: 'Done', onPressed: () {}),
                    const IosMessageCreateButton(actions: []),
                  ],
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final controls = find.byKey(
          const ValueKey('underlying-controls'),
          skipOffstage: false,
        );
        final nativeViews = find.descendant(
          of: controls,
          matching: find.byType(UiKitView, skipOffstage: false),
        );
        final initialSize = tester.getSize(controls);
        final initialPosition = tester.getTopLeft(controls);
        expect(nativeViews, findsNWidgets(3));
        Widget destination(BuildContext context) => Scaffold(
          key: const ValueKey('moving-sheet'),
          body: IosGlassNavigationButton(
            icon: IosGlassNavigationIcon.close,
            semanticLabel: 'Close incoming screen',
            onPressed: () => Navigator.of(context).pop(),
          ),
        );
        navigator.currentState!.push(
          bottomUp
              ? BottomUpPageRoute<void>(
                  builder: destination,
                  reducedMotion: false,
                )
              : MaterialPageRoute<void>(builder: destination),
        );
        await tester.pump();
        expect(
          nativeViews,
          bottomUp ? findsNWidgets(3) : findsNothing,
          reason: 'sheets cover mounted controls; other routes suppress them',
        );
        expect(tester.getSize(controls), initialSize);
        await tester.pump(const Duration(milliseconds: 100));
        expect(nativeViews, bottomUp ? findsNWidgets(3) : findsNothing);
        expect(
          find.byWidgetPredicate(
            (w) => w is UiKitView && w.viewType == 'buzz/navigation_glass',
          ),
          bottomUp ? findsNWidgets(4) : findsOneWidget,
          reason: 'incoming control stays visible',
        );
        if (bottomUp) {
          expect(tester.getTopLeft(controls), initialPosition);
          final backing = find.byWidgetPredicate(
            (w) => w is UiKitView && w.viewType == 'buzz/toast_layer',
          );
          expect(backing, findsOneWidget);
          expect(
            tester.getSize(backing),
            tester.view.physicalSize / tester.view.devicePixelRatio,
          );
          final params =
              tester.widget<UiKitView>(backing).creationParams as Map;
          expect((params['color'] as int) >> 24, 255);
        }
        await tester.pumpAndSettle();
        navigator.currentState!.pop();
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));
        expect(
          nativeViews,
          bottomUp ? findsNWidgets(3) : findsNothing,
          reason: 'sheet reveals mounted controls throughout dismissal',
        );
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('moving-sheet')),
            matching: find.byWidgetPredicate(
              (w) => w is UiKitView && w.viewType == 'buzz/navigation_glass',
            ),
          ),
          findsOneWidget,
          reason: 'the departing sheet keeps its own controls until it exits',
        );
        if (bottomUp) {
          expect(tester.getTopLeft(controls), initialPosition);
          expect(
            tester.getTopLeft(find.byKey(const ValueKey('moving-sheet'))).dy,
            greaterThan(0),
          );
        }
        await tester.pumpAndSettle();
        expect(nativeViews, findsNWidgets(3));
        expect(tester.getSize(controls), initialSize);
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }
}
