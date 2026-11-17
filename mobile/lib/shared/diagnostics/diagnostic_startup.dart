import 'dart:async';

import 'package:sentry_flutter/sentry_flutter.dart';

import 'diagnostic_filter.dart';
import 'diagnostic_reporter.dart';
import 'sentry_diagnostic_reporter.dart';

/// The build's reporting destination, a Sentry DSN supplied at build time.
/// Builds made from this repository alone have none and report nothing.
const reportingDestination = String.fromEnvironment('BUZZ_SENTRY_DSN');

/// Runs [app] with the reporter this build calls for.
///
/// Without a [destination] the SDK is never initialized: a reporting library
/// given an empty destination can still install its hooks (ADR 0007). With
/// one, the SDK's own handlers capture native crashes, Flutter framework
/// errors and uncaught asynchronous Dart errors from then on.
Future<void> runWithDiagnostics({
  required String destination,
  required FutureOr<void> Function(DiagnosticReporter reporter) app,
}) async {
  if (destination.isEmpty) return app(const NoopDiagnosticReporter());
  await SentryFlutter.init(
    (options) => configureDiagnostics(options, destination),
    appRunner: () => app(SentryDiagnosticReporter()),
  );
}

/// Applies the reporting settings from `docs/mobile-diagnostic-reporting.md`.
void configureDiagnostics(SentryFlutterOptions options, String destination) {
  options
    ..dsn = destination
    ..sendDefaultPii = false
    ..attachScreenshot = false
    ..attachViewHierarchy = false
    // Breadcrumbs come only from the app's fixed list and the SDK's
    // foreground/background entries; native ones can carry addresses.
    ..enableAutoNativeBreadcrumbs = false
    ..enableUserInteractionBreadcrumbs = false
    ..enableAppLifecycleBreadcrumbs = true
    ..beforeSend = ((event, hint) => DiagnosticFilter.filterEvent(event))
    ..beforeBreadcrumb = ((crumb, hint) =>
        DiagnosticFilter.filterBreadcrumb(crumb))
    ..beforeSendMetric = ((metric, hint) =>
        DiagnosticFilter.filterMetric(metric));
}
