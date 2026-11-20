part of 'channels_provider.dart';

/// When anyone last posted in each channel, self-authored and muted messages
/// included, as the list's reads and live delivery saw it. Unlike
/// [Channel.lastMessageAt], it does not affect unread or sort state.
class _ChannelActivity {
  final Map<String, int> _latest = {};
  final Set<String> _known = {};
  // Channels whose last unread catch-up failed: a post may have arrived while
  // disconnected that no read has seen yet.
  final Set<String> _catchUpFailed = {};
  // Per latest-message read in flight, the newest arrival in each channel
  // since it began. A read that never reports, such as a superseded refresh,
  // is dropped once newer reads begin.
  final Map<int, Map<String, int>> _arrivalsByRead = {};
  final Map<int, int> _readSentAt = {};
  int _nextRead = 0;
  // Per channel, when the last read that covered it (a successful latest read
  // or unread catch-up) was sent. Unlike the unread cursor, only [clear]
  // resets it, so the catch-up can always ask for posts since then.
  final Map<String, int> _coveredAt = {};

  /// When the last read that covered [channelId] was sent, if any.
  int? coveredAt(String channelId) => _coveredAt[channelId];

  /// `known` is false until a read or live message covered the channel, and
  /// while its last unread catch-up failed.
  ({bool known, int? at}) lookup(String channelId) => (
    known: _known.contains(channelId) && !_catchUpFailed.contains(channelId),
    at: _latest[channelId],
  );

  /// Starts a latest-message read; pass the result to [noteRead].
  int beginRead() {
    final read = _nextRead++;
    _arrivalsByRead[read] = {};
    _readSentAt[read] = currentUnixSeconds();
    if (_arrivalsByRead.length > 8) {
      final oldest = _arrivalsByRead.keys.first;
      _arrivalsByRead.remove(oldest);
      _readSentAt.remove(oldest);
    }
    return read;
  }

  /// Records a latest-message read. A channel it covered is known even when it
  /// has no messages, and its activity becomes the read's newest message or a
  /// newer arrival since the read began, so a deleted latest post stops
  /// counting. A failed batch or channel stays unknown.
  void noteRead(
    int read,
    List<ChannelData> requested,
    Set<String> failedIds,
    List<NostrEvent>? events,
  ) {
    final arrivals = _arrivalsByRead.remove(read);
    final sentAt = _readSentAt.remove(read);
    if (events == null) return;
    final newest = <String, int>{};
    for (final event in events) {
      if (event.channelId case final channelId?) {
        final at = _clamp(event.createdAt);
        newest.update(channelId, (v) => max(v, at), ifAbsent: () => at);
      }
    }
    final covered = <String>{};
    for (final channel in requested) {
      if (failedIds.contains(channel.id)) continue;
      covered.add(channel.id);
      _known.add(channel.id);
      if (sentAt != null) _coveredAt[channel.id] = sentAt;
      // Without its arrivals (dropped), only raise, as a live event would.
      if (arrivals == null) continue;
      final read = newest[channel.id];
      final arrived = arrivals[channel.id];
      final at = read == null || arrived == null
          ? read ?? arrived
          : max(read, arrived);
      if (at == null) {
        _latest.remove(channel.id);
      } else {
        _latest[channel.id] = at;
      }
    }
    for (final MapEntry(key: channelId, value: at) in newest.entries) {
      if (arrivals == null || !covered.contains(channelId)) {
        _raise(channelId, at);
      }
    }
  }

  void noteLive(String channelId, int createdAt) {
    _known.add(channelId);
    note(channelId, createdAt);
  }

  /// Records an unread catch-up's outcome per channel. Returns whether that
  /// changed which channels are known.
  /// [sentAt] is when a catch-up that answered was sent.
  bool noteCatchUp(
    Iterable<String> channelIds,
    Set<String> failedIds, {
    int? sentAt,
  }) {
    var changed = false;
    for (final channelId in channelIds) {
      if (failedIds.contains(channelId)) {
        changed |= _catchUpFailed.add(channelId);
        continue;
      }
      changed |= _catchUpFailed.remove(channelId);
      if (sentAt != null) _coveredAt[channelId] = sentAt;
    }
    return changed;
  }

  /// Records a message that arrived live or by catch-up. Returns whether it
  /// advanced the channel's activity.
  bool note(String channelId, int createdAt) {
    final at = _clamp(createdAt);
    for (final arrivals in _arrivalsByRead.values) {
      arrivals.update(channelId, (v) => max(v, at), ifAbsent: () => at);
    }
    return _raise(channelId, at);
  }

  bool _raise(String channelId, int at) {
    final current = _latest[channelId];
    if (current != null && at <= current) return false;
    _latest[channelId] = at;
    return true;
  }

  // The relay accepts posts up to 900s ahead of its clock. Count one as now
  // rather than drop it: it is still the channel's newest activity.
  static int _clamp(int createdAt) =>
      min(createdAt, DateTime.now().millisecondsSinceEpoch ~/ 1000);

  void clear() {
    _latest.clear();
    _known.clear();
    _catchUpFailed.clear();
    _arrivalsByRead.clear();
    _readSentAt.clear();
    _coveredAt.clear();
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
