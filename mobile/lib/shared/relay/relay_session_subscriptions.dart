part of 'relay_session.dart';

class _HistorySubscription {
  final List<NostrEvent> events = [];
  final Completer<List<NostrEvent>> completer;
  final Timer timeout;
  final DateTime startedAt;

  /// Where the call was made. Errors completed later, from a timer or a
  /// socket callback, carry it, so an escaped error's stack reaches the caller.
  final StackTrace callSite = StackTrace.current;

  void fail(Object error) => completer.completeError(error, callSite);

  void timedOut(Duration timeout) => fail(
    RelayTimeoutException(
      RelayCall.history,
      'Relay history request timed out after $timeout',
      timeout,
    ),
  );

  /// The filter of a request held until the socket authenticates.
  NostrFilter? heldFilter;

  _HistorySubscription({
    required this.completer,
    required this.timeout,
    required this.startedAt,
  });
}

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  final void Function(RelaySubscriptionStatus status)? onStatusChanged;

  /// Whether a replay narrows [filter] with a `since` from what this
  /// subscription last saw.
  final bool resumeSince;

  /// Where the call was made. Errors completed later, from a timer or a
  /// socket callback, carry it, so an escaped error's stack reaches the caller.
  final StackTrace callSite = StackTrace.current;

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
    this.resumeSince = true,
    this.readyCompleter,
  });

  /// The REQ filter for a replay, resumed from what this subscription saw.
  NostrFilter get replayFilter {
    if (!resumeSince) return filter;
    final since = relayResumeSince(
      originalSince: filter.since,
      until: filter.until,
      lastSeenCreatedAt: lastSeenCreatedAt,
      liveUntil: liveUntil,
    );
    return since == null || since == filter.since
        ? filter
        : filter.copyWithSince(since);
  }
}

class _ClosedRetry {
  final _LiveSubscription subscription;
  final int generation;
  _ClosedRetry({required this.subscription, required this.generation});
}

class _PendingEvent {
  final Completer<NostrEvent> completer;
  final Timer timeout;
  final DateTime startedAt;

  /// Where the call was made. Errors completed later, from a timer or a
  /// socket callback, carry it, so an escaped error's stack reaches the caller.
  final StackTrace callSite = StackTrace.current;

  void fail(Object error) => completer.completeError(error, callSite);

  void timedOut(String eventId, Duration timeout) => fail(
    RelayAckTimeoutError(
      'Event $eventId not acknowledged within $timeout',
      timeout,
    ),
  );

  _PendingEvent({
    required this.completer,
    required this.timeout,
    required this.startedAt,
  });
}

class _BufferedEvent {
  final String subId;
  final NostrEvent event;

  _BufferedEvent(this.subId, this.event);
}
