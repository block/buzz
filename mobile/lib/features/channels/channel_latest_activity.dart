part of 'channels_provider.dart';

/// When anyone last posted in each channel, self-authored and muted messages
/// included, as the list's reads and live delivery saw it. Unlike
/// [Channel.lastMessageAt], it does not affect unread or sort state.
class _ChannelActivity {
  final Map<String, ({int at, String id})> _latest = {};
  final Set<String> _known = {};
  // Channels whose last unread catch-up failed: a post may have arrived while
  // disconnected that no read has seen yet.
  final Set<String> _catchUpFailed = {};
  // Per latest-message read in flight, the newest arrival in each channel
  // since it began. A read that never reports, such as a superseded refresh,
  // is dropped once newer reads begin.
  final Map<int, Map<String, ({int at, String id})>> _arrivalsByRead = {};
  final Map<int, int> _readSentAt = {};
  int _nextRead = 0;
  // Reads begun before the last [clear] belong to another scope.
  int _firstRead = 0;
  // Per channel, when the last read that covered it (a successful latest read
  // or unread catch-up) was sent. Unlike the unread cursor, only [clear]
  // resets it, so the catch-up can always ask for posts since then.
  final Map<String, int> _coveredAt = {};
  // Channels an Active section is showing, with how many listeners each.
  final Map<String, int> _watched = {};

  /// When the last read that covered [channelId] was sent, if any.
  int? coveredAt(String channelId) => _coveredAt[channelId];

  /// The channel's activity time, for deciding whether a read can change it.
  int? latestAt(String channelId) => _latest[channelId]?.at;

  /// Whether a latest read or live message ever covered [channelId]. A list
  /// read that keeps the list's times still reads the channels without one.
  bool hasBaseline(String channelId) => _known.contains(channelId);

  /// Whether an Active section shows [channelId], so reads that only serve
  /// the filter are worth sending for it.
  bool isWatched(String channelId) => _watched.containsKey(channelId);

  /// Registers an Active section's interest in [channelId]; call the result
  /// when it ends.
  void Function() watch(String channelId) {
    _watched.update(channelId, (n) => n + 1, ifAbsent: () => 1);
    var done = false;
    return () {
      if (done) return;
      done = true;
      final n = _watched[channelId]! - 1;
      n == 0 ? _watched.remove(channelId) : _watched[channelId] = n;
    };
  }

  /// `known` is false until a read or live message covered the channel, and
  /// while its last unread catch-up failed.
  ({bool known, int? at}) lookup(String channelId) => (
    known: _known.contains(channelId) && !_catchUpFailed.contains(channelId),
    at: _latest[channelId]?.at,
  );

  /// The channels whose activity is one of [eventIds]. Event ids are unique,
  /// so a deletion without an `h` tag finds its channel too.
  Set<String> headsAmong(Set<String> eventIds) => {
    for (final MapEntry(key: id, value: head) in _latest.entries)
      if (eventIds.contains(head.id)) id,
  };

  /// Whether [read] began since the last [clear].
  bool isCurrent(int read) => read >= _firstRead;

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

  /// Records a latest-message read of [requested]. A channel it covered is
  /// known even when it has no messages, and its activity becomes the read's
  /// newest authentic message or a newer arrival since the read began, so a
  /// deleted latest post stops counting. A failed batch or channel, or one
  /// whose messages all fail verification, stays unknown.
  void noteRead(
    int read,
    Iterable<String> requested,
    Set<String> failedIds,
    List<NostrEvent>? events,
  ) {
    if (read < _firstRead) return;
    final arrivals = _arrivalsByRead.remove(read);
    final sentAt = _readSentAt.remove(read);
    if (events == null) return;
    final byChannel = <String, List<NostrEvent>>{};
    for (final event in events) {
      if (event.channelId case final channelId?) {
        byChannel.putIfAbsent(channelId, () => []).add(event);
      }
    }
    final newest = <String, ({int at, String id})>{};
    final invalid = <String>{};
    for (final MapEntry(key: channelId, value: posts) in byChannel.entries) {
      posts.sort((a, b) => b.createdAt.compareTo(a.createdAt));
      final head = posts.where(_isAuthentic).firstOrNull;
      if (head == null) {
        invalid.add(channelId);
      } else {
        newest[channelId] = (at: _clamp(head.createdAt), id: head.id);
      }
    }
    final covered = <String>{};
    for (final channelId in requested) {
      if (failedIds.contains(channelId) || invalid.contains(channelId)) {
        continue;
      }
      covered.add(channelId);
      _known.add(channelId);
      _catchUpFailed.remove(channelId);
      if (sentAt != null) _coveredAt[channelId] = sentAt;
      // Without its arrivals (dropped), only raise, as a live event would.
      if (arrivals == null) continue;
      final head = newest[channelId];
      final arrived = arrivals[channelId];
      final latest = head == null || arrived == null
          ? head ?? arrived
          : (arrived.at > head.at ? arrived : head);
      if (latest == null) {
        _latest.remove(channelId);
      } else {
        _latest[channelId] = latest;
      }
    }
    for (final MapEntry(key: channelId, value: head) in newest.entries) {
      if (arrivals == null || !covered.contains(channelId)) {
        _raise(channelId, head);
      }
    }
  }

