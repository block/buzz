part of 'channels_provider.dart';

extension _ChannelPushCache on ChannelsNotifier {
  Future<void> _exportPushCache(
    String? communityID,
    List<NostrEvent> metadata,
    List<NostrEvent> membership,
  ) async {
    if (defaultTargetPlatform != TargetPlatform.iOS || communityID == null) {
      return;
    }
    if (_pushCacheExporting) {
      // A refresh can contain different channels. Do not replace its predecessor
      // or retain another raw batch: refetch the complete scope after it drains.
      _pushCacheDirty = true;
      return;
    }
    _pushCacheExporting = true;
    try {
      await _pushExport.export(
        () => cacheBuzzPushChannelEvents(communityID, metadata, membership),
      );
      // A failed operation is terminal only for its own input. A refresh that
      // arrived meanwhile still owns an independent dirty obligation.
      while (_lifecycleRef.mounted && _pushCacheDirty) {
        _pushCacheDirty = false;
        final session = _lifecycleRef.read(relaySessionProvider.notifier);
        final community = _lifecycleRef.read(activeCommunityProvider).value?.id;
        final relay = _lifecycleRef.read(relayConfigProvider).baseUrl;
        final pubkey = _lifecycleRef.read(myPubkeyProvider);
        if (community == null || pubkey == null) return;
        bool current() =>
            _lifecycleRef.mounted &&
            _lifecycleRef.read(activeCommunityProvider).value?.id ==
                community &&
            _lifecycleRef.read(relayConfigProvider).baseUrl == relay &&
            _lifecycleRef.read(myPubkeyProvider) == pubkey &&
            identical(
              _lifecycleRef.read(relaySessionProvider.notifier),
              session,
            );

        void ensureCurrent() {
          if (!current()) throw const _StaleChannelRefresh();
        }

        // This fetch has no UI writes or channel-refresh generation changes.
        // New ordinary refreshes can proceed and mark this snapshot dirty again.
        await _pushExport.export(() async {
          try {
            if (!current()) return;
            final members = await _fetchChannelMemberships(
              session,
              pubkey,
              ensureCurrent: ensureCurrent,
            );
            if (!current()) return;
            final ids = members
                .map((event) => event.getTagValue('d'))
                .whereType<String>()
                .toSet()
                .toList();
            final memberMetadata = ids.isEmpty
                ? const <NostrEvent>[]
                : await session.fetchHistory(NostrFilters.channelMetadata(ids));
            if (!current()) return;
            final memberSnapshots = ids.isEmpty
                ? const <NostrEvent>[]
                : await session.fetchHistory(
                    NostrFilter(
                      kinds: const [39002],
                      tags: {'#d': ids},
                      limit: ids.length,
                    ),
                  );
            if (!current()) return;
            await cacheBuzzPushChannelEvents(community, memberMetadata, [
              ...members,
              ...memberSnapshots,
            ]);
          } catch (_) {
            if (current()) rethrow;
            // Retired fetches must neither write nor report an error in the
            // replacement scope. Its ordinary refresh owns any new dirty work.
          }
        });
        // Only new producer input can set dirty again; failure alone never
        // schedules another attempt after recovery's bounded retries.
      }
    } finally {
      _pushCacheExporting = false;
    }
  }
}
