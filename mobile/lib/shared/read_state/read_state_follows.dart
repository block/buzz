part of 'read_state_manager.dart';

/// Synced thread follow choices on `thread-follows:<slot>` coordinates.
extension ReadStateFollows on ReadStateManager {
  /// The merged follow choices of every slot, by `<channel>:<root>` key.
  Map<String, ThreadFollowChoice> get threadFollows {
    final merged = <String, ThreadFollowChoice>{};
    void add(String key, ThreadFollowChoice choice) {
      final current = merged[key];
      merged[key] = current == null
          ? choice
          : mergeThreadFollowChoices(current, choice);
    }

    for (final choices in _remoteFollows.values) {
      choices.forEach(add);
    }
    ThreadFollowsBlob(
      clientId: _clientId,
      entries: _ownFollows,
    ).choices.forEach(add);
    return merged;
  }

  /// Whether follow state may be missing entries: no complete load yet, or
  /// an unreadable follow coordinate.
  bool get threadFollowsIncomplete =>
      !_followsLoaded || _unreadableFollowSlots.isNotEmpty;

  /// Record an explicit follow or unfollow in this installation's own slot.
  /// Returns false, leaving state unchanged, when the slots are full.
  bool setThreadFollow(String channelId, String rootId, bool follow) {
    final key = threadFollowKey(channelId, rootId);
    if (_disposed || !isValidThreadFollowKey(key)) return false;
    final previous = _ownFollows[key];
    _ownFollows[key] = {...?previous, 'follow': follow, 'at': _now()};
    if (!_packFollows()) {
      if (previous == null) {
        _ownFollows.remove(key);
      } else {
        _ownFollows[key] = previous;
      }
      return false;
    }
    _persistLocalState();
    _onChanged();
    _schedulePublish();
    return true;
  }

  /// Import pre-sync local choices at `at = 0`, only for keys with no synced
  /// entry and only after a complete merge. Returns the keys now settled
  /// (imported, or already synced) so the caller can retire them.
  Set<String> importLegacyFollows(Map<String, bool> choices) {
    if (_disposed || threadFollowsIncomplete) return const {};
    final merged = threadFollows;
    final settled = <String>{};
    final added = <String>[];
    for (final MapEntry(:key, value: follow) in choices.entries) {
      if (!isValidThreadFollowKey(key)) continue;
      settled.add(key);
      if (merged.containsKey(key) || _ownFollows.containsKey(key)) continue;
      _ownFollows[key] = {'follow': follow, 'at': 0};
      added.add(key);
    }
    if (added.isEmpty) return settled;
    if (!_packFollows()) {
      added.forEach(_ownFollows.remove);
      return const {};
    }
    _persistLocalState();
    _onChanged();
    _schedulePublish();
    return settled;
  }

  /// Re-place own follow entries. Before spilling into more slots, drops own
  /// entries that are not the merged winner for their key.
  bool _packFollows() {
    List<Set<String>> pack() => packThreadFollowKeys(
      _ownFollows,
      clientId: _clientId,
      previous: _followPlacement,
      extra: _ownFollowExtra,
    );
    try {
      _followPlacement = pack();
      return true;
    } on StateError {
      final merged = threadFollows;
      final own = ThreadFollowsBlob(
        clientId: _clientId,
        entries: _ownFollows,
      ).choices;
      final dropped = {
        for (final MapEntry(:key, :value) in own.entries)
          if (merged[key] != value) key: _ownFollows[key]!,
      };
      dropped.keys.forEach(_ownFollows.remove);
      try {
        _followPlacement = pack();
        return true;
      } on StateError {
        _ownFollows.addAll(dropped);
        return false;
      }
    }
  }

  void _mergeFollowEvent(NostrEvent event, String slot) {
    final ThreadFollowsBlob? blob;
    try {
      blob = decodeThreadFollowsBlob(_crypto.decrypt(event.content));
    } catch (_) {
      _unreadableFollowSlots.add(slot);
      return;
    }
    if (blob == null) {
      _unreadableFollowSlots.add(slot);
      return;
    }
    _unreadableFollowSlots.remove(slot);
    if (_isPlausibleCreatedAt(event.createdAt)) {
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
    }
    final index = _followSlots.indexOf(slot);
    if (index >= 0 && blob.clientId != _clientId) {
      // Never replace another installation's coordinate.
      final fresh = generateReadStateSlotId();
      _followSlots[index] = fresh;
      _pending.remove('$threadFollowsDTagPrefix$slot');
      _followPublished.remove(slot);
    } else if (index >= 0) {
      // Recover own entries the journal lost; local edits stay newer.
      for (final MapEntry(:key, :value) in blob.entries.entries) {
        _ownFollows.putIfAbsent(key, () => value);
        if (!_followPlacement.any((keys) => keys.contains(key))) {
          while (_followPlacement.length <= index) {
            _followPlacement.add({});
          }
          _followPlacement[index].add(key);
        }
      }
      if (_ownFollowExtra.isEmpty) _ownFollowExtra = Map.of(blob.extra);
      _followPublished[slot] = blob.encode();
    }
    _remoteFollows[slot] = blob.choices;
  }

  /// The plaintext of each own follow slot as it should be published.
  Map<String, String> _plannedFollowSlots() {
    if (_ownFollows.isEmpty && _followSlots.isEmpty) return const {};
    while (_followSlots.length < _followPlacement.length) {
      _followSlots.add(generateReadStateSlotId());
    }
    return {
      for (var index = 0; index < _followPlacement.length; index++)
        _followSlots[index]: ThreadFollowsBlob(
          clientId: _clientId,
          entries: {
            for (final key in _followPlacement[index]) key: ?_ownFollows[key],
          },
          extra: _ownFollowExtra,
        ).encode(),
    };
  }

  /// Own follow coordinates may be published only after a complete merge
  /// and while none of them is unreadable.
  bool get _canPublishFollows =>
      _followsLoaded && !_followSlots.any(_unreadableFollowSlots.contains);
}
