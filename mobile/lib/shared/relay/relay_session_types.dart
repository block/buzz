import 'package:flutter/foundation.dart';

import 'relay_call_error.dart';
import 'relay_socket.dart';

enum SessionStatus { disconnected, connecting, connected, reconnecting }

typedef RelaySocketFactory =
    RelaySocket Function({
      required String wsUrl,
      required String? nsec,
      required void Function(List<dynamic> message) onMessage,
      required void Function() onConnected,
      required void Function(Object? error) onDisconnected,
    });

@immutable
class SessionState {
  final SessionStatus status;
  final int reconnectAttempt;

  const SessionState({required this.status, this.reconnectAttempt = 0});
}

/// Recovery lifecycle for a live relay subscription.
enum RelaySubscriptionStatus { ready, retrying }

/// A write was cancelled because the active community or identity changed
/// after it started. Callers that report failures to the user should treat
/// this differently from relay or network failures.
class ActiveCommunityChangedError extends StateError {
  ActiveCommunityChangedError(super.message);
}

/// A publish was attempted while the relay socket is not connected.
class RelayNotConnectedError extends StateError {
  RelayNotConnectedError() : super('Relay session is not connected');
}

/// The relay did not acknowledge a published EVENT in time, so it may or may
/// not have accepted it. Other relay timeouts (history, queries) happen before
/// anything is published and are plain [RelayTimeoutException]s.
class RelayAckTimeoutError extends RelayTimeoutException {
  RelayAckTimeoutError(String message, [Duration? duration])
    : super(RelayCall.publish, message, duration);
}

/// The connection ended after an EVENT was sent but before the relay
/// acknowledged it, so the relay may or may not have accepted it.
class RelayAckLostError implements Exception {
  RelayAckLostError([this.cause]);

  final Object? cause;

  @override
  String toString() =>
      'Connection lost before the relay acknowledged the event'
      '${cause == null ? '' : ': $cause'}';
}
