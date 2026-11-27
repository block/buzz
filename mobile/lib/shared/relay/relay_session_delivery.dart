part of 'relay_session.dart';

extension _RelaySessionDelivery on RelaySessionNotifier {
  void _scheduleFlush() {
    _flushTimer ??= Timer(
      const Duration(milliseconds: RelaySessionNotifier._eventBatchMs),
      _flushEventBuffer,
    );
  }

  void _flushBufferedEventsNow() {
    _flushTimer?.cancel();
    _flushTimer = null;
    _flushEventBuffer();
  }

  void _flushEventBuffer() {
    _flushTimer = null;
    if (_eventBuffer.isEmpty) return;

    final batch = List<_BufferedEvent>.from(_eventBuffer);
    _eventBuffer.clear();

    for (final buffered in batch) {
      final sub = _liveSubscriptions[buffered.subId];
      if (sub == null) continue;

      // Deduplicate per subscription. The same relay event can legitimately
      // match multiple live subscriptions, e.g. the channel list unread listener
      // and the open channel message listener.
      final deliveryKey = '${buffered.subId}:${buffered.event.id}';
      if (_recentDeliveryKeys.contains(deliveryKey)) continue;

      // Cap the dedup set to prevent unbounded memory growth.
      if (_recentDeliveryKeys.length >=
          RelaySessionNotifier._maxRecentDeliveryKeys) {
        _recentDeliveryKeys.clear();
      }
      _recentDeliveryKeys.add(deliveryKey);

      // A subscriber that throws must not cost the rest of the batch: its
      // delivery keys are already recorded, so lost events never come back.
      try {
        sub.onEvent(buffered.event);
      } catch (error, stackTrace) {
        _diagnostics.reportError(
          error,
          stackTrace,
          operation: DiagnosticOperation.eventSubscriber,
        );
      }
    }
  }

  void _reportInvalidRequest(String message) {
    if (!isInvalidRequestRejection(message)) return;
    _diagnostics.reportError(
      RelayInvalidRequest(),
      StackTrace.current,
      operation: DiagnosticOperation.relayRejection,
    );
  }
}
