part of 'relay_session.dart';

class _HistorySubscription {
  final List<NostrEvent> events = [];
  final Completer<List<NostrEvent>> completer;
  final Timer timeout;

  /// The filter of a request held until the socket authenticates.
  NostrFilter? heldFilter;

  _HistorySubscription({required this.completer, required this.timeout});
}

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  final void Function(RelaySubscriptionStatus status)? onStatusChanged;
  Completer<void>? readyCompleter;
  int? lastSeenCreatedAt;

  /// Whether EOSE arrived on the current connection and no CLOSED has
  /// interrupted the stream since.
  bool liveOnConnection = false;

  /// Client clock (seconds) when the last connection this subscription was
  /// live on closed. Kept across later connections that never reach EOSE.
  int? liveUntil;

  /// The connection generation of this subscription's latest REQ, and the
  /// one on which it sent a second REQ, leaving its EOSE ambiguous.
  int? reqGeneration;
  int? ambiguousGeneration;
  int closedRetryAttempt = 0;
  Timer? closedRetryTimer;

  _LiveSubscription({
    required this.filter,
    required this.onEvent,
    this.onClosed,
    this.onStatusChanged,
    this.readyCompleter,
  });
}

class _ClosedRetry {
  final _LiveSubscription subscription;
  final int generation;
  _ClosedRetry({required this.subscription, required this.generation});
}

class _PendingEvent {
  final Completer<NostrEvent> completer;
  final Timer timeout;

  _PendingEvent({required this.completer, required this.timeout});
}

class _BufferedEvent {
  final String subId;
  final NostrEvent event;

  _BufferedEvent(this.subId, this.event);
}
