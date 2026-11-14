part of 'read_state_manager.dart';

/// Synced whole-channel override actions and their primary-slot plan.
extension ReadStateOverrides on ReadStateManager {
  /// Channels whose synced clear failed and still stands.
  Set<String> get failedClears =>
      Set.unmodifiable(_failedClears.intersection(activeChannelOverrides));

  /// Synchronized whole-channel mark unread (NIP-RS Actions). Returns false,
  /// leaving state unchanged, when the action must fail visibly: no complete
  /// load, an exhausted counter, or a primary that cannot fit the group.
  bool markChannelUnread(String channelId) {
    if (_disposed || !_hasFullState || !isChannelOverrideTarget(channelId)) {
      return false;
    }
    final previous = _overrides[channelId];
    final register = previous ?? (s: 0, c: 0, b: 0);
    final top = max(register.s, register.c);
    if (top == 4294967295) return false;
    // The group travels with its frontier; a channel never read keeps the
    // retention floor as both.
    final previousFrontier = _effectiveState[channelId];
    final frontier = max(previousFrontier ?? 0, readStateFloor(_now()));
    _effectiveState[channelId] = frontier;
    _overrides[channelId] = (s: top + 1, c: register.c, b: frontier);
    if (!_primaryFits()) {
      if (previous == null) {
        _overrides.remove(channelId);
      } else {
        _overrides[channelId] = previous;
      }
      if (previousFrontier == null) {
        _effectiveState.remove(channelId);
      } else {
        _effectiveState[channelId] = previousFrontier;
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

  /// Ends a live channel override with a clear-counter increment, even when
  /// no frontier moved. Returns null when no live override needs a clear,
  /// and false, setting [syncError], when the counter is exhausted or the
  /// primary cannot carry the clear.
  bool? _clearChannelOverride(String channelId) {
    final register = _overrides[channelId];
    if (register == null ||
        !_hasFullState ||
        !isOverrideActive(register, _frontierOf(channelId))) {
      return null;
    }
    final top = max(register.s, register.c);
    if (top < 4294967295) {
      _overrides[channelId] = (s: register.s, c: top + 1, b: register.b);
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

  /// The primary's override groups and their co-located frontiers.
  Map<String, int> _primaryOverrides() => {
    ...canonicalOverrideEntries(_overrides, _frontierOf),
    for (final channelId in _overrides.keys)
      overrideFrontierKey(channelId): _frontierOf(channelId),
  };

  bool _primaryFits() {
    try {
      _packSlots(_primaryOverrides());
      return true;
    } on StateError {
      return false;
    }
  }

  List<Map<String, int>> _packSlots(Map<String, int> primary) =>
      packReadStateContexts(
        _publishableLocalContexts(),
        clientId: _clientId,
        previous: _placement,
        primary: primary,
      );

  /// The slot plan. When the merged groups overfill the primary, it keeps
  /// the last published groups so frontiers still publish; only override
  /// actions fail ([_primaryFits]).
  List<Map<String, int>> _planSlots() {
    try {
      return _packSlots(_primaryOverrides());
    } on StateError {
      // Canonical against current frontiers, so it can only shrink.
      final last = channelOverrideRegisters(
        _published[_slots.first] ?? const {},
      );
      return _packSlots({
        ...canonicalOverrideEntries(last, _frontierOf),
        for (final channelId in last.keys)
          overrideFrontierKey(channelId): _frontierOf(channelId),
      });
    }
  }
}
