import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';

/// A reporter that records what it was given, and who the member was when it
/// was given.
class RecordingDiagnostics extends NoopDiagnosticReporter {
  final counts = <({NetworkEvent event, String? member})>[];

  /// The networking breadcrumbs left, in order.
  final breadcrumbs = <NetworkEvent>[];
  final errors = <(Type, DiagnosticOperation)>[];
  String? _member;

  Iterable<NetworkEvent> get events => counts.map((count) => count.event);

  /// Each counted event as `(endpoint, failure)`, a success having none.
  List<(NetworkEndpoint, NetworkFailure?)> get outcomes => [
    for (final event in events) (event.endpoint, event.failure),
  ];

  /// Forgets everything recorded so far.
  void clear() {
    counts.clear();
    breadcrumbs.clear();
  }

  @override
  String? get member => _member;

  @override
  void setMember(String? pubkey) {
    if (pubkey != _member) _memberChanges++;
    _member = pubkey;
  }

  @override
  int get memberChanges => _memberChanges;
  var _memberChanges = 0;

  @override
  void countNetworkEvent(NetworkEvent event) =>
      counts.add((event: event, member: _member));

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) {
    if (breadcrumb case NetworkBreadcrumb(:final event)) breadcrumbs.add(event);
  }

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) => errors.add((error.runtimeType, operation));
}

/// A reporter whose every call fails, to prove reporting cannot reach the
/// work it observes.
class ThrowingDiagnostics extends NoopDiagnosticReporter {
  @override
  void countNetworkEvent(NetworkEvent event) => throw StateError('down');

  @override
  void leaveBreadcrumb(DiagnosticBreadcrumb breadcrumb) =>
      throw StateError('down');
}
