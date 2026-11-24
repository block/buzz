part of 'channels_provider.dart';

extension on ChannelsNotifier {
  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request. The relay preserves NIP-01 per-filter limits while
  /// executing the filters with bounded concurrency, avoiding an unbounded
  /// burst of websocket REQs on communities with many channels.
  Future<List<NostrEvent>?> _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<ChannelData> channels,
    Set<String> failedHistory,
  ) async {
    if (channels.isEmpty) return const [];

    final filters = [
      for (final channel in channels)
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          tags: {
            '#h': [channel.id],
          },
          limit: channel.channelType == 'dm' ? 1 : 20,
        ),
    ];

    return _fetchChannelHistoryBatch(
      session,
      filters,
      operation: 'latest-message query',
      failedChannelIds: failedHistory,
    );
  }

  void _cacheFilterEvidence(
    List<NostrEvent> events,
    List<ChannelData> requestedHistory,
    Set<String> failedHistory,
    Map<String, int> initialRevisions,
  ) {
    final evidence = <String, ({NostrEvent? event})>{
      for (final channel in requestedHistory)
        if (!failedHistory.contains(channel.id)) channel.id: (event: null),
    };
    for (final event in events) {
      final id = event.channelId;
      if (id == null ||
          !evidence.containsKey(id) ||
          !EventKind.channelMessageEventKinds.contains(event.kind)) {
        continue;
      }
      if (event.createdAt > (evidence[id]?.event?.createdAt ?? -1)) {
        evidence[id] = (event: event);
      }
    }
    for (final entry in evidence.entries) {
      if (activityRevisionFor(entry.key) != initialRevisions[entry.key]) {
        continue;
      }
      _filterEvidence[entry.key] = entry.value;
    }
  }
}
