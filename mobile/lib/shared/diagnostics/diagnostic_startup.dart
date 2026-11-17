import 'dart:async';

import 'package:flutter/foundation.dart';
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
/// errors and uncaught asynchronous Dart errors from then on. A destination
/// the SDK cannot parse is treated as none, so a configuration mistake never
/// stops the app from starting.
Future<void> runWithDiagnostics({
  required String destination,
  required FutureOr<void> Function(DiagnosticReporter reporter) app,
}) async {
  if (!_isUsable(destination)) return app(const NoopDiagnosticReporter());
  await SentryFlutter.init(
    (options) => configureDiagnostics(options, destination),
    appRunner: () => app(SentryDiagnosticReporter()),
  );
}

bool _isUsable(String destination) {
  if (destination.isEmpty) return false;
  try {
    Dsn.parse(destination);
    return true;
  } on Object catch (error) {
    // Dsn.parse throws ArgumentError or FormatException; the message would
    // repeat the destination, so only the type is printed.
    debugPrint(
      '[Diagnostics] reporting off: unusable destination '
      '(${error.runtimeType})',
    );
    return false;
  }
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
    // On iOS this is Cocoa's own network breadcrumbs, which keep each
    // request's query and fragment and reach native crash reports, which
    // never pass through the filter below.
    ..recordHttpBreadcrumbs = false
    // Otherwise the SDK replaces debugPrint in non-debug builds with one that
    // records a breadcrumb and prints nothing.
    ..enablePrintBreadcrumbs = false
    ..enableUserInteractionBreadcrumbs = false
    ..enableAppLifecycleBreadcrumbs = true
    // Main-thread hangs, at the SDK's default thresholds. Dart runs on the
    // main thread, so blocking Dart work is what these catch.
    ..enableAppHangTracking = true
    ..anrEnabled = true
    ..enableWatchdogTerminationTracking = true
    ..beforeSend = ((event, hint) => DiagnosticFilter.filterEvent(event))
    ..beforeBreadcrumb = ((crumb, hint) =>
        DiagnosticFilter.filterBreadcrumb(crumb))
    ..beforeSendMetric = ((metric, hint) =>
        DiagnosticFilter.filterMetric(metric));
}
