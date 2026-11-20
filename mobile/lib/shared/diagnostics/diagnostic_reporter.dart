import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The one interface app code uses for diagnostic reporting.
///
/// See `docs/mobile-diagnostic-reporting.md`. Every call returns at once and
/// never throws: a failure inside reporting is dropped so it cannot reach the
/// code that made the call.
abstract interface class DiagnosticReporter {
  /// Reports something that should never happen, such as a frame the app
  /// cannot decode. [operation] names where it happened, from a fixed list.
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  });

  /// Counts one networking event, with how long it took when known.
  void countNetworkEvent(NetworkEvent event);

  /// Adds one entry to the trail attached to later reports.
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb);

  /// Attributes later reports and counts to [pubkey], or to no one.
  void setMember(String? pubkey);

  /// The member later reports and counts name, as last given to [setMember].
  String? get member;

  /// Whether this build has a reporting destination. Only then does Settings
  /// offer the checks below.
  bool get reporting;

  /// Sends a [DiagnosticTestError], to check a build's reporting end to end.
  void sendTestReport();

  /// Crashes the app in native code, to check native crash reports.
  void crashForTest();
}

/// The error a test report carries, so it is easy to tell apart.
///
/// Not const: each report needs its own instance, or the SDK's own
/// deduplication drops a repeat of the last error object.
class DiagnosticTestError implements Exception {
  DiagnosticTestError();

  @override
  String toString() => 'DiagnosticTestError';
}

/// Where an explicit error report comes from.
enum DiagnosticOperation {
  relayFrame,
  relayQuery,
  relayRejection,
  accountResponse,
  pushGatewayResponse,
  storedData,
  eventSubscriber,
  testReport,
}

/// The operation a networking event belongs to.
enum NetworkEndpoint { relayConnection, relayQuery, relayPublish }

/// How a networking event ended.
enum NetworkOutcome { success, failure }

/// Why a networking event failed.
enum NetworkFailure {
  timeout,
  disconnected,
  offline,
  unreachable,
  insecure,
  rateLimited,
  authRefused,
  serverError,
  relayError,
  relayRefused,
}

/// One counted networking event. [backend] is the server's host name.
class NetworkEvent {
  const NetworkEvent({
    required this.backend,
    required this.endpoint,
    required this.outcome,
    this.failure,
    this.duration,
  }) : assert((outcome == NetworkOutcome.failure) == (failure != null));

  final String backend;
  final NetworkEndpoint endpoint;
  final NetworkOutcome outcome;
  final NetworkFailure? failure;
  final Duration? duration;
}

/// One entry in the trail of recent networking events.
class DiagnosticBreadcrumb {
  const DiagnosticBreadcrumb(this.event);

  /// Breadcrumbs from the fixed list are networking events.
  final NetworkEvent event;
}

/// Used when the build has no reporting destination.
class NoopDiagnosticReporter implements DiagnosticReporter {
  const NoopDiagnosticReporter();

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    // A throwing subscriber is a bug in the app, and the delivery guard
    // catches it. With no destination, surface it the way an uncaught throw
    // would: it fails the test that hit it and prints in development.
    if (operation == DiagnosticOperation.eventSubscriber) {
      Zone.current.handleUncaughtError(error, stackTrace ?? StackTrace.current);
    }
  }

  @override
  void countNetworkEvent(NetworkEvent event) {}

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) {}

  @override
  void setMember(String? pubkey) {}

  @override
  String? get member => null;

  @override
  bool get reporting => false;

  @override
  void sendTestReport() {}

  @override
  void crashForTest() {}
}

/// Reports from work signed by [signer], such as a one-shot socket for a
/// community that may not be active. They go through while [reporter] names
/// [signer] or no member (reports then carry the install identifier), and are
/// dropped while it names another member, who didn't hit them.
class SignerDiagnosticReporter implements DiagnosticReporter {
  const SignerDiagnosticReporter(this.reporter, {required this.signer});

  final DiagnosticReporter reporter;
  final String? signer;

  bool get _current => switch (reporter.member) {
    null => true,
    final member => member == signer,
  };

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    if (_current) {
      reporter.reportError(error, stackTrace, operation: operation);
    }
  }

  @override
  void countNetworkEvent(NetworkEvent event) {
    if (_current) reporter.countNetworkEvent(event);
  }

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) {
    if (_current) reporter.leaveBreadcrumb(breadcrumb);
  }

  /// Signed work never changes who the app's reports name.
  @override
  void setMember(String? pubkey) {}

  @override
  String? get member => reporter.member;

  @override
  bool get reporting => reporter.reporting;

  @override
  void sendTestReport() => reporter.sendTestReport();

  @override
  void crashForTest() => reporter.crashForTest();
}

/// The reporter chosen at startup. The root scope overrides it; tests
/// override it with a fake that records what it was given.
final diagnosticReporterProvider = Provider<DiagnosticReporter>(
  (ref) => const NoopDiagnosticReporter(),
);
