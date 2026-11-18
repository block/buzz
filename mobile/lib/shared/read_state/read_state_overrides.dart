part of 'read_state_manager.dart';

/// Synced whole-channel override actions and the version-2 unread record
/// that carries them (NIP-RS, Expiring unread state).
extension ReadStateOverrides on ReadStateManager {
  /// The channels whose synced manual unread is live.
  Set<String> get activeChannelOverrides => {
    for (final MapEntry(key: channelId, value: register) in _overrides.entries)
      if (_isLive(channelId, register)) channelId,
  };

  /// Channels whose synced clear failed and still stands.
  Set<String> get failedClears =>
      Set.unmodifiable(_failedClears.intersection(activeChannelOverrides));

  /// The expiry clock: the local clock, never behind any time this
  /// installation already used, so a rollback cannot revive a mark.
  int get _expiryNow => _expiryWatermark = max(_expiryWatermark, _now());

  String get _unreadCoordinate => '$readStateUnreadDTagPrefix$_unreadSlot';

  bool _isLive(String channelId, OverrideRegister register) {
    final frontier = _frontierOf(channelId);
    final live = isOverrideActive(register, frontier, now: _expiryNow);
    // An expiry seen here must survive a crash and a clock rollback.
    if (!live &&
        isOverrideActive(register, frontier, now: _savedExpiryWatermark)) {
      _persistLocalState();
    }
    return live;
  }

  /// A fresh action stamp above [register], or null when it would pass
  /// uint32 or run more than a minute ahead of the local clock.
  int? _allocateStamp(OverrideRegister register) =>
      allocateOverrideStamp(register, now: _now());

  /// Synchronized whole-channel mark unread (NIP-RS Actions). Returns false,
  /// leaving state unchanged, when the action must fail visibly: no complete
  /// load, no stamp to allocate, or a record that cannot fit the group.
  bool markChannelUnread(String channelId) {
    if (_disposed || !_hasFullState || !isChannelOverrideTarget(channelId)) {
      return false;
    }
    final previous = _overrides[channelId];
    final register = previous ?? (s: 0, c: 0, b: 0);
    final stamp = _allocateStamp(register);
    if (stamp == null) return false;
    // Baseline is the stored frontier, never the moving retention floor.
    _overrides[channelId] = (
      s: stamp,
      c: register.c,
      b: _frontierOf(channelId),
    );
    if (!_primaryFits()) {
      if (previous == null) {
        _overrides.remove(channelId);
      } else {
        _overrides[channelId] = previous;
      }
      return false;
    }
    _persistLocalState();
    _onChanged();
    _schedulePublish();
    return true;
  }

  /// Ends a live channel override without moving its frontier: reaching the
  /// bottom of a channel with nothing new to read. Returns false when the
  /// clear failed and [syncError] says so. A [reading] clear refused only
  /// because the view is incomplete is retried after the next usable
  /// snapshot, if the register is still the one the reader saw.
  bool clearChannelOverride(String channelId, {bool reading = false}) {
    if (_disposed) return false;
    final cleared = _clearChannelOverride(channelId, reading: reading);
    if (cleared != true) return cleared == null;
    _persistLocalState();
    _onChanged();
    _schedulePublish();
    return true;
  }

  /// Ends a live channel override with a fresh clear stamp, even when no
  /// frontier moved. Returns null when no live override needs a clear, and
  /// false, setting [syncError], when no stamp can be allocated or the
  /// record cannot carry the clear.
  bool? _clearChannelOverride(String channelId, {bool reading = false}) {
    // Resolve held-back groups first: one may be this channel's.
    final complete = _hasFullState;
    final register = _overrides[channelId];
    if (register == null || !_isLive(channelId, register)) return null;
    if (reading && !complete) _refusedReadingClears[channelId] = register;
    // An incomplete view refuses the clear, visibly.
    final stamp = complete ? _allocateStamp(register) : null;
    if (stamp != null) {
      _overrides[channelId] = (s: register.s, c: stamp, b: register.b);
      if (_primaryFits()) {
        _failedClears.remove(channelId);
        return true;
      }
      _overrides[channelId] = register;
    }
    _failedClears.add(channelId);
    syncError = _standingError;
    _onChanged();
    return false;
  }

  /// Retries each refused reading clear once the view is complete, after a
  /// whole snapshot merged. Only a register still live and exactly as the
  /// reader saw it is cleared: a newer or different mark stays. A failure
  /// here is reported as any clear's is, and not retried again. A publisher
  /// shut off for good keeps the refusal standing: the clear could not sync.
  void _retryRefusedReadingClears() {
    if (_disposed ||
        _remoteUnsupported ||
        _refusedReadingClears.isEmpty ||
        !_hasFullState) {
      return;
    }
    final refused = Map.of(_refusedReadingClears);
    _refusedReadingClears.clear();
    var cleared = false;
    for (final MapEntry(key: channelId, value: seen) in refused.entries) {
      if (_overrides[channelId] != seen) continue;
      if (_clearChannelOverride(channelId) == true) cleared = true;
    }
    if (!cleared) return;
    syncError = _standingError;
    _persistLocalState();
    _onChanged();
    _schedulePublish();
  }

  /// Merges received version-2 groups. An expired replay is dropped before
  /// its baseline can enter a register; a group stamped too far ahead is
  /// deferred until the clock reaches it.
  void _mergeOverrides(Map<String, int> contexts) {
    for (final MapEntry(key: channelId, value: register)
        in channelOverrideRegisters(contexts).entries) {
      _mergeRegister(channelId, register);
    }
  }

