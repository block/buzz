// Sentry marks these stable configuration fields as experimental.
// ignore_for_file: experimental_member_use

import 'package:sentry_flutter/sentry_flutter.dart';

/// Compile-time Sentry settings injected only by official release builds.
class SentryConfig {
  const SentryConfig({
    required this.dsn,
    required this.release,
    required this.dist,
    required this.environment,
  });

  const SentryConfig.fromEnvironment()
    : dsn = const String.fromEnvironment('SENTRY_DSN'),
      release = const String.fromEnvironment('SENTRY_RELEASE'),
      dist = const String.fromEnvironment('SENTRY_DIST'),
      environment = const String.fromEnvironment('SENTRY_ENVIRONMENT');

  final String dsn;
  final String release;
  final String dist;
  final String environment;

  bool get isConfigured => dsn.trim().isNotEmpty;

  void applyTo(SentryFlutterOptions options) {
    final trimmedDsn = dsn.trim();
    final trimmedRelease = release.trim();
    final trimmedDist = dist.trim();
    final trimmedEnvironment = environment.trim();

    options
      ..dsn = trimmedDsn
      ..sendDefaultPii = false
      ..attachScreenshot = false
      ..attachViewHierarchy = false
      ..reportViewHierarchyIdentifiers = false
      ..enableAutoSessionTracking = false
      ..enableWatchdogTerminationTracking = false
      ..enableAppHangTracking = false
      ..anrEnabled = false
      ..enableTombstone = false
      ..enableNdkScopeSync = false
      ..enableLogs = false
      ..enableMetrics = false
      ..reportPackages = false
      ..sendClientReports = false
      ..maxBreadcrumbs = 0
      ..tracesSampleRate = 0
      ..profilesSampleRate = 0
      ..enableAutoPerformanceTracing = false
      ..enableUserInteractionTracing = false
      ..enableUserInteractionBreadcrumbs = false
      ..enableTimeToFullDisplayTracing = false
      ..enableFramesTracking = false
      ..enableNativeTraceSync = false
      ..enablePrintBreadcrumbs = false
      ..enableAutoNativeBreadcrumbs = false
      ..enableAppLifecycleBreadcrumbs = false
      ..enableWindowMetricBreadcrumbs = false
      ..enableBrightnessChangeBreadcrumbs = false
      ..enableTextScaleChangeBreadcrumbs = false
      ..enableMemoryPressureBreadcrumbs = false
      ..recordHttpBreadcrumbs = false
      ..captureFailedRequests = false
      ..captureNativeFailedRequests = false
      ..maxRequestBodySize = MaxRequestBodySize.never
      ..replay.sessionSampleRate = 0
      ..replay.onErrorSampleRate = 0
      ..privacy.maskAllText = true
      ..privacy.maskAllImages = true;

    if (trimmedRelease.isNotEmpty) {
      options.release = trimmedRelease;
    }
    if (trimmedDist.isNotEmpty) {
      options.dist = trimmedDist;
    }
    if (trimmedEnvironment.isNotEmpty) {
      options.environment = trimmedEnvironment;
    }
  }
}
