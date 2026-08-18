import 'package:buzz/features/channels/sticky_date_header.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('updates the native iOS glass date and app theme', (
    tester,
  ) async {
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
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(body: StickyDateHeader(state: state)),
          ),
        ),
      );

      var nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeView.viewType, 'buzz/sticky_date_glass');
      expect(nativeView.creationParams, <String, Object>{
        'label': 'Yesterday',
        'brightness': 'light',
      });
      expect(find.byType(BackdropFilter), findsNothing);

      nativeView.onPlatformViewCreated!(42);
      await tester.pump();
      expect(
        methodCalls.lastWhere((call) => call.method == 'setLabel').arguments,
        'Yesterday',
      );
      expect(
        methodCalls
            .lastWhere((call) => call.method == 'setBrightness')
            .arguments,
        'light',
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

      methodCalls.clear();
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.dark(),
            home: Scaffold(body: StickyDateHeader(state: state)),
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        methodCalls
            .lastWhere((call) => call.method == 'setBrightness')
            .arguments,
        'dark',
      );
    } finally {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      );
      state.dispose();
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('keeps the Flutter date surface on Android', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final state = ValueNotifier(const StickyDateHeaderState(label: 'Today'));
    try {
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(body: StickyDateHeader(state: state)),
          ),
        ),
      );

      expect(find.byType(UiKitView), findsNothing);
      expect(find.byType(BackdropFilter), findsOneWidget);
      expect(find.text('Today'), findsOneWidget);
    } finally {
      state.dispose();
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
