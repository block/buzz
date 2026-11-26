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

  /// Counts a connection completing AUTH, and the recovery it ends when an
  /// open connection was lost before it.
  void _noteConnected() {
    _noteNetwork(NetworkEndpoint.relayConnection, since: _connectStartedAt);
    final lost = _connectionLost;
    _connectionLost = null;
    // A loss in another community's connection, or before a move to the
    // background, is not this one's recovery.
    if (lost != null &&
        lost.context == _contextGeneration &&
        lost.pauses == _pauses) {
      _noteNetwork(
        NetworkEndpoint.relayRecovery,
        duration: _elapsed() - lost.at,
      );
    }
  }

  /// Counts the end of a connect attempt or of an open connection, once: a
  /// socket that fails while authenticating reports it twice.
  ///
  /// [denied] says the relay announced it was refusing the member, which
  /// outranks however the socket then ended.
  void _noteDisconnected(Object? error, {bool denied = false}) {
    final since = _connectStartedAt;
    if (since == null && _connectedAt == null) return;
    // Recovery runs from the last frame, since a dead socket can look open
    // for a while. Pongs never reach here, but one is due every ping
    // interval, so a quiet socket was alive until at most two intervals
    // before the loss. Failed attempts after the loss keep the first time.
    if (_connectedAt != null && _backgroundedAt == null) {
      final provenAlive = _elapsed() - RelaySocket.pingInterval * 2;
      final inbound = _lastFrameElapsed;
      _connectionLost ??= (
        at: inbound != null && inbound > provenAlive ? inbound : provenAlive,
        context: _contextGeneration,
        pauses: _pauses,
      );
    }
    _connectStartedAt = null;
    _noteNetwork(
      NetworkEndpoint.relayConnection,
      failure: denied ? NetworkFailure.authRefused : _connectionFailure(error),
      // How long a failed attempt took. A lost connection has no duration.
      since: _connectedAt == null ? since : null,
    );
  }

  /// Counts a connect attempt a protected relay refused at the upgrade, which
  /// the session retries once with a renewed assertion.
  void _noteUpgradeRefused() => _noteNetwork(
    NetworkEndpoint.relayConnection,
    failure: NetworkFailure.authRefused,
    since: _connectStartedAt,
  );

  /// Remembers the relay's terminal denial, which an expired hosted session
  /// also gets, so the close that follows is counted as a refusal. A bare
  /// NOTICE changes nothing else.
  void _noteAuthorizationDenied(List<dynamic> data) {
    if (data.length > 1 && data[1] == _authorizationDeniedNotice) {
      _authorizationDenied = true;
    }
  }

  /// Counts the queries and publishes that were sent or waiting when the
  /// socket dropped, once each: their timers and futures end with the socket.
  /// Closing the session on purpose never comes through here.
  void _noteLostOperations() {
    for (final subscription in _historySubscriptions.values) {
      if (subscription.completer.isCompleted) continue;
      _noteNetwork(
        NetworkEndpoint.relayQuery,
        failure: NetworkFailure.disconnected,
        since: subscription.startedAt,
      );
    }
    for (final pending in _pendingEvents.values) {
      if (pending.completer.isCompleted) continue;
      _noteNetwork(
        NetworkEndpoint.relayPublish,
        failure: NetworkFailure.disconnected,
        since: pending.startedAt,
      );
    }
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

/// The session's default clock for durations. Unlike the wall clock, a device
/// clock correction does not move it.
Duration _monotonic() => _uptime.elapsed;
final _uptime = Stopwatch()..start();

/// The relay's NOTICE for a refused or expired authorization, sent just before
/// it closes the connection with 1008.
const _authorizationDeniedNotice = 'restricted: authorization denied';

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
