part of 'read_state_manager.dart';

/// The relay refused a pending update for its timestamp, but the update is
/// not old by this device's clock, so the device clock is off from the
/// relay's. Signing a newer update would be refused the same way, so the
/// exact update is kept for retry.
class ReadStateClockSkewError implements Exception {
  const ReadStateClockSkewError();

  @override
  String toString() =>
      'ReadStateClockSkewError: the device clock differs from the relay clock';
}

/// The relay's accepted distance between an event's `created_at` and its own
/// clock (`MAX_TIMESTAMP_DRIFT_SECS` in the relay's ingest).
const readStateRelayTimestampWindowSeconds = 15 * 60;

/// Backoff between snapshot retries after a failed sync: doubling from
/// [_snapshotRetryBase], at most [_snapshotRetryMax].
const _snapshotRetryBase = Duration(seconds: 5);
const _snapshotRetryMax = Duration(minutes: 5);

/// Failed-sync retry, unpublished-state checks, refused-publish recovery
/// and acknowledgement.
extension ReadStateRetry on ReadStateManager {
  /// Retry now after a failed sync: the remote snapshot again if the last one
  /// failed, else any unpublished state. Does nothing while in sync.
  Future<void> retrySync() async {
    if (_disposed ||
        !_remoteEnabled ||
        !_initialized ||
        syncError == null ||
        !_isConnected()) {
      return;
    }
    if (!_lastSnapshotSucceeded) {
      _snapshotRetryTimer?.cancel();
      _snapshotRetryTimer = null;
      // A successful read hands any unpublished marks to publication.
      await _requestSync();
      return;
    }
    await flush();
  }

  void _scheduleSnapshotRetry() {
    if (_disposed || _snapshotRetryTimer != null) return;
    final seconds =
        _snapshotRetryBase.inSeconds << min(_snapshotRetryAttempt, 6);
    _snapshotRetryAttempt++;
    _snapshotRetryTimer = Timer(
      Duration(seconds: min(seconds, _snapshotRetryMax.inSeconds)),
      () {
        _snapshotRetryTimer = null;
        unawaited(retrySync());
      },
    );
  }

  bool _isTimestampRefusal(Exception error) =>
      error.toString().contains('timestamp too far from server time');

  /// Retires [pending] after the relay refused it for its timestamp. The
  /// outcome is known, the relay did not store it, so NIP-RS allows a
  /// strictly newer successor in place of exact retry. Its marks were planned
  /// from the journaled local state, which still holds them, so the next plan
  /// reads a fresh snapshot and sends a successor, journaled before it is
  /// sent, unless the relay already holds this update or an own update
  /// covering it. An update that is not old by this device's clock means the
  /// device clock is off, so it is kept for exact retry.
  Future<void> _recoverRefusedPending(String slot, NostrEvent pending) async {
    if (pending.createdAt >= _now() - readStateRelayTimestampWindowSeconds) {
      throw const ReadStateClockSkewError();
    }
    if (identical(_pending[slot], pending)) _pending.remove(slot);
    _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, pending.createdAt);
    await _persistLocalState();
  }

  void _acknowledge(String slot, NostrEvent event) {
    if (slot.startsWith(threadFollowsDTagPrefix)) {
      // An OK is evidence about this exact packet only. A later packet or a
      // newer head may already have replaced it.
      final index = _followSlots.indexOf(
        slot.substring(threadFollowsDTagPrefix.length),
      );
      if (_pending[slot]?.id != event.id || index < 0) return;
      final blob = decodeThreadFollowsBlob(_crypto.decrypt(event.content));
      if (blob == null) throw StateError('Invalid pending thread follow event');
      _applyOwnFollowHead(index, event, blob);
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
      return;
    }
    final unread = slot.startsWith(readStateUnreadDTagPrefix);
    final blob = decodeReadStateBlob(
      _crypto.decrypt(event.content),
      version: unread ? 2 : 1,
    );
    if (blob == null) throw StateError('Invalid pending read-state event');
    if (unread) {
      final head = _unreadHead;
      // A newer own head that lacks this packet is what the relay holds; the
      // successor is rebuilt above it from the saved intent.
      if (slot == _unreadCoordinate &&
          (head == null || !_outranks(head, event))) {
        _unreadPublished = Map.of(blob.contexts);
      } else if (!_primaryFits()) {
        // The overfull fallback keeps the head's groups; keep the packet
        // and fail visibly unless that successor still carries it.
        final plan = _planUnread();
        for (final MapEntry(:key, :value) in blob.contexts.entries) {
          if ((plan[key] ?? -1) < value) {
            throw StateError(readStateFullMessage);
          }
        }
      }
    } else {
      _published[slot] = Map.of(blob.contexts);
    }
    _pending.remove(slot);
    _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
  }

  /// [_acknowledge], except that a packet held for capacity stays pending
  /// and its error is returned, so the rest of the flush still goes out.
  StateError? _acknowledgeOrHold(String slot, NostrEvent event) {
    try {
      _acknowledge(slot, event);
    } on StateError catch (error) {
      if (error.message != readStateFullMessage) rethrow;
      return error;
    }
    return null;
  }

  bool _isPermanentReadStateRemoteError(Object error) {
    // Relay rejections come back as `Exception("<message>")` from the
    // websocket OK handler. Pattern-match on the message text since we no
    // longer have HTTP status codes.
    final msg = error.toString().toLowerCase();
    return msg.contains('unknown event kind') ||
        msg.contains('missing users:write') ||
        msg.contains('insufficient scope') ||
        msg.contains('restricted: unknown');
  }

  /// Whether a publish would send anything: a pending event to retry, or a
  /// slot whose packed contexts differ from what was last published. Mirrors
  /// the packing in [_publishOnce] without changing any state.
  bool _hasUnpublishedState() {
    if (_pending.isNotEmpty) return true;
    for (final MapEntry(key: slot, value: plaintext)
        in _plannedFollowSlots().entries) {
      if (_followPublished[slot] != plaintext) return true;
    }
    final List<Map<String, int>> placement;
    try {
      placement = _planSlots();
    } on StateError {
      // Let the real publish surface the capacity error.
      return true;
    }
    if (placement.length > _slots.length) return true;
    if (_unreadPublishable && !mapEquals(_unreadPublished, _planUnread())) {
      return true;
    }
    for (var index = 0; index < placement.length; index++) {
      final published = _published[_slots[index]] ?? const <String, int>{};
      if (!mapEquals(published, placement[index])) return true;
    }
    return false;
  }
}
