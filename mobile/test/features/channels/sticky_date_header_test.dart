import 'package:buzz/features/channels/message_action_backdrop_state.dart';
import 'package:buzz/features/channels/sticky_date_header.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'keeps the native iOS date compact and stationary during push-off',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final state = ValueNotifier(
        const StickyDateHeaderState(label: 'Yesterday'),
      );
      const channel = MethodChannel('buzz/sticky_date_glass/42');
      final methodCalls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        methodCalls.add(call);
        return null;
      });
      try {
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(body: StickyDateHeader(state: state)),
          ),
        );

        var nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
        expect(nativeView.viewType, 'buzz/sticky_date_glass');
        expect(nativeView.creationParams, <String, Object>{
          'label': 'Yesterday',
          'brightness': 'light',
        });
        expect(find.byType(BackdropFilter), findsNothing);
        final nativeSurface = find.byKey(
          const ValueKey('channel-sticky-date-header-surface'),
        );
        expect(
          tester.getSize(nativeSurface).width,
          lessThan(tester.getSize(find.byType(StickyDateHeader)).width),
        );
        expect(
          tester.getSize(nativeSurface).height,
          StickyDateHeader.heightOf(
            tester.element(find.byType(StickyDateHeader)),
          ),
        );
        expect(
          find.byKey(const ValueKey('sticky-date-header-clip')),
          findsNothing,
        );
        final initialSurfaceTop = tester.getTopLeft(nativeSurface).dy;
        final stickyHeight = StickyDateHeader.heightOf(
          tester.element(find.byType(StickyDateHeader)),
        );

        state.value = StickyDateHeaderState(
          label: 'Yesterday',
          translateY: -(stickyHeight + 5) / 2,
        );
        await tester.pump();
        expect(
          tester
              .widget<Opacity>(
                find.byKey(const ValueKey('sticky-date-push-off-opacity')),
              )
              .opacity,
          closeTo(0.5, 0.001),
        );
        expect(
          tester.getTopLeft(nativeSurface).dy,
          closeTo(initialSurfaceTop, 0.01),
          reason: 'Native glass must not cross the app-bar compositing edge.',
        );

        state.value = StickyDateHeaderState(
          label: 'Yesterday',
          translateY: -(stickyHeight + 5),
        );
        await tester.pump();
        expect(
          tester
              .widget<Opacity>(
                find.byKey(const ValueKey('sticky-date-push-off-opacity')),
              )
              .opacity,
          0,
        );

        nativeView.onPlatformViewCreated!(42);
        await tester.pump();
        expect(
          methodCalls.lastWhere((call) => call.method == 'setLabel').arguments,
          'Yesterday',
        );

        state.value = const StickyDateHeaderState(label: 'Today');
        await tester.pump();
        nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
        expect(nativeView.creationParams, <String, Object>{
          'label': 'Today',
          'brightness': 'light',
        });
        expect(
          methodCalls.lastWhere((call) => call.method == 'setLabel').arguments,
          'Today',
        );
        expect(
          tester
              .widget<Opacity>(
                find.byKey(const ValueKey('sticky-date-push-off-opacity')),
              )
              .opacity,
          1,
        );

        messageActionBackdropActive.value = true;
        await tester.pump();
        expect(find.byType(UiKitView), findsNothing);
        expect(find.byType(BackdropFilter), findsOneWidget);
        expect(
          find.byKey(const ValueKey('sticky-date-header-clip')),
          findsNothing,
        );
        expect(
          tester
              .getSize(
                find.byKey(
                  const ValueKey('channel-sticky-date-header-surface'),
                ),
              )
              .width,
          lessThan(tester.getSize(find.byType(StickyDateHeader)).width),
        );
      } finally {
        messageActionBackdropActive.value = false;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        );
        state.dispose();
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('keeps the compact Flutter date surface on Android', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final state = ValueNotifier(const StickyDateHeaderState(label: 'Today'));
    try {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(body: StickyDateHeader(state: state)),
        ),
      );

      expect(find.byType(UiKitView), findsNothing);
      expect(find.byType(BackdropFilter), findsOneWidget);
      expect(find.text('Today'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('sticky-date-header-clip')),
        findsNothing,
      );
      final flutterSurface = find.byKey(
        const ValueKey('channel-sticky-date-header-surface'),
      );
      final initialSurfaceTop = tester.getTopLeft(flutterSurface).dy;
      final stickyHeight = StickyDateHeader.heightOf(
        tester.element(find.byType(StickyDateHeader)),
      );

      state.value = StickyDateHeaderState(
        label: 'Today',
        translateY: -(stickyHeight + 5) / 2,
      );
      await tester.pump();
      expect(
        tester
            .widget<Opacity>(
              find.byKey(const ValueKey('sticky-date-push-off-opacity')),
            )
            .opacity,
        closeTo(0.5, 0.001),
      );
      expect(
        tester.getTopLeft(flutterSurface).dy,
        closeTo(initialSurfaceTop, 0.01),
        reason: 'Android uses the same stationary date handoff as iOS.',
      );
      expect(
        tester
            .getSize(
              find.byKey(const ValueKey('channel-sticky-date-header-surface')),
            )
            .width,
        lessThan(tester.getSize(find.byType(StickyDateHeader)).width),
      );
    } finally {
      state.dispose();
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
