part of 'relay_session.dart';

/// How long a liveness probe waits for the relay to answer before the socket
/// is treated as dead.
const relayLivenessProbeTimeout = Duration(seconds: 5);

/// The probe in flight on the current connection, if any.
class _LivenessProbe {
  String? subId;
  Timer? timer;
}

/// Notices a socket that looks open but no longer delivers anything, as one
/// left on the previous network after a Wi-Fi or cellular change. Without
/// this, only the WebSocket ping notices it, which can take a minute.
extension _RelaySessionLiveness on RelaySessionNotifier {
  /// Asks the relay for an immediate answer and reconnects if none arrives in
  /// [relayLivenessProbeTimeout]. Any frame proves the socket, so a busy
  /// connection passes even when the answer itself is queued behind others.
  /// A session that isn't connected reconnects now instead of waiting out
  /// its backoff, since the network just changed.
  ///
  /// Runs on a network change and when a request over the socket times out.
  void _checkConnection(SessionStatus status, Ref ref) {
    if (_disposed || _paused || _ageRestricted) return;
    if (!_socketConnected || status != SessionStatus.connected) {
      if (status == SessionStatus.reconnecting && _connectStartedAt == null) {
        _reconnectTimer?.cancel();
        _reconnectDelayMs = RelaySessionNotifier._baseReconnectDelayMs;
        _connect(ref.read(relayConfigProvider));
      }
      return;
    }
    if (_liveness.subId != null) return;
    final generation = _connectionGeneration;
    final sentAt = _now();
    final subId = _nextSubId('p');
    _liveness.subId = subId;
    // `limit: 0` reads nothing stored, so the relay answers EOSE at once.
    _socket?.send([
      'REQ',
      subId,
      {
        'ids': [_nothingId],
        'limit': 0,
      },
    ]);
    _liveness.timer = _reconnectTimerFactory(relayLivenessProbeTimeout, () {
      if (_liveness.subId != subId) return;
      _liveness.subId = null;
      _liveness.timer = null;
      if (!_isActiveConnection(generation) || !_socketConnected) return;
      final inbound = _lastInboundAt;
      if (inbound != null && !inbound.isBefore(sentAt)) {
        _sendClose(subId);
        return;
      }
      _handleDisconnected(
        generation,
        TimeoutException(
          'Relay did not answer within $relayLivenessProbeTimeout',
        ),
      );
      // Reconnect now, as on resume, rather than after the backoff delay.
      _reconnectTimer?.cancel();
      _connect(ref.read(relayConfigProvider));
    });
  }

  /// Whether [subId] is the probe's, which an EOSE or CLOSED answers.
  bool _answerLivenessProbe(String subId) {
    if (subId != _liveness.subId) return false;
    _liveness.timer?.cancel();
    _liveness.timer = null;
    _liveness.subId = null;
    _sendClose(subId);
    return true;
  }
}

/// An event id nothing can have.
const _nothingId =
    '0000000000000000000000000000000000000000000000000000000000000000';
