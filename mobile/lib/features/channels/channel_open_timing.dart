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
/// The relay and its member are fixed when the request starts. An open that
/// finishes after the reporter moved to another member belonged to someone
/// else, and one that spanned a move to the background measured the time
/// away, so neither is counted. Nor is one started in the background.
class ChannelOpenTimer {
  factory ChannelOpenTimer(Ref ref) {
    final config = ref.read(relayConfigProvider);
    final reporter = ref.read(diagnosticReporterProvider);
    return ChannelOpenTimer._(
      SignerDiagnosticReporter(reporter, signer: pubkeyFromNsec(config.nsec)),
      Uri.tryParse(config.baseUrl)?.host,
      ref.read(relaySessionProvider.notifier),
    );
  }

  ChannelOpenTimer._(this._reporter, this._host, this._session)
    : _memberChanges = _reporter.memberChanges,
      _pauses = _session.pauses,
      _startedAt = clock.now(),
      _done = _session.backgrounded;

  /// Continues [pending] if it is still running, since an open cut short by
  /// a reconnect keeps its clock and the wait counts; otherwise starts one.
  static ChannelOpenTimer resume(ChannelOpenTimer? pending, Ref ref) =>
      pending != null && !pending.isDone ? pending : ChannelOpenTimer(ref);

  /// Ends [timer] with the relay's refusal. A first open refused before its
  /// history was asked for has no timer and counts with no duration.
  static void refused(
    Ref ref,
    ChannelOpenTimer? timer,
    Object error, {
    required bool loaded,
  }) {
    if (timer != null) return timer.finish(error: error);
    if (!loaded) ChannelOpenTimer(ref)._count(error, timed: false);
  }

  final DiagnosticReporter _reporter;
  final String? _host;
  final RelaySessionNotifier _session;
  final int _memberChanges;
  final int _pauses;
  final DateTime _startedAt;
  bool _done;

  /// Whether the open was already counted or dropped, or spanned a move to
  /// the background, so a later load starts a new one.
  bool get isDone => _done || _session.pauses != _pauses;

  /// Counts the open once, as a failure when [error] is set. Later calls do
  /// nothing.
  void finish({Object? error}) {
    if (isDone) return;
    _done = true;
    _count(error);
  }

  void _count(Object? error, {bool timed = true}) {
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
        duration: timed ? clock.now().difference(_startedAt) : null,
      ),
    );
  }
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
