part of 'relay_session.dart';

/// Counts the session's networking events: connecting, losing a connection,
/// reconnecting, and what the relay does with queries, publishes and
/// subscriptions. Routine failures are counted here and never reported as
/// errors. The session's [_diagnostics] reporter drops counts made for
/// another member than the one this session signs as.
extension _RelaySessionDiagnostics on RelaySessionNotifier {
  void _noteNetwork(
    NetworkEndpoint endpoint, {
    NetworkFailure? failure,
    DateTime? since,
    Duration? duration,
  }) {
    final host = Uri.tryParse(_relayUrl ?? '')?.host;
    if (host == null || host.isEmpty) return;
    _diagnostics.noteNetworkEvent(
      NetworkEvent(
        backend: host,
        endpoint: endpoint,
        outcome: failure == null
            ? NetworkOutcome.success
            : NetworkOutcome.failure,
        failure: failure,
        duration: duration ?? (since == null ? null : _now().difference(since)),
      ),
    );
  }

  /// Counts the end of a connect attempt or of an open connection, once: a
  /// socket that fails while authenticating reports it twice.
  void _noteDisconnected(Object? error) {
    final since = _connectStartedAt;
    if (since == null && _connectedAt == null) return;
    _connectStartedAt = null;
    _noteNetwork(
      NetworkEndpoint.relayConnection,
      failure: _connectionFailure(error),
      // How long a failed attempt took. A lost connection has no duration.
      since: _connectedAt == null ? since : null,
    );
  }

  NetworkFailure _connectionFailure(Object? error) {
    if (error is RelayAuthRejectedException || error == AccountFailure.denied) {
      return NetworkFailure.authRefused;
    }
    if (error is AccountFailure) return NetworkFailure.other;
    if (error is Exception &&
        error.toString().startsWith('Exception: error:')) {
      return NetworkFailure.relayError;
    }
    // A close with no error, or one this app has no name for, is a lost
    // connection.
    return classifyTransportError(error) ?? NetworkFailure.disconnected;
  }
}

/// The failure kind of a relay `OK` or `CLOSED` reason. Only the prefix is
/// read: the rest is free text.
NetworkFailure _relayMessageFailure(String message) {
  final text = message.trim().toLowerCase();
  if (classifyRelayClosed(text) == RelayClosedClass.rateLimited) {
    return NetworkFailure.rateLimited;
  }
  if (isRelayDeadlineError(text)) return NetworkFailure.timeout;
  const refusals = [
    'restricted:',
    'auth-required:',
    'blocked:',
    'invalid:',
    'pow:',
    'duplicate:',
    'unsupported:',
  ];
  return refusals.any(text.startsWith)
      ? NetworkFailure.relayRefused
      : NetworkFailure.relayError;
}