  void noteLive(NostrEvent event) {
    final channelId = event.channelId;
    if (channelId == null) return;
    _known.add(channelId);
    note(channelId, event);
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
  /// advanced the channel's activity. Only an authentic message counts.
  bool note(String channelId, NostrEvent event) {
    final at = _clamp(event.createdAt);
    final current = _latest[channelId]?.at;
    final raises = current == null || at > current;
    final arrives = _arrivalsByRead.values.any(
      (arrivals) => (arrivals[channelId]?.at ?? -1) < at,
    );
    if (!raises && !arrives) return false;
    if (!_isAuthentic(event)) return false;
    final head = (at: at, id: event.id);
    for (final arrivals in _arrivalsByRead.values) {
      arrivals.update(
        channelId,
        (v) => v.at >= at ? v : head,
        ifAbsent: () => head,
      );
    }
    return _raise(channelId, head);
  }

  bool _raise(String channelId, ({int at, String id}) head) {
    final current = _latest[channelId];
    if (current != null && head.at <= current.at) return false;
    _latest[channelId] = head;
    return true;
  }

  // The activity time decides which rows the filter shows, so it comes only
  // from messages with a valid id and signature, as the old per-channel lookup
  // required. Only a message that would change the activity is checked, so a
  // read checks about one per channel, natively, as read-state sync does.
  static bool _isAuthentic(NostrEvent event) {
    try {
      verifiedNostrEventFromJson(jsonEncode(event.toJson()));
      return true;
    } catch (_) {
      return false;
    }
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
    _firstRead = _nextRead;
  }
}

extension _ChannelLatestMessages on ChannelsNotifier {
  Set<String> _deletionTargets(NostrEvent deletion) => {
    for (final tag in deletion.tags)
      if (tag.length >= 2 && tag[0] == 'e') tag[1],
  };

  /// Re-reads the newest post of each channel whose activity was one of
  /// [deletedIds], so a deleted latest post stops counting. Activity can only
  /// rise otherwise. One `limit: 1` filter per such channel; a failed read
  /// leaves the channel unknown, which the Active filter shows.
  void _reconcileDeletedActivity(Set<String> deletedIds) {
    if (deletedIds.isEmpty) return;
    final channelIds = _activity.headsAmong(deletedIds);
    if (channelIds.isNotEmpty) unawaited(_rereadActivity(channelIds));
  }

  Future<void> _rereadActivity(Set<String> channelIds) async {
    final read = _activity.beginRead();
    final failed = <String>{};
    List<NostrEvent>? events;
    try {
      events = await _fetchChannelHistoryBatch(
        _lifecycleRef.read(relaySessionProvider.notifier),
        [
          for (final channelId in channelIds)
            NostrFilter(
              kinds: EventKind.channelMessageEventKinds,
              tags: {
                '#h': [channelId],
              },
              limit: 1,
            ),
        ],
        operation: 'activity re-read',
        failedChannelIds: failed,
        optional: true,
      );
    } catch (error) {
      debugPrint('[ChannelsNotifier] activity re-read failed: $error');
    }
    if (!_lifecycleRef.mounted || !_activity.isCurrent(read)) return;
    final surviving = events
        ?.where(
          (event) =>
              !_unscopedUnreadDeletedIds.contains(event.id) &&
              _unreadDeletedIdsByChannel[event.channelId]?.contains(event.id) !=
                  true,
        )
        .toList();
    _activity.noteRead(read, channelIds, failed, surviving);
    if (surviving == null) _activity.noteCatchUp(channelIds, channelIds);
    _publishUnreadCatchUp(const {});
  }

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
