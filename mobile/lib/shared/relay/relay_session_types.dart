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

  @override
  bool operator ==(Object other) =>
      other is SessionState &&
      other.status == status &&
      other.reconnectAttempt == reconnectAttempt;

  @override
  int get hashCode => Object.hash(status, reconnectAttempt);

  @override
  String toString() => 'SessionState($status, attempt: $reconnectAttempt)';
}

/// Recovery lifecycle for a live relay subscription.
enum RelaySubscriptionStatus { ready, retrying }
