part of 'read_state_manager.dart';

/// The local journal: this installation's durable read-state copy.
extension ReadStateJournal on ReadStateManager {
  void _hydrateFromLocalStorage() {
    final journal = _storage.readJournal(_journalKey);
    Map<String, int> contexts(Object? value) => (value as Map).map(
      (key, value) => MapEntry(key as String, value as int),
    );
    if (journal != null) {
      // An existing journal, even empty, means this is not a first landing.
      _initialSeedComplete = true;
      _clientId = journal['clientId'] as String;
      _effectiveState.addAll(contexts(journal['observed']));
      _localContexts.addAll(contexts(journal['local']));
      _mergeOverrides({
        ...contexts(journal['observed']),
        ...contexts(journal['overrides']),
      });
      _followSlots.addAll(
        ((journal['followSlots'] as List?) ?? const []).cast<String>(),
      );
      _followPlacement = [
        for (final keys in (journal['followPlacement'] as List?) ?? const [])
          (keys as List).cast<String>().toSet(),
      ];
      for (final entry
          in ((journal['ownFollows'] as Map?) ?? const {}).entries) {
        _ownFollows[entry.key as String] = Map<String, Object?>.from(
          entry.value as Map,
        );
      }
      for (final entry
          in ((journal['followExtra'] as Map?) ?? const {}).entries) {
        _followExtra[entry.key as String] = Map.from(entry.value as Map);
      }
      _hydrateFollowHeads(journal['followHeads']);
      for (final entry
          in ((journal['followEdits'] as Map?) ?? const {}).entries) {
        _followEdits[entry.key as String] = entry.value as int;
      }
      for (final entry
          in ((journal['followCaptured'] as Map?) ?? const {}).entries) {
        _followCaptured[entry.key as String] = (entry.value as Map).map(
          (key, value) => MapEntry(key as String, value as int),
        );
      }
      for (final entry
          in ((journal['followPublished'] as Map?) ?? const {}).entries) {
        _followPublished[entry.key as String] = entry.value as String;
      }
      _slots
        ..clear()
        ..addAll((journal['slots'] as List).cast<String>());
      if (_slots.isEmpty) {
        throw const FormatException('Empty read-state slot list');
      }
      _slotId = _slots.first;
      _placement = (journal['placement'] as List).map(contexts).toList();
      for (final entry in (journal['published'] as Map).entries) {
        _published[entry.key as String] = contexts(entry.value);
      }
      // Follow events have their own field, which older builds ignore.
      for (final entry in {
        ...journal['pending'] as Map,
        ...?journal['followPending'] as Map?,
      }.entries) {
        _pending[entry.key as String] = NostrEvent.fromJson(
          Map<String, dynamic>.from(entry.value as Map),
        );
      }
      _maxFetchedCreatedAt = journal['createdAt'] as int;
      // Never below anything already reserved, even if the journal's own
      // counter is older than a retained edit or packet.
      _lastReserved = [
        (journal['reserved'] as int?) ?? 0,
        ..._followEdits.values,
        for (final event in _pending.values) event.createdAt,
        for (final slot in _followSlots) ?_followHeads[slot]?.createdAt,
      ].reduce(max);
    } else {
      final stored = _storage.read(pubkey, relayUrl);
      _effectiveState.addEntries(
        stored.contexts.entries.where((entry) => !isOverrideContext(entry.key)),
      );
      // Legacy storage did not preserve provenance. Carry its existing
      // publishable values once; subsequent peer merges never enter this map.
      _localContexts.addEntries(
        stored.contexts.entries.where(
          (entry) =>
              stored.publishableContextIds.contains(entry.key) &&
              !isOverrideContext(entry.key) &&
              !entry.key.startsWith(msgContextPrefix),
        ),
      );
      _mergeOverrides({
        for (final entry in stored.contexts.entries)
          if (!isOverrideContext(entry.key) ||
              stored.publishableContextIds.contains(entry.key))
            entry.key: entry.value,
      });
    }
    _persistLocalState();
  }

  /// Saves like [_persistLocalState], resolving with the follow keys the
  /// write it awaited actually covered; see [_savedFollows].
  Future<Map<String, int>> _persistFollowKeys() {
    _persistLocalState();
    return _savedFollows;
  }

  Future<void> _persistLocalState() {
    if (_saveScheduled) return _saved;
    _saveScheduled = true;
    Future<Map<String, int>> save() async {
      _saveScheduled = false;
      if (!_ownsJournal) {
        throw StateError('Read-state journal writer retired');
      }
      final observed = pruneStaleContexts(
        _effectiveState,
        nowUnixSeconds: _now(),
      );
      // Every retained register keeps its frontier, an explicit 0 included.
      for (final channelId in _overrides.keys) {
        observed[overrideFrontierKey(channelId)] = _frontierOf(channelId);
      }
      final encoded = jsonEncode({
        'v': 1,
        'clientId': _clientId,
        'observed': observed,
        'local': _localContexts,
        'overrides': canonicalOverrideEntries(_overrides, _frontierOf),
        'followSlots': _followSlots,
        'followPlacement': [
          for (final keys in _followPlacement) keys.toList()..sort(),
        ],
        'ownFollows': _ownFollows,
        'followExtra': _followExtra,
        'followHeads': _followHeadsJson(),
        'followPublished': _followPublished,
        'followEdits': _followEdits,
        'followCaptured': _followCaptured,
        'reserved': _lastReserved,
        'slots': _slots,
        'placement': _placement,
        'published': _published,
        for (final follows in [false, true])
          follows ? 'followPending' : 'pending': {
            for (final MapEntry(:key, :value) in _pending.entries)
              if (key.startsWith(threadFollowsDTagPrefix) == follows)
                key: value.toJson(),
          },
        'createdAt': _maxFetchedCreatedAt,
      });
      final follows = {
        for (final key in threadFollows.keys) key: _followEdits[key] ?? 0,
      };
      await _storage.writeJournal(_journalKey, encoded, owner: _journalOwner);
      return follows;
    }

    // Coalesce synchronous batches without allowing a later write to race an
    // earlier write. A sender awaits this entire chain before network I/O.
    _savedFollows = _saved.then(
      (_) => save(),
      onError: (Object _, StackTrace _) => save(),
    );
    _saved = _savedFollows.then((_) {});
    unawaited(
      _saved.catchError((Object error) {
        syncError = error;
        debugPrint('[ReadStateManager] journal save failed: $error');
      }),
    );
    return _saved;
  }
}
