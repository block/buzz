part of 'relay_session.dart';

extension _RelaySessionEventAdmission on RelaySessionNotifier {
  void _handleEvent(List<dynamic> data) {
    if (data.length < 3 || data[1] is! String) return;
    final subId = data[1] as String;
    final connectionGeneration = _connectionGeneration;
    final contextGeneration = _contextGeneration;

    final historySub = _historySubscriptions[subId];
    if (historySub != null) {
      historySub.receivedEventFrames++;
      final encodedBytes = _relayEventEncodedByteUpperBound(data[2]);
      if (encodedBytes == null) return;
      if (!_reserveRelayAdmission(
        encodedBytes,
        historySub.pendingVerifications,
        historySub.pendingVerificationBytes,
      )) {
        _failHistoryVerification(
          subId,
          historySub,
          StateError('Relay event admission queue is full'),
          StackTrace.current,
        );
        return;
      }
      historySub.pendingVerifications++;
      historySub.pendingVerificationBytes += encodedBytes;
      final verification = _observeHistoryVerification(
        subId,
        historySub,
        _verifier.verify(data[2]),
        encodedBytes,
        connectionGeneration,
        contextGeneration,
      );
      historySub.verificationTail = historySub.verificationTail.then((_) async {
        final event = await verification;
        if (_historySubscriptions[subId] != historySub ||
            connectionGeneration != _connectionGeneration ||
            contextGeneration != _contextGeneration) {
          return;
        }
        historySub.pendingVerifications--;
        historySub.pendingVerificationBytes -= encodedBytes;
        if (event != null) historySub.events.add(event);
      });
      return;
    }

    final liveSub = _liveSubscriptions[subId];
    if (liveSub != null && liveSub.acceptsRelayFrames) {
      final verificationEpoch = liveSub.verificationEpoch;
      liveSub.receivedEventFrames++;
      final encodedBytes = _relayEventEncodedByteUpperBound(data[2]);
      if (encodedBytes == null) return;
      if (!_reserveRelayAdmission(
        encodedBytes,
        liveSub.pendingVerifications,
        liveSub.pendingVerificationBytes,
      )) {
        _failLiveVerification(
          subId,
          liveSub,
          StateError('Relay event admission queue is full'),
          StackTrace.current,
        );
        return;
      }
      liveSub.pendingVerifications++;
      liveSub.pendingVerificationBytes += encodedBytes;
      final verification = _observeLiveVerification(
        subId,
        liveSub,
        _verifier.verify(data[2]),
        encodedBytes,
        connectionGeneration,
        contextGeneration,
        verificationEpoch,
      );
      liveSub.verificationTail = liveSub.verificationTail.then((_) async {
        final event = await verification;
        if (_liveSubscriptions[subId] != liveSub ||
            connectionGeneration != _connectionGeneration ||
            contextGeneration != _contextGeneration ||
            verificationEpoch != liveSub.verificationEpoch) {
          return;
        }
        liveSub.pendingVerifications--;
        liveSub.pendingVerificationBytes -= encodedBytes;
        if (event == null) return;

        // Only a verified event can prove relay activity. Replay progress is
        // recorded later, after the event reaches the subscription callback.
        _resetClosedRetry(liveSub);
        liveSub.validEventFrames++;
        _eventBuffer.add(_BufferedEvent(subId, event));
        _scheduleFlush();
      });
    }
  }

  bool _reserveRelayAdmission(
    int encodedBytes,
    int subscriptionPending,
    int subscriptionPendingBytes,
  ) {
    if (subscriptionPending >= _maxPendingRelayEvents ||
        encodedBytes >
            _maxPendingRelayEventEncodedByteUpperBound -
                subscriptionPendingBytes ||
        _pendingRelayAdmissions >= _maxPendingRelayEvents ||
        encodedBytes >
            _maxPendingRelayEventEncodedByteUpperBound -
                _pendingRelayAdmissionBytes) {
      return false;
    }
    _pendingRelayAdmissions++;
    _pendingRelayAdmissionBytes += encodedBytes;
    return true;
  }

  void _releaseRelayAdmission(int encodedBytes) {
    if (_pendingRelayAdmissions == 0) return;
    _pendingRelayAdmissions--;
    _pendingRelayAdmissionBytes -= encodedBytes;
  }

  Future<NostrEvent?> _observeHistoryVerification(
    String subId,
    _HistorySubscription subscription,
    Future<NostrEvent?> verification,
    int encodedBytes,
    int connectionGeneration,
    int contextGeneration,
  ) => verification.then(
    (event) {
      _releaseRelayAdmission(encodedBytes);
      return event;
    },
    onError: (Object error, StackTrace stackTrace) {
      _releaseRelayAdmission(encodedBytes);
      if (_historySubscriptions[subId] == subscription &&
          connectionGeneration == _connectionGeneration &&
          contextGeneration == _contextGeneration) {
        _failHistoryVerification(subId, subscription, error, stackTrace);
      }
      return null;
    },
  );

  Future<NostrEvent?> _observeLiveVerification(
    String subId,
    _LiveSubscription subscription,
    Future<NostrEvent?> verification,
    int encodedBytes,
    int connectionGeneration,
    int contextGeneration,
    int verificationEpoch,
  ) => verification.then(
    (event) {
      _releaseRelayAdmission(encodedBytes);
      return event;
    },
    onError: (Object error, StackTrace stackTrace) {
      _releaseRelayAdmission(encodedBytes);
      if (_liveSubscriptions[subId] == subscription &&
          connectionGeneration == _connectionGeneration &&
          contextGeneration == _contextGeneration &&
          verificationEpoch == subscription.verificationEpoch) {
        _failLiveVerification(subId, subscription, error, stackTrace);
      }
      return null;
    },
  );

