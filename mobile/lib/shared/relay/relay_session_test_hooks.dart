part of 'relay_session.dart';

/// Test hooks into the session's private connection state.
extension RelaySessionTestHooks on RelaySessionNotifier {
  @visibleForTesting
  void debugAttachSocketForTest(RelaySocket socket) {
    _socket?.dispose();
    _socket = socket;
    _socketConnected = true;
  }

  @visibleForTesting
  void debugHandleDisconnected([Object? error]) {
    _socketConnected = false;
    _handleDisconnected(_connectionGeneration, error);
  }

  @visibleForTesting
  void debugResetClosedRetriesForDisconnect() {
    _socketConnected = false;
    _resetAllClosedRetries();
  }

  @visibleForTesting
  void debugSetSessionStatus(SessionStatus status) {
    _socketConnected = status == SessionStatus.connected;
  }

  @visibleForTesting
  void debugHandleMessage(List<dynamic> data) => _handleMessage(data);

  @visibleForTesting
  void debugFlushEventBuffer() => _flushEventBuffer();

  @visibleForTesting
  Future<void> debugHandleConnected() =>
      _handleConnected(_connectionGeneration);

  @visibleForTesting
  Future<void> debugReplayLiveSubscriptions() =>
      _replayLiveSubscriptions(_connectionGeneration);

  @visibleForTesting
  void debugDispose() => _dispose();

  @visibleForTesting
  void debugSupersedeConnection() => _connectionGeneration++;

  @visibleForTesting
  void debugPauseNow() => _pauseNow();

  @visibleForTesting
  void debugHandleSocketMessageForTest(List<dynamic> data) =>
      _handleMessage(data);
}
