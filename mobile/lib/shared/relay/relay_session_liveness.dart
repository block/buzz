part of 'relay_session.dart';

/// How long a liveness probe waits for the relay to answer before the socket
/// is treated as dead.
const relayLivenessProbeTimeout = Duration(seconds: 5);

/// The longest a dead socket waits to reconnect after a network change. Each
/// client picks a uniform delay up to this, so phones that lose a shared
/// network together do not all reconnect in the same instant.
const relayNetworkChangeReconnectSpread = Duration(seconds: 3);

/// The probe in flight on the current connection, if any.
class _LivenessProbe {
  String? subId;
  Timer? timer;

  /// A probe found the socket dead after a network change and spread out its
  /// reconnect. A later change leaves that wait and the attempt it starts
  /// alone, so phones do not bunch up again when the network comes back.
  bool spreadReconnect = false;

  /// A network change restarted the connection, and that attempt is still
  /// in flight. A return from a brief inactive spell keeps it.
  bool networkAttempt = false;
}

/// Notices a socket that looks open but no longer delivers anything, as one
/// left on the previous network after a Wi-Fi or cellular change. Without
/// this, only the WebSocket ping notices it, which can take a minute.
extension _RelaySessionLiveness on RelaySessionNotifier {
  /// Asks the relay for an immediate answer and reconnects if none arrives in
  /// [relayLivenessProbeTimeout]. Any frame proves the socket, so a busy
  /// connection passes even when the answer itself is queued behind others.
  ///
  /// Runs on a network change ([networkChanged]) and when a request over the
  /// socket times out. After a network change the old address may be gone, so
  /// a session that is not connected restarts its connection at once, as on
  /// resume, and a dead socket reconnects within
  /// [relayNetworkChangeReconnectSpread]. After a timeout the relay may be
  /// overloaded, so a dead socket reconnects after the usual jittered backoff.
  void _checkConnection(
    SessionStatus status,
    Ref ref, {
    bool networkChanged = false,
  }) {
    if (_disposed || _paused || _ageRestricted) return;
    if (!_socketConnected || status != SessionStatus.connected) {
      // An attempt in flight may be stuck on the old network: replace it,
      // unless an earlier change spread out the reconnect.
      if (networkChanged && !_liveness.spreadReconnect) {
        _reconnectTimer?.cancel();
        _reconnectDelayMs = RelaySessionNotifier._baseReconnectDelayMs;
        _liveness.networkAttempt = true;
        _connect(ref.read(relayConfigProvider));
      }
      return;
    }
    final pending = _liveness.subId;
    if (pending != null) {
      // A frame from before a network change proves nothing after it.
      if (!networkChanged) return;
      _sendClose(pending);
      _resetLiveness();
    }
    final generation = _connectionGeneration;
    final sentAt = _now();
    final subId = _nextSubId('p');
    _liveness.subId = subId;
    // A membership-list filter skips the relay's channel access lookup, and
    // `limit: 0` with an id nothing has reads nothing stored, so the relay
    // answers at once even when its database is busy.
    _socket?.send([
      'REQ',
      subId,
      {
        'kinds': [_membershipListKind],
        'ids': [_nothingId],
        'limit': 0,
      },
    ]);
    _liveness.timer = _reconnectTimerFactory(relayLivenessProbeTimeout, () {
      if (_liveness.subId != subId) return;
      _resetLiveness();
      if (!_isActiveConnection(generation) || !_socketConnected) return;
      final inbound = _lastInboundAt;
      if (inbound != null && !inbound.isBefore(sentAt)) {
        _sendClose(subId);
        return;
      }
      final dead = _socket;
      _socket = null;
      _handleDisconnected(
        generation,
        TimeoutException(
          'Relay did not answer within $relayLivenessProbeTimeout',
        ),
      );
      // Nothing the dead socket still delivers belongs to a connection.
      _connectionGeneration++;
      dead?.dispose();
      if (!networkChanged || _disposed || _paused) return;
      _reconnectTimer?.cancel();
      final spread = relayNetworkChangeReconnectSpread.inMilliseconds;
      _liveness.spreadReconnect = true;
      _reconnectTimer = _reconnectTimerFactory(
        Duration(milliseconds: (_random.nextDouble() * spread).round()),
        () {
          _liveness.networkAttempt = true;
          _connect(ref.read(relayConfigProvider));
        },
      );
    });
  }

  /// Whether [subId] is the probe's, which its EOSE or CLOSED answers.
  bool _answerLivenessProbe(String subId, {bool closed = false}) {
    if (subId != _liveness.subId) return false;
    _resetLiveness();
    if (!closed) _sendClose(subId);
    return true;
  }

  /// Ends what a network change started, once its attempt connected or
  /// failed, or the session paused or ended.
  void _endNetworkReconnect() {
    _liveness.spreadReconnect = false;
    _liveness.networkAttempt = false;
  }

  /// Forgets the probe, as when its connection ends.
  void _resetLiveness() {
    _liveness.timer?.cancel();
    _liveness.timer = null;
    _liveness.subId = null;
  }
}

/// The relay's NIP-43 membership list, which it answers without reading the
/// member's channel access.
const _membershipListKind = 13534;

/// An event id nothing can have.
const _nothingId =
    '0000000000000000000000000000000000000000000000000000000000000000';
