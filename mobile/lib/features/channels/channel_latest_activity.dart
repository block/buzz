part of 'channels_provider.dart';

/// When anyone last posted in each channel, self-authored and muted messages
/// included, as the list's reads and live delivery saw it. Unlike
/// [Channel.lastMessageAt], it does not affect unread or sort state.
class _ChannelActivity {
  final Map<String, int> _latest = {};
  final Set<String> _known = {};

  /// `known` is false until a read or live message covered the channel.
  ({bool known, int? at}) lookup(String channelId) =>
      (known: _known.contains(channelId), at: _latest[channelId]);

  /// Records a latest-message read. A channel it covered is known even when it
  /// has no messages; a failed batch or channel stays unknown.
  void noteRead(
    List<ChannelData> requested,
    Set<String> failedIds,
    List<NostrEvent>? events,
  ) {
    if (events == null) return;
    for (final channel in requested) {
      if (!failedIds.contains(channel.id)) _known.add(channel.id);
    }
    for (final event in events) {
      if (event.channelId case final channelId?) {
        note(channelId, event.createdAt);
      }
    }
  }

  void noteLive(String channelId, int createdAt) {
    _known.add(channelId);
    note(channelId, createdAt);
  }

  /// Returns whether [createdAt] advanced the channel's activity.
  bool note(String channelId, int createdAt) {
    // A future-dated message would keep the channel active indefinitely.
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    if (createdAt > now + 300) return false;
    final current = _latest[channelId];
    if (current != null && createdAt <= current) return false;
    _latest[channelId] = createdAt;
    return true;
  }

  void clear() {
    _latest.clear();
    _known.clear();
  }
}

extension _ChannelLatestMessages on ChannelsNotifier {
  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request. The relay preserves NIP-01 per-filter limits while
  /// executing the filters with bounded concurrency, avoiding an unbounded
  /// burst of websocket REQs on communities with many channels.
  Future<List<NostrEvent>?> _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<ChannelData> channels, {
    Set<String>? failedChannelIds,
  }) async {
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
      failedChannelIds: failedChannelIds,
    );
  }
}
