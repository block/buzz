import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:sentry_flutter/sentry_flutter.dart';

import 'diagnostic_filter.dart';
import 'diagnostic_reporter.dart';

/// Sends diagnostic reports through the Sentry SDK.
///
/// The only file besides startup that imports the SDK. Calls hand their work
/// to the SDK without waiting, and a failure inside reporting is dropped: the
/// spec requires that reporting never fails or delays the caller.
class SentryDiagnosticReporter implements DiagnosticReporter {
  SentryDiagnosticReporter({Hub? hub, SentryMetrics? metrics})
    : _hub = hub ?? HubAdapter(),
      _metricsOverride = metrics;

  final Hub _hub;
  final SentryMetrics? _metricsOverride;

  // Read on each call: the SDK installs its metrics when it starts.
  SentryMetrics get _metrics => _metricsOverride ?? Sentry.metrics;

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) => _drop(
    () => _hub.captureException(
      error,
      stackTrace: stackTrace,
      withScope: (scope) => scope.setTag('operation', operation.name),
    ),
  );

  @override
  void countNetworkEvent(NetworkEvent event) => _drop(() {
    final attributes = _attributes(event);
    _metrics.count('network.event', 1, attributes: attributes);
    if (event.duration case final duration?) {
      _metrics.distribution(
        'network.duration',
        duration.inMicroseconds / 1000,
        unit: SentryMetricUnit.millisecond,
        attributes: attributes,
      );
    }
    return null;
  });

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) => _drop(
    () => _hub.addBreadcrumb(
      Breadcrumb(
        category: DiagnosticFilter.networkCategory,
        level: breadcrumb.event.outcome == NetworkOutcome.failure
            ? SentryLevel.warning
            : SentryLevel.info,
        data: {
          for (final entry in _attributes(breadcrumb.event).entries)
            entry.key: entry.value.value,
        },
      ),
    ),
  );

  @override
  String? get member => _member;
  String? _member;

  @override
  int get memberChanges => _memberChanges;
  var _memberChanges = 0;

  @override
  void setMember(String? pubkey) {
    final previous = _member;
    _member = pubkey;
    if (pubkey != previous) _memberChanges++;
    _drop(
      () => _hub.configureScope((scope) {
        final user = scope.setUser(
          pubkey == null ? null : SentryUser(id: pubkey),
        );
        // The trail holds the previous member's network events, with their
        // relay's host. Leaving a member clears it, so none of them reach
        // the next member's reports.
        if (previous == null || previous == pubkey) return user;
        return Future.wait([user, scope.clearBreadcrumbs()]);
      }),
    );
  }

  @override
  bool get reporting => true;

  @override
  void sendTestReport() => reportError(
    DiagnosticTestError(),
    StackTrace.current,
    operation: DiagnosticOperation.testReport,
  );

  @override
  void crashForTest() => _drop(SentryFlutter.nativeCrash);

  static Map<String, SentryAttribute> _attributes(NetworkEvent event) => {
    'backend': SentryAttribute.string(event.backend),
    'endpoint': SentryAttribute.string(event.endpoint.name),
    'outcome': SentryAttribute.string(event.outcome.name),
    if (event.failure case final failure?)
      'failure': SentryAttribute.string(failure.name),
  };

  static void _drop(Object? Function() send) {
    try {
      final result = send();
      if (result is Future<Object?>) {
        unawaited(result.then<Object?>((_) => null, onError: _dropped));
      }
    } catch (error) {
      _dropped(error);
    }
  }

  static Null _dropped(Object error) {
    debugPrint('[Diagnostics] report dropped: ${error.runtimeType}');
    return null;
  }
}
