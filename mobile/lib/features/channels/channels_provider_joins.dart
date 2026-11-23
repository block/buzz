part of 'channels_provider.dart';

/// Read-your-writes for this client's own joins.
///
/// The relay confirms a join on its writer pool before its read replicas see
/// the new roster. A list read from a replica that still lacks it would
/// install a nonmember snapshot and take the composer away again, so while a
/// confirmed join is missing from a replica read, the read is repeated
/// against the writer pool, whose answer is authoritative either way.
extension ChannelsNotifierJoins on ChannelsNotifier {
  /// Records [channelId] as a join the writer pool has confirmed for this
  /// user, then reads the list again (from the writer pool while replicas
  /// lack the join).
  ///
  /// Returns whether the new list has the channel as a member. Read failures
  /// propagate; the join stays recorded, so a later read still shows it.
  Future<bool> installConfirmedJoin(String channelId) async {
    _confirmedJoins.add(channelId);
    // A rebuild retires a read in flight, and a read that starts it retires
    // itself once. Retry past both; a further retirement propagates like any
    // other read failure.
    for (var attempt = 1; ; attempt++) {
      try {
        final channels = await fetchFresh();
        return channels.any(
          (channel) => channel.id == channelId && channel.isMember,
        );
      } on _StaleChannelRefresh {
        if (attempt == 3) rethrow;
      }
    }
  }

  /// Records [channelId] as a membership this client itself ended (a
  /// confirmed leave or delete), so the next list read doesn't treat its
  /// absence as a loss that needs a writer read. The saved-page purge has
  /// already been recorded.
  void noteConfirmedRemoval(String channelId) {
    _confirmedJoins.remove(channelId);
    _confirmedRemovals.add(channelId);
  }

  void _forgetConfirmedMemberships() {
    _confirmedJoins.clear();
    _confirmedRemovals.clear();
  }

  /// The current user's latest kind:39002 rosters for a list read.
  ///
  /// A writer-pool answer also settles each recorded join: one it lacks was
  /// removed since, so it is forgotten rather than hidden. A replica answer
  /// that has every recorded join shows the replicas caught up.
  Future<List<NostrEvent>> _readMemberships(
    _ChannelRefreshFence fence,
    RelaySessionNotifier session,
    String pubkey,
  ) async {
    final scope = _lifecycleRef.read(savedMessageScopeProvider);
    final removedHere = {..._confirmedRemovals};
    _confirmedRemovals.clear();
    final previouslyJoined = <String>{
      if (scope != null && _hasLoaded)
        for (final channel in _listState.value ?? const <Channel>[])
          if (channel.isMember) channel.id,
      if (scope != null)
        for (final entry in _memberSnapshotsByChannelId.entries)
          if (entry.value.any((member) => member.pubkey == pubkey)) entry.key,
    }..removeAll(removedHere);
    Future<List<NostrEvent>> read(bool strong) async => _latestChannelEvents(
      await _fenced(
        fence,
        _fetchChannelMemberships(session, pubkey, strong: strong),
      ),
      kind: 39002,
    );
    Set<String> ids(List<NostrEvent> rosters) => rosters
        .map((event) => event.getTagValue('d'))
        .whereType<String>()
        .toSet();

    final memberships = await read(false);
    final listed = ids(memberships);
    if (listed.containsAll(_confirmedJoins) &&
        listed.containsAll(previouslyJoined)) {
      _confirmedJoins.clear();
      return memberships;
    }
    // A stale replica can omit a recent join or recovery. Only a writer read
    // can turn the disappearance of a known membership into a disk purge.
    final fromWriter = await read(true);
    final confirmed = ids(fromWriter);
    _confirmedJoins.retainAll(confirmed);
    fence.ensureCurrent();
    if (scope != null &&
        scope == _lifecycleRef.read(savedMessageScopeProvider)) {
      final removed = previouslyJoined.difference(confirmed);
      if (removed.isNotEmpty) {
        final cache = _lifecycleRef.read(savedMessageStoreProvider);
        final purges = [
          for (final channelId in removed)
            cache.removeChannel(scope, channelId),
        ];
        for (final channelId in removed) {
          invalidateSavedMessagePreviews(_lifecycleRef, channelId);
        }
        // Keep the old membership on disk until its cache purge runs.
        await _fenced(fence, Future.wait(purges));
      }
    }
    return fromWriter;
  }
}
