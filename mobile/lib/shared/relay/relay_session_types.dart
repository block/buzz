import 'package:flutter/foundation.dart';

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
