part of 'relay_session.dart';

extension _RelaySessionBackgroundWork on RelaySessionNotifier {
  /// Connects for work the app does while backgrounded, such as sending a
  /// notification reply, and waits up to [timeout] until the session can
  /// publish. Returns whether it can. A backgrounded app disconnects again
  /// after the usual grace period.
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
      if (_backgroundedAt != null) onAppPaused();
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
