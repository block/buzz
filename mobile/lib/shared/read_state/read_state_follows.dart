part of 'read_state_manager.dart';

/// The failure shown for a follow or unfollow refused before
/// [ReadStateFollows.threadFollowsReady].
const threadFollowsNotReadyMessage =
    'Follow choices are not ready. Connect and try again.';

/// The failure shown when a follow choice would overfill the account's
/// full read-state snapshot.
const threadFollowsFullMessage =
    'Follow choices are full for this account. Unfollow a thread and try again.';

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

  /// Whether follow and unfollow taps are accepted: this writer has
  /// durably saved a complete recovery of its own state and every owned follow head is
  /// readable. Each new writer starts not ready, whatever its journal holds.
  bool get threadFollowsReady => _followsRecovered && _canPublishFollows;

  /// Record an explicit follow or unfollow in this installation's own slot.
  /// The choice shows at once but is accepted only once journaled: resolves
  /// false, leaving the previous choice, before [threadFollowsReady], when the
  /// slots are full, no later timestamp can be reserved, or the journal write
  /// fails.
  Future<bool> setThreadFollow(
    String channelId,
    String rootId,
    bool follow,
  ) async {
    final key = threadFollowKey(channelId, rootId);
    if (_disposed || !isValidThreadFollowKey(key)) return false;
    if (!threadFollowsReady) {
      syncError = StateError(threadFollowsNotReadyMessage);
      _onChanged();
      return false;
    }
    final previous = _ownFollows[key];
    final previousEdit = _followEdits[key];
    final previousReserved = _lastReserved;
    void revert() {
      if (previous == null) {
        _ownFollows.remove(key);
      } else {
        _ownFollows[key] = previous;
      }
      if (previousEdit == null) {
        _followEdits.remove(key);
      } else {
        _followEdits[key] = previousEdit;
      }
    }

    final int generation;
    try {
      generation = _reserve();
    } on StateError catch (error) {
      syncError = error;
      _onChanged();
      return false;
    }
    final previousSlots = List.of(_followSlots);
    final previousPlacement = _followPlacement;
    _ownFollows[key] = {...?previous, 'follow': follow, 'at': _now()};
    _followEdits[key] = generation;
    if (!_packFollows()) {
      revert();
      _lastReserved = previousReserved;
      return false;
    }
    // Refused where the user sees it, not later in publication.
    if (!_followsFitSnapshot()) {
      revert();
      _lastReserved = previousReserved;
      _followSlots
        ..clear()
        ..addAll(previousSlots);
      _followPlacement = previousPlacement;
      syncError = StateError(threadFollowsFullMessage);
      _onChanged();
      return false;
    }
    _onChanged();
    try {
      await _persistLocalState();
    } catch (error) {
      // Undo only this tap; a newer tap on the key owns it now.
      if (_followEdits[key] == generation) revert();
      syncError = error;
      if (!_disposed) _onChanged();
      return false;
    }
    if (_disposed) return false;
    _schedulePublish();
    return true;
  }

  /// Import pre-sync local choices at `at = 0`, only for keys with no synced
  /// entry and only after a complete merge. Returns the keys now settled
  /// (imported, or already synced) so the caller can retire them.
  Future<Set<String>> importLegacyFollows(Map<String, bool> choices) async {
    if (_disposed || threadFollowsIncomplete) return const {};
    final merged = threadFollows;
    final candidates = <String>{};
    final imported = <String, int>{};
    final previousReserved = _lastReserved;
    void revert() {
      for (final MapEntry(:key, value: generation) in imported.entries) {
        // Undo only imports no newer tap has replaced.
        if (_followEdits[key] != generation) continue;
        _ownFollows.remove(key);
        _followEdits.remove(key);
      }
    }

    for (final MapEntry(:key, value: follow) in choices.entries) {
      if (!isValidThreadFollowKey(key)) continue;
      candidates.add(key);
      if (merged.containsKey(key) || _ownFollows.containsKey(key)) continue;
      final int generation;
      try {
        generation = _reserve();
      } on StateError catch (error) {
        revert();
        _lastReserved = previousReserved;
        syncError = error;
        return const {};
      }
      _ownFollows[key] = {'follow': follow, 'at': 0};
      _followEdits[key] = imported[key] = generation;
    }
    if (imported.isNotEmpty && !_packFollows()) {
      revert();
      _lastReserved = previousReserved;
      return const {};
    }
    if (imported.isNotEmpty) _onChanged();
    // A key is settled, so the caller may retire its legacy source, only if
    // the successful journal write this import awaited covered it, and the
    // entry it wrote is still the current one: never from the live map alone,
    // which can hold a later write's tentative entry, nor from a write whose
    // tentative entry was since reverted.
    final Map<String, int> written;
    try {
      written = await _persistFollowKeys();
    } catch (error) {
      revert();
      syncError = error;
      if (!_disposed) _onChanged();
      return const {};
    }
    if (_disposed) return const {};
    if (imported.isNotEmpty) _schedulePublish();
    final present = threadFollows;
    return {
      for (final key in candidates)
        if (present.containsKey(key) &&
            written[key] == (_followEdits[key] ?? 0))
          key,
    };
  }

  /// Whether every changed follow slot, at its largest timestamp, still fits
  /// the last full snapshot alongside the retained packets.
  bool _followsFitSnapshot() {
    final writes = [
      ..._pending.values,
      for (final MapEntry(key: slot, value: plaintext)
          in _plannedFollowSlots().entries)
        if (_followPublished[slot] != plaintext)
          NostrEvent(
            id: '0' * 64,
            pubkey: pubkey,
            sig: '0' * 128,
            kind: EventKind.readState,
            createdAt: 4294967295,
            tags: [
              ['d', '$threadFollowsDTagPrefix$slot'],
              ['t', threadFollowsTag],
            ],
            content: _crypto.encrypt(plaintext),
          ),
    ];
    try {
      _checkSnapshotCapacity(_capacitySnapshot, writes);
      return true;
    } on StateError {
      return false;
    }
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
    // Owned slot IDs exist before an edit placed in them is journaled, so a
    // restored journal recognizes its own heads.
    bool placed(List<Set<String>> placement) {
      _followPlacement = placement;
      while (_followSlots.length < placement.length) {
        _followSlots.add(generateReadStateSlotId());
      }
      return true;
    }

    try {
      return placed(pack());
    } on StateError {
      final merged = threadFollows;
      final own = ThreadFollowsBlob(
        clientId: _clientId,
        entries: _ownFollows,
      ).choices;
      final dropped = {
        for (final MapEntry(:key, :value) in own.entries)
          // Pending edits, captured or not, and explicit unfollows are never
          // dropped to make room.
          if (merged[key] != value &&
              value.follow &&
              !_followEdits.containsKey(key))
            key: _ownFollows[key]!,
      };
      dropped.keys.forEach(_ownFollows.remove);
      try {
        return placed(pack());
      } on StateError {
        _ownFollows.addAll(dropped);
        return false;
      }
    }
  }

  /// Forgets cached peer follow coordinates a complete snapshot no longer
  /// has: their owner deleted them. [before] is the heads as the read
  /// started; a head a live event changed since then is newer and stays.
  /// Our own coordinates and their pending intent are never touched.
  void _dropAbsentPeerFollows(
    Map<String, ({int createdAt, String id})> before,
    List<NostrEvent> snapshot,
  ) {
    final present = {for (final event in snapshot) ?threadFollowsSlotOf(event)};
    for (final slot in _followHeads.keys.toList()) {
      if (_followSlots.contains(slot) ||
          present.contains(slot) ||
          _followHeads[slot] != before[slot]) {
        continue;
      }
      _followHeads.remove(slot);
      _remoteFollows.remove(slot);
      _unreadableFollowSlots.remove(slot);
    }
  }

  void _mergeFollowEvent(NostrEvent event, String slot) {
    final index = _followSlots.indexOf(slot);
    // Only the coordinate's newest head counts (created_at, then lowest id),
    // but an older echo of a retained packet still acknowledges that packet.
    final head = _followHeads[slot];
    if (head != null && !_outranksFollowHead(event, head)) {
      final coordinate = '$threadFollowsDTagPrefix$slot';
      if (index >= 0 && _pending[coordinate]?.id == event.id) {
        _resolveFollowPacket(coordinate, event);
      }
      return;
    }
    ThreadFollowsBlob? blob;
    try {
      blob = decodeThreadFollowsBlob(_crypto.decrypt(event.content));
    } catch (_) {}
    if (blob == null) {
      _followHeads[slot] = (createdAt: event.createdAt, id: event.id);
      _unreadableFollowSlots.add(slot);
      _remoteFollows.remove(slot);
      return;
    }
    _unreadableFollowSlots.remove(slot);
    if (_isPlausibleCreatedAt(event.createdAt)) {
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
    }
    if (index >= 0 && blob.clientId != _clientId) {
      // Never replace another installation's coordinate. Pending edits stay
      // and are re-placed on the fresh coordinate.
      _followHeads[slot] = (createdAt: event.createdAt, id: event.id);
      final fresh = generateReadStateSlotId();
      _followSlots[index] = fresh;
      _pending.remove('$threadFollowsDTagPrefix$slot');
      _followCaptured.remove('$threadFollowsDTagPrefix$slot');
      _followPublished.remove(slot);
      _followExtra.remove(slot);
      _remoteFollows[slot] = blob.choices;
      return;
    }
    if (index >= 0) {
      _applyOwnFollowHead(index, event, blob);
      return;
    }
    _followHeads[slot] = (createdAt: event.createdAt, id: event.id);
    _remoteFollows[slot] = blob.choices;
  }

  static bool _outranksFollowHead(
    NostrEvent event,
    ({int createdAt, String id}) head,
  ) =>
      event.createdAt > head.createdAt ||
      event.createdAt == head.createdAt &&
          event.id.toLowerCase().compareTo(head.id.toLowerCase()) < 0;

  /// Resolve the retained packet [event] by its exact ID: only the edit
  /// generations it captured, and only while they are still current.
  void _resolveFollowPacket(String coordinate, NostrEvent event) {
    if (_pending[coordinate]?.id != event.id) return;
    // A newer unreadable head may not contain this packet's edits: hold them.
    if (_unreadableFollowSlots.contains(
      coordinate.substring(threadFollowsDTagPrefix.length),
    )) {
      return;
    }
    final captured = _followCaptured.remove(coordinate) ?? const {};
    for (final MapEntry(:key, value: generation) in captured.entries) {
      if (_followEdits[key] == generation) _followEdits.remove(key);
    }
    _pending.remove(coordinate);
  }

  /// A readable own head that outranks the accepted one (or the exact
  /// retained packet, after the relay accepted it) becomes the accepted head.
  /// Edits on this coordinate reserved at or below it are covered by it,
  /// since every full-slot packet carries all earlier pending edits; later
  /// edits stay pending. The whole payload is adopted otherwise.
  void _applyOwnFollowHead(
    int index,
    NostrEvent event,
    ThreadFollowsBlob blob,
  ) {
    final slot = _followSlots[index];
    final coordinate = '$threadFollowsDTagPrefix$slot';
    _resolveFollowPacket(coordinate, event);
    final head = _followHeads[slot];
    if (head != null && !_outranksFollowHead(event, head)) return;
    _followHeads[slot] = (createdAt: event.createdAt, id: event.id);
    // Every later reservation must outrank the accepted head, even one dated
    // past the plausibility window.
    _lastReserved = max(_lastReserved, event.createdAt);
    while (_followPlacement.length <= index) {
      _followPlacement.add({});
    }
    final placed = _followPlacement[index];
    _followEdits.removeWhere(
      (key, generation) =>
          generation <= event.createdAt && placed.contains(key),
    );
    for (final key in placed.toList()) {
      if (_followEdits.containsKey(key) || blob.entries.containsKey(key)) {
        continue;
      }
      _ownFollows.remove(key);
      placed.remove(key);
    }
    for (final MapEntry(:key, :value) in blob.entries.entries) {
      if (!_followEdits.containsKey(key)) _ownFollows[key] = value;
      if (!_followPlacement.any((keys) => keys.contains(key))) {
        placed.add(key);
      }
    }
    // A retained packet the new head outranks was superseded, not accepted.
    final pending = _pending[coordinate];
    if (pending != null &&
        (pending.id == event.id ||
            !_outranksFollowHead(pending, (
              createdAt: event.createdAt,
              id: event.id,
            )))) {
      _pending.remove(coordinate);
      _followCaptured.remove(coordinate);
    }
    _followExtra[slot] = Map.of(blob.extra);
    _followPublished[slot] = blob.encode();
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