  void _handleEose(List<dynamic> data) {
    if (data.length < 2 || data[1] is! String) return;
    final subId = data[1] as String;

    final historySub = _historySubscriptions[subId];
    if (historySub != null) {
      if (historySub.pendingVerifications == 0) {
        _completeHistoryEose(subId, historySub);
      } else {
        unawaited(_completeHistoryEoseAfterVerification(subId, historySub));
      }
      return;
    }

    final liveSub = _liveSubscriptions[subId];
    if (liveSub != null && liveSub.acceptsRelayFrames) {
      final connectionGeneration = _connectionGeneration;
      final contextGeneration = _contextGeneration;
      final verificationEpoch = liveSub.verificationEpoch;
      if (liveSub.pendingVerifications == 0) {
        _completeLiveEose(
          subId,
          liveSub,
          connectionGeneration,
          contextGeneration,
          verificationEpoch,
        );
      } else {
        unawaited(
          _completeLiveEoseAfterVerification(
            subId,
            liveSub,
            connectionGeneration,
            contextGeneration,
            verificationEpoch,
          ),
        );
      }
    }
  }

  Future<void> _completeHistoryEoseAfterVerification(
    String subId,
    _HistorySubscription subscription,
  ) async {
    await subscription.verificationTail;
    _completeHistoryEose(subId, subscription);
  }

  void _completeHistoryEose(String subId, _HistorySubscription subscription) {
    if (_historySubscriptions[subId] != subscription) return;
    _historySubscriptions.remove(subId);
    subscription.timeout.cancel();
    if (!subscription.completer.isCompleted) {
      if (subscription.receivedEventFrames > 0 && subscription.events.isEmpty) {
        subscription.completer.completeError(
          const FormatException(
            'relay history returned no valid signed events',
          ),
        );
      } else {
        subscription.completer.complete(subscription.events);
      }
    }
    _sendClose(subId);
  }

  void _failHistoryVerification(
    String subId,
    _HistorySubscription subscription,
    Object error,
    StackTrace stackTrace,
  ) {
    if (_historySubscriptions[subId] != subscription) return;
    _historySubscriptions.remove(subId);
    subscription.timeout.cancel();
    if (!subscription.completer.isCompleted) {
      subscription.completer.completeError(error, stackTrace);
    }
    _sendClose(subId);
  }

  Future<void> _completeLiveEoseAfterVerification(
    String subId,
    _LiveSubscription subscription,
    int connectionGeneration,
    int contextGeneration,
    int verificationEpoch,
  ) async {
    await subscription.verificationTail;
    _completeLiveEose(
      subId,
      subscription,
      connectionGeneration,
      contextGeneration,
      verificationEpoch,
    );
  }

  void _completeLiveEose(
    String subId,
    _LiveSubscription subscription,
    int connectionGeneration,
    int contextGeneration,
    int verificationEpoch,
  ) {
    if (_liveSubscriptions[subId] != subscription ||
        connectionGeneration != _connectionGeneration ||
        contextGeneration != _contextGeneration ||
        verificationEpoch != subscription.verificationEpoch) {
      return;
    }
    if (subscription.receivedEventFrames > 0 &&
        subscription.validEventFrames == 0) {
      _failLiveVerification(
        subId,
        subscription,
        const FormatException(
          'relay subscription returned no valid signed events',
        ),
        StackTrace.current,
      );
      return;
    }

    subscription.receivedEventFrames = 0;
    subscription.validEventFrames = 0;
    // Flush replay callbacks before signaling ready. This ordering matters for
    // retry replays, whose original ready completer has already been released.
    _resetClosedRetry(subscription);
    _flushBufferedEventsNow();
    subscription.onStatusChanged?.call(RelaySubscriptionStatus.ready);
    final readyCompleter = subscription.readyCompleter;
    if (readyCompleter != null && !readyCompleter.isCompleted) {
      readyCompleter.complete();
      subscription.readyCompleter = null;
    }
  }

  void _failLiveVerification(
    String subId,
    _LiveSubscription subscription,
    Object error,
    StackTrace stackTrace,
  ) {
    if (_liveSubscriptions[subId] != subscription) return;
    final readyCompleter = subscription.readyCompleter;
    if (readyCompleter != null && !readyCompleter.isCompleted) {
      readyCompleter.completeError(error, stackTrace);
    } else {
      subscription.onClosed?.call('event verification failed: $error');
    }
    _removeLiveSubscription(subId, subscription);
    _sendClose(subId);
  }

  void _retireLiveVerification() {
    for (final entry in _liveSubscriptions.entries) {
      _retireLiveSubscriptionVerification(entry.key, entry.value);
    }
  }

  void _retireLiveSubscriptionVerification(
    String subId,
    _LiveSubscription subscription,
  ) {
    subscription.verificationEpoch++;
    subscription.acceptsRelayFrames = false;
    subscription.verificationTail = Future.value();
    subscription.pendingVerifications = 0;
    subscription.pendingVerificationBytes = 0;
    subscription.receivedEventFrames = 0;
    subscription.validEventFrames = 0;
    _eventBuffer.removeWhere((event) => event.subId == subId);
    if (_eventBuffer.isEmpty) {
      _flushTimer?.cancel();
      _flushTimer = null;
    }
  }
}
