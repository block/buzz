part of 'relay_session.dart';

extension _RelaySessionBackgroundWork on RelaySessionNotifier {
  /// Reconnects on returning to the foreground, unless the socket stayed open
  /// through a short absence or for background work.
  void _resumeFromBackground(SessionStatus status, Ref ref) {
    _paused = false;
    final backgroundedAt = _backgroundedAt;
    _backgroundedAt = null;
    _backgroundGraceTimer?.cancel();
    _backgroundGraceTimer = null;

    final backgroundedLongEnoughToRequireReconnect =
        backgroundedAt != null &&
        _now().difference(backgroundedAt) >=
            RelaySessionNotifier._backgroundGraceDuration;
    // Keep a socket held open for background work: an OK may still come.
    final heldOpen = _backgroundWorkHolds > 0 && _socketConnected;
    if ((heldOpen || !backgroundedLongEnoughToRequireReconnect) &&
        status == SessionStatus.connected) {
      return;
    }
    // A return from a brief inactive spell (Control Center) keeps an attempt
    // a network change just started, or its spread-out wait.
    if (backgroundedAt == null &&
        (_liveness.networkAttempt || _liveness.spreadReconnect)) {
      return;
    }

    // Cancel any in-flight reconnect backoff timer so we reconnect immediately
    // instead of waiting for the (possibly large) exponential delay. This
    // attempt replaces any a network change started.
    _endNetworkReconnect();
    _reconnectTimer?.cancel();
    _reconnectDelayMs = RelaySessionNotifier._baseReconnectDelayMs;
    final config = ref.read(relayConfigProvider);
    _connect(config);
  }

  /// Keeps the background grace period from disconnecting the session while
  /// the app does work in the background, such as sending a notification
  /// reply. Returns a callback that ends the hold; it also ends by itself
  /// after [maxHold]. When the last hold ends in the background, the usual
  /// grace period starts again.
  ///
  /// A grace disconnect already waiting on its before-pause callbacks checks
  /// for holds before it disconnects, so it can't cut off work that started
  /// after it began.
  void Function() _holdForBackgroundWork(Duration maxHold) {
    _backgroundWorkHolds++;
    var released = false;
    late final Timer limit;
    void release() {
      if (released) return;
      released = true;
      limit.cancel();
      _backgroundWorkHolds--;
      if (_backgroundWorkHolds == 0 && _backgroundedAt != null && !_paused) {
        onAppPaused();
      }
    }

    limit = Timer(maxHold, release);
    return release;
  }

  /// Connects for background work and waits up to [timeout] until the session
  /// can publish. Returns whether it can. Hold the session with
  /// [_holdForBackgroundWork] first, or the grace period may disconnect it.
  Future<bool> _connectForBackgroundWork(
    Duration timeout,
    SessionStatus status,
    Ref ref,
  ) async {
    if (_socketConnected) return true;
    if (_paused ||
        _reconnectTimer?.isActive == true ||
        status == SessionStatus.disconnected) {
      _paused = false;
      _endNetworkReconnect();
      _reconnectTimer?.cancel();
      _reconnectDelayMs = RelaySessionNotifier._baseReconnectDelayMs;
      unawaited(_connect(ref.read(relayConfigProvider)));
    }
    var timedOut = false;
    await Future.doWhile(() async {
      if (_socketConnected || _disposed || timedOut) return false;
      await Future<void>.delayed(const Duration(milliseconds: 100));
      return true;
    }).timeout(timeout, onTimeout: () => timedOut = true);
    return _socketConnected;
  }
}
