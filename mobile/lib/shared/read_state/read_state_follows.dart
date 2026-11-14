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
      extras: [for (final slot in _followSlots) _followExtra[slot] ?? const {}],
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
    // Only the coordinate's newest head counts (created_at, then lowest id).
    final head = _followHeads[slot];
    if (head != null &&
        (event.createdAt < head.createdAt ||
            event.createdAt == head.createdAt &&
                event.id.compareTo(head.id) >= 0)) {
      return;
    }
    _followHeads[slot] = (createdAt: event.createdAt, id: event.id);
    ThreadFollowsBlob? blob;
    try {
      blob = decodeThreadFollowsBlob(_crypto.decrypt(event.content));
    } catch (_) {}
    if (blob == null) {
      _unreadableFollowSlots.add(slot);
      _remoteFollows.remove(slot);
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
      _followExtra.remove(slot);
    } else if (index >= 0) {
      // The relay's own head is authoritative. A pending event it already
      // supersedes was sent; a local entry survives only while its `at` is
      // strictly greater, since greater `at` wins the merge anyway.
      final coordinate = '$threadFollowsDTagPrefix$slot';
      if (_pending[coordinate] case final sent?
          when sent.createdAt <= event.createdAt) {
        _acknowledge(coordinate, sent);
      }
      final local = ThreadFollowsBlob(
        clientId: _clientId,
        entries: _ownFollows,
      ).choices;
      final relay = blob.choices;
      for (final MapEntry(:key, :value) in blob.entries.entries) {
        final (mine, theirs) = (local[key], relay[key]);
        if (mine == null || theirs != null && mine.at <= theirs.at) {
          _ownFollows[key] = value;
        }
        if (!_followPlacement.any((keys) => keys.contains(key))) {
          while (_followPlacement.length <= index) {
            _followPlacement.add({});
          }
          _followPlacement[index].add(key);
        }
      }
      _followExtra[slot] = Map.of(blob.extra);
      _followPublished[slot] = blob.encode();
      return;
    }
    _remoteFollows[slot] = blob.choices;
  }

  /// Peer follow heads and choices, cached in the journal for offline use.
  /// An unreadable head is not cached, so the next fetch decodes it again.
  Map<String, Object?> _followHeadsJson() => {
    for (final MapEntry(key: slot, value: head) in _followHeads.entries)
      if (!_unreadableFollowSlots.contains(slot))
        slot: {
          'createdAt': head.createdAt,
          'id': head.id,
          'threads': {
            for (final MapEntry(:key, :value)
                in (_remoteFollows[slot] ?? const {}).entries)
              key: {'follow': value.follow, 'at': value.at},
          },
        },
  };

  void _hydrateFollowHeads(Object? json) {
    for (final MapEntry(:key, :value) in ((json as Map?) ?? const {}).entries) {
      final slot = key as String;
      final head = value as Map;
      _followHeads[slot] = (
        createdAt: head['createdAt'] as int,
        id: head['id'] as String,
      );
      if (_followSlots.contains(slot)) continue;
      _remoteFollows[slot] = {
        for (final MapEntry(:key, :value) in (head['threads'] as Map).entries)
          key as String: (
            follow: (value as Map)['follow'] as bool,
            at: value['at'] as int,
          ),
      };
    }
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
          extra: _followExtra[_followSlots[index]] ?? const {},
        ).encode(),
    };
  }

  /// Own follow coordinates may be published only after a complete merge
  /// and while none of them is unreadable.
  bool get _canPublishFollows =>
      _followsLoaded && !_followSlots.any(_unreadableFollowSlots.contains);
}
