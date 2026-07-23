// Sentry marks these stable configuration fields as experimental.
// ignore_for_file: experimental_member_use

import 'package:buzz/shared/diagnostics/diagnostics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sentry_flutter/sentry_flutter.dart';

void main() {
  test('empty DSN is not configured', () {
    const config = SentryConfig(
      dsn: '  ',
      release: '',
      dist: '',
      environment: '',
    );

    expect(config.isConfigured, isFalse);
  });

  test('omits blank optional release metadata', () {
    const config = SentryConfig(
      dsn: 'https://public@example.invalid/1',
      release: '  ',
      dist: '',
      environment: '\n',
    );
    final options = SentryFlutterOptions();

    config.applyTo(options);

    expect(options.release, isNull);
    expect(options.dist, isNull);
    expect(options.environment, isNull);
  });

  test('applies crash-only privacy configuration', () {
    const config = SentryConfig(
      dsn: ' https://public@example.invalid/1 ',
      release: ' buzz@1.2.3 ',
      dist: ' 42 ',
      environment: ' production ',
    );
    final options = SentryFlutterOptions();

    config.applyTo(options);

    expect(options.dsn, 'https://public@example.invalid/1');
    expect(options.release, 'buzz@1.2.3');
    expect(options.dist, '42');
    expect(options.environment, 'production');
    expect(options.sendDefaultPii, isFalse);
    expect(options.markAutomaticallyCollectedErrorsAsFatal, isTrue);
    expect(options.attachScreenshot, isFalse);
    expect(options.attachViewHierarchy, isFalse);
    expect(options.reportViewHierarchyIdentifiers, isFalse);
    expect(options.enableAutoSessionTracking, isFalse);
    expect(options.enableWatchdogTerminationTracking, isFalse);
    expect(options.enableAppHangTracking, isFalse);
    expect(options.anrEnabled, isFalse);
    expect(options.enableTombstone, isFalse);
    expect(options.enableNdkScopeSync, isFalse);
    expect(options.enableLogs, isFalse);
    expect(options.enableMetrics, isFalse);
    expect(options.reportPackages, isFalse);
    expect(options.sendClientReports, isFalse);
    expect(options.maxBreadcrumbs, 0);
    expect(options.tracesSampleRate, 0);
    expect(options.profilesSampleRate, 0);
    expect(options.enableAutoPerformanceTracing, isFalse);
    expect(options.enableUserInteractionTracing, isFalse);
    expect(options.enableUserInteractionBreadcrumbs, isFalse);
    expect(options.enableTimeToFullDisplayTracing, isFalse);
    expect(options.enableFramesTracking, isFalse);
    expect(options.enableNativeTraceSync, isFalse);
    expect(options.enablePrintBreadcrumbs, isFalse);
    expect(options.enableAutoNativeBreadcrumbs, isFalse);
    expect(options.enableAppLifecycleBreadcrumbs, isFalse);
    expect(options.enableWindowMetricBreadcrumbs, isFalse);
    expect(options.enableBrightnessChangeBreadcrumbs, isFalse);
    expect(options.enableTextScaleChangeBreadcrumbs, isFalse);
    expect(options.enableMemoryPressureBreadcrumbs, isFalse);
    expect(options.recordHttpBreadcrumbs, isFalse);
    expect(options.captureFailedRequests, isFalse);
    expect(options.captureNativeFailedRequests, isFalse);
    expect(options.maxRequestBodySize, MaxRequestBodySize.never);
    expect(options.replay.sessionSampleRate, 0);
    expect(options.replay.onErrorSampleRate, 0);
    expect(options.privacy.maskAllText, isTrue);
    expect(options.privacy.maskAllImages, isTrue);
  });
}
