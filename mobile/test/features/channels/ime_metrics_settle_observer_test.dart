import 'package:buzz/features/channels/ime_metrics_settle_observer.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('coalesces Android IME metrics until the viewport settles', (
    tester,
  ) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      var callbacks = 0;
      final observer = ImeMetricsSettleObserver(
        onMetricsSettled: () => callbacks += 1,
      );
      addTearDown(observer.dispose);

      observer.didChangeMetrics();
      await tester.pump(const Duration(milliseconds: 60));
      observer.didChangeMetrics();
      await tester.pump(const Duration(milliseconds: 119));
      expect(callbacks, 0);

      await tester.pump(const Duration(milliseconds: 1));
      expect(callbacks, 1);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });

  testWidgets('keeps non-Android metrics callbacks immediate', (tester) async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      var callbacks = 0;
      final observer = ImeMetricsSettleObserver(
        onMetricsSettled: () => callbacks += 1,
      );
      addTearDown(observer.dispose);

      observer.didChangeMetrics();
      observer.didChangeMetrics();

      expect(callbacks, 2);
    } finally {
      debugDefaultTargetPlatformOverride = previousPlatform;
    }
  });
}
