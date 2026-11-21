import 'package:clock/clock.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/diagnostics/diagnostic_reporter.dart';
import '../../shared/diagnostics/network_event_reporting.dart';
import '../../shared/http/app_http_client.dart';
import '../../shared/relay/relay.dart';

/// Times a channel opening: from asking for its newest history to showing
/// it, or to the failure that stopped it. A catch-up after a reconnect is not
/// an open, so callers time only a channel's first load.
///
/// The relay and the reporter's member are fixed when the request starts. An
/// open that finishes after the member changed belonged to someone else and
/// is not counted.
class ChannelOpenTimer {
  ChannelOpenTimer(Ref ref)
    : _reporter = ref.read(diagnosticReporterProvider),
      _host = Uri.tryParse(ref.read(relayConfigProvider).baseUrl)?.host,
      _startedAt = clock.now() {
    _memberChanges = _reporter.memberChanges;
  }

  /// Continues [pending] if it is still running, since an open cut short by
  /// a reconnect keeps its clock and the wait counts; otherwise starts one.
  static ChannelOpenTimer resume(ChannelOpenTimer? pending, Ref ref) =>
      pending != null && !pending.isDone ? pending : ChannelOpenTimer(ref);

  final DiagnosticReporter _reporter;
  final String? _host;
  final DateTime _startedAt;
  late final int _memberChanges;
  bool _done = false;

  /// Whether the open was already counted or dropped.
  bool get isDone => _done;

  /// Counts the open once, as a failure when [error] is set. Later calls do
  /// nothing.
  void finish({Object? error}) {
    if (_done) return;
    _done = true;
    final host = _host;
    if (host == null || host.isEmpty) return;
    if (_reporter.memberChanges != _memberChanges) return;
    final failure = error == null ? null : _failure(error);
    _reporter.noteNetworkEvent(
      NetworkEvent(
        backend: host,
        endpoint: NetworkEndpoint.channelOpen,
        outcome: failure == null
            ? NetworkOutcome.success
            : NetworkOutcome.failure,
        failure: failure,
        duration: clock.now().difference(_startedAt),
      ),
    );
  }

  /// Drops the open uncounted, as when nobody is waiting for it any more.
  void cancel() => _done = true;
}

/// The kind of failure that ended an open: the relay's own answers first,
/// then the transport's.
NetworkFailure _failure(Object error) {
  if (isRelayChannelAccessDenied(error)) return NetworkFailure.relayRefused;
  if (isRelayDeadlineError(error)) return NetworkFailure.timeout;
  if (error is RelayException) {
    return switch (error.statusCode) {
      429 => NetworkFailure.rateLimited,
      >= 500 => NetworkFailure.serverError,
      >= 400 => NetworkFailure.relayRefused,
      _ => NetworkFailure.other,
    };
  }
  final closed = error.toString().replaceFirst(RegExp(r'^Exception:\s*'), '');
  if (classifyRelayClosed(closed) == RelayClosedClass.rateLimited) {
    return NetworkFailure.rateLimited;
  }
  return classifyTransportError(error) ?? NetworkFailure.other;
}
