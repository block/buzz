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

/// The relay answered an EVENT with `OK false`: it definitely did not apply it.
class RelayEventRejected implements Exception {
  const RelayEventRejected(this.message);

  final String message;

  @override
  String toString() => 'Exception: $message';
}

/// [RelaySessionNotifier.publish] had no connection to send on, so it sent
/// nothing.
class RelayNotConnected extends StateError {
  RelayNotConnected() : super('Relay session is not connected');
}

/// Recovery lifecycle for a live relay subscription.
enum RelaySubscriptionStatus { ready, retrying }
