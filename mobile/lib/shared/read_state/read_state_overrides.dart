part of 'read_state_manager.dart';

/// Synced whole-channel override actions and the version-2 unread record
/// that carries them (NIP-RS, Expiring unread state).
extension ReadStateOverrides on ReadStateManager {
  /// Channels whose synced clear failed and still stands.
  Set<String> get failedClears =>
      Set.unmodifiable(_failedClears.intersection(activeChannelOverrides));

  /// The expiry clock: the local clock, never behind any time this
  /// installation already used, so a rollback cannot revive a mark.
  int get _expiryNow => _expiryWatermark = max(_expiryWatermark, _now());

  String get _unreadCoordinate => '$readStateUnreadDTagPrefix$_unreadSlot';

  bool _isLive(String channelId, OverrideRegister register) =>
      isOverrideActive(register, _frontierOf(channelId), now: _expiryNow);

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
  /// clear failed and [syncError] says so.
  bool clearChannelOverride(String channelId) {
    if (_disposed) return false;
    final cleared = _clearChannelOverride(channelId);
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
  bool? _clearChannelOverride(String channelId) {
    final register = _overrides[channelId];
    if (register == null || !_hasFullState || !_isLive(channelId, register)) {
      return null;
    }
    if (_allocateStamp(register) case final stamp?) {
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

  /// Merges received or journaled version-2 groups. An expired replay is
  /// dropped before its baseline can enter a register; a group stamped too
  /// far ahead is deferred until the clock reaches it.
  void _mergeOverrides(Map<String, int> contexts) {
    for (final MapEntry(key: channelId, value: register)
        in channelOverrideRegisters(contexts).entries) {
      _mergeRegister(channelId, register);
    }
  }

  void _mergeRegister(String channelId, OverrideRegister register) {
    if (isOverrideExpired(register, now: _expiryNow)) return;
    final target = isOverrideDeferred(register, now: _now())
        ? _deferredOverrides
        : _overrides;
    final current = target[channelId];
    target[channelId] = current == null
        ? register
        : mergeOverrideRegisters(current, register);
  }

  /// Merges the deferred groups the clock has reached. Returns whether none
  /// remain: until then the override view is incomplete.
  bool _resolveDeferred() {
    if (_deferredOverrides.isEmpty) return true;
    final waiting = Map.of(_deferredOverrides);
    _deferredOverrides.clear();
    waiting.forEach(_mergeRegister);
    return _deferredOverrides.isEmpty;
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
  void _mergeUnreadRecord(String slot, ReadStateBlob blob) {
    if (slot == _unreadSlot) {
      if (blob.clientId == _clientId) {
        _unreadPublished = Map.of(blob.contexts);
      } else {
        _pending.remove(_unreadCoordinate);
        _unreadSlot = generateReadStateSlotId();
        _unreadPublished = const {};
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
