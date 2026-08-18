import 'package:buzz/features/channels/jump_to_latest_button.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('keeps the native iOS glass in sync with the app theme', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const channel = MethodChannel('buzz/jump_to_latest_glass/41');
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
            home: Scaffold(body: JumpToLatestButton(onPressed: () {})),
          ),
        ),
      );

      final nativeView = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(nativeView.viewType, 'buzz/jump_to_latest_glass');
      expect(nativeView.creationParams, <String, Object>{
        'brightness': 'light',
      });
      expect(
        find.byKey(const ValueKey('channel-jump-to-latest-ios-glass')),
        findsOneWidget,
      );

      nativeView.onPlatformViewCreated!(41);
      await tester.pump();
      expect(
        methodCalls
            .lastWhere((call) => call.method == 'setBrightness')
            .arguments,
        'light',
      );

      methodCalls.clear();
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.dark(),
            home: Scaffold(body: JumpToLatestButton(onPressed: () {})),
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
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
