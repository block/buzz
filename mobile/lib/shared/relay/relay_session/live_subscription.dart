part of '../relay_session.dart';

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  final bool Function(NostrEvent)? admitEvent;
  final bool replayFromWatermark;
  Completer<void>? readyCompleter;
  int? lastSeenCreatedAt;
  int closedRetryAttempt = 0;
  Timer? closedRetryTimer;

  _LiveSubscription({
    required this.filter,
    required this.onEvent,
    this.onClosed,
    this.admitEvent,
    this.replayFromWatermark = true,
    this.readyCompleter,
  });
}
