part of 'relay_session.dart';

/// Publishes durable sends over the authenticated session.
extension RelaySessionOutbox on RelaySessionNotifier {
  /// Publishes a durable send after the outbox commits its attempt marker.
  Future<NostrEvent> publishQueued(
    NostrEvent event, {
    required Future<bool> Function() beforeWire,
    required void Function() validateScope,
  }) => _publish(
    event,
    timeout: const Duration(seconds: 8),
    mayPublish: beforeWire,
    validatePublish: validateScope,
  );

  Future<NostrEvent> _publish(
    NostrEvent event, {
    required Duration timeout,
    Future<bool> Function()? mayPublish,
    void Function()? validatePublish,
  }) async {
    final generation = _connectionGeneration;
    if (_rateLimitGate.isActive) await _rateLimitGate.wait();
    bool connected() => _isActiveConnection(generation) && _socketConnected;
    if (!connected()) throw RelayNotConnectedError();
    if ((mayPublish != null && !await mayPublish()) || !connected()) {
      throw StateError('Event no longer wanted');
    }
    validatePublish?.call();

    final completer = Completer<NostrEvent>();
    final timer = Timer(timeout, () {
      final pending = _pendingEvents.remove(event.id);
      if (pending != null && !pending.completer.isCompleted) {
        _noteNetwork(
          NetworkEndpoint.relayPublish,
          failure: NetworkFailure.timeout,
          since: pending.startedAt,
        );
        pending.completer.completeError(
          RelayAckTimeoutError(
            'Event ${event.id} not acknowledged within $timeout',
          ),
        );
      }
    });

    _pendingEvents[event.id] = _PendingEvent(
      completer: completer,
      timeout: timer,
      startedAt: _now(),
      queuedEvent: validatePublish == null ? null : event,
    );

    _socket?.send(['EVENT', event.toJson()]);
    return completer.future;
  }
}

extension _RelaySessionDelivery on RelaySessionNotifier {
  void _confirmQueuedEcho(NostrEvent event) {
    final queued = _pendingEvents[event.id]?.queuedEvent;
    if (queued != null &&
        queued.pubkey == event.pubkey &&
        queued.sig == event.sig) {
      _handleOk(['OK', event.id, true, '']);
    }
  }

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
