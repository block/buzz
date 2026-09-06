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

  /// Non-null when the session stopped for a reason no amount of waiting will
  /// fix — currently a rejected NIP-42 AUTH. The session schedules no
  /// reconnect in this case, so consumers blocked on "wait for connected"
  /// must fail instead of hanging forever.
  final Object? terminalError;

  const SessionState({
    required this.status,
    this.reconnectAttempt = 0,
    this.terminalError,
  });

  bool get isTerminal => terminalError != null;
}

/// Recovery lifecycle for a live relay subscription.
enum RelaySubscriptionStatus { ready, retrying }
