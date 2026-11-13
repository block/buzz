part of 'relay_session.dart';

/// Test hooks into the session's private connection state.
extension RelaySessionTestHooks on RelaySessionNotifier {
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
}