  /// Restores journaled groups. Local state keeps a dead group until the
  /// discard guard passes, unlike an incoming replay.
  void _hydrateOverrides(Map<String, OverrideRegister> registers) =>
      registers.forEach(
        (channelId, register) =>
            _mergeRegister(channelId, register, local: true),
      );

  void _mergeRegister(
    String channelId,
    OverrideRegister register, {
    bool local = false,
  }) {
    final now = _expiryNow;
    if (local
        ? isOverrideDiscardable(register, now: now)
        : isOverrideExpired(register, now: now)) {
      return;
    }
    if (isOverrideDeferred(register, now: _now())) {
      final held = (channelId: channelId, register: register, local: local);
      // A repeated delivery holds nothing new.
      if (!_deferredOverrides.contains(held)) _deferredOverrides.add(held);
      return;
    }
    final current = _overrides[channelId];
    _overrides[channelId] = current == null
        ? register
        : mergeOverrideRegisters(current, register);
  }

  /// Merges the deferred groups the clock has reached. Returns whether none
  /// remain: until then the override view is incomplete.
  bool _resolveDeferred() {
    if (_deferredOverrides.isEmpty) return true;
    final waiting = List.of(_deferredOverrides);
    _deferredOverrides.clear();
    // Each contribution meets its own rule: an incoming one the replay
    // cutoff, before its baseline can merge; accepted local state the guard.
    for (final held in waiting) {
      _mergeRegister(held.channelId, held.register, local: held.local);
    }
    if (_deferredOverrides.length == waiting.length) return false;
    _persistLocalState();
    _onChanged();
    _schedulePublish();
    return _deferredOverrides.isEmpty;
  }

  /// Clock upkeep, on the display tick and on resume: merges held-back
  /// groups the clock has reached and saves any expiry the tick shows.
  void maintainOverrideClock() {
    if (_disposed) return;
    _resolveDeferred();
    activeChannelOverrides;
  }

  /// Whether an unread candidate may be planned: held-back groups the clock
  /// has reached merge first, so the save before sending covers them.
  bool get _unreadPublishable => _resolveDeferred();

  /// Whether the candidate planned as [planned] may still be sent. Changes
  /// nothing: anything held back, or admitted since planning, refuses it,
  /// and the next pass saves and rebuilds, so an older record never
  /// replaces the only carrier of a mark the journal does not cover.
  bool _unreadStillCurrent(Map<String, int> planned) {
    if (_deferredOverrides.isEmpty && mapEquals(_planUnread(), planned)) {
      return true;
    }
    _schedulePublish();
    return false;
  }

  /// Drops dead groups past the discard guard, with their ceilings.
  void _discardOverrides() {
    final now = _expiryNow;
    _overrides.removeWhere(
      (_, register) => isOverrideDiscardable(register, now: now),
    );
  }

  /// The record's canonical groups and their paired frontiers, 0 included.
  Map<String, int> _unreadEntries(Map<String, OverrideRegister> registers) {
    final now = _expiryNow;
    final kept = {
      for (final MapEntry(:key, :value) in registers.entries)
        if (!isOverrideDiscardable(value, now: now)) key: value,
    };
    return {
      ...canonicalOverrideEntries(kept, _frontierOf, now: now),
      for (final channelId in kept.keys)
        overrideFrontierKey(channelId): _frontierOf(channelId),
    };
  }

  Map<String, int> _primaryOverrides() => _unreadEntries(_overrides);

  bool _unreadFits(Map<String, int> contexts) =>
      contexts.length <= 10000 &&
      utf8
              .encode(
                jsonEncode(
                  ReadStateBlob(
                    clientId: _clientId,
                    contexts: contexts,
                    version: 2,
                  ).toJson(),
                ),
              )
              .length <=
          readStatePlaintextBytes;

  bool _primaryFits() => _unreadFits(_primaryOverrides());

  /// The unread record to publish. When the merged groups overfill it, it
  /// keeps the last published groups, canonical and expired against the
  /// current clock and frontiers, so it can only shrink; only override
  /// actions fail ([_primaryFits]).
  Map<String, int> _planUnread() {
    final merged = _primaryOverrides();
    if (_unreadFits(merged)) return merged;
    return _unreadEntries(channelOverrideRegisters(_unreadPublished));
  }

  /// Takes in one decoded unread record. A foreign record at this
  /// installation's coordinate moves it to a fresh slot.
  void _mergeUnreadRecord(String slot, ReadStateBlob blob, NostrEvent event) {
    if (slot == _unreadSlot) {
      if (blob.clientId == _clientId) {
        // Our own head raises the reservation floor whatever its clock, so
        // the next version can replace it (the peer filter doesn't apply).
        _lastReserved = max(_lastReserved, event.createdAt);
        final head = _unreadHead;
        // A superseded own packet, such as a late echo, is not what the
        // relay holds.
        if (head == null || !_outranks(head, event)) {
          _unreadHead = event;
          _unreadPublished = Map.of(blob.contexts);
        }
      } else {
        _pending.remove(_unreadCoordinate);
        _unreadSlot = generateReadStateSlotId();
        _unreadPublished = const {};
        _unreadHead = null;
      }
    }
    _mergeOverrides(blob.contexts);
    for (final MapEntry(:key, :value) in blob.contexts.entries) {
      if (isOverrideContext(key)) continue;
      if (_applyRemoteContextTimestamp(contextId: key, timestamp: value) ==
          _ApplyRemoteContextResult.advanced) {
        _pendingSyncedAdvances.add(key);
      }
    }
  }
}

/// Replaceable-event order: the later `created_at`, then the lower id.
bool _outranks(NostrEvent a, NostrEvent b) =>
    a.createdAt > b.createdAt ||
    (a.createdAt == b.createdAt && a.id.compareTo(b.id) < 0);
