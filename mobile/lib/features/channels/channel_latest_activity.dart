part of 'channels_provider.dart';

/// Runs the activity signature checks off the UI isolate. Tests run them in
/// place, since fake time never delivers another isolate's result.
@visibleForTesting
Future<Set<String>> Function(Set<String> Function() checks)
debugRunActivityChecks = (checks) => Isolate.run(checks);

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
  // since it began. Only the newest read can still report, so a read that
  // begins drops older ones, such as a superseded refresh.
  final Map<int, Map<String, ({int at, String id})>> _arrivalsByRead = {};
  final Map<int, int> _readSentAt = {};
  // Per latest-message read in flight, the channels whose latest post was
  // deleted since it began. Its answer may still hold that post, so it leaves
  // them unknown for the next list read to ask again.
  final Map<int, Set<String>> _forgottenByRead = {};
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

  /// The id of the channel's latest post, if one is known.
  String? latestId(String channelId) => _latest[channelId]?.id;

  /// Whether a latest read or live message covered [channelId] since its
  /// latest post was last deleted, if ever. A list read that keeps the list's
  /// times still reads the channels without one.
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

  /// Forgets the channels whose activity is one of [eventIds], so a deleted
  /// latest post stops counting: each reads as unknown until a later list read
  /// re-reads it. Event ids are unique, so a deletion without an `h` tag finds
  /// its channel too. Returns whether any channel was forgotten.
  bool forgetHeads(Set<String> eventIds) {
    final channelIds = [
      for (final MapEntry(key: id, value: head) in _latest.entries)
        if (eventIds.contains(head.id)) id,
    ];
    for (final channelId in channelIds) {
      _latest.remove(channelId);
      _known.remove(channelId);
      for (final MapEntry(key: read, value: arrivals)
          in _arrivalsByRead.entries) {
        arrivals.remove(channelId);
        _forgottenByRead[read]!.add(channelId);
      }
    }
    return channelIds.isNotEmpty;
  }

  /// Starts a latest-message read; pass the result to [noteRead].
  int beginRead() {
    final read = _nextRead++;
    _arrivalsByRead
      ..clear()
      ..[read] = {};
    _readSentAt
      ..clear()
      ..[read] = currentUnixSeconds();
    _forgottenByRead
      ..clear()
      ..[read] = {};
    return read;
  }

  /// Records a latest-message read of [requested]. A channel it covered is
  /// known even when it has no messages, and its activity becomes the read's
  /// newest authentic message or a newer arrival since the read began, so a
  /// deleted latest post stops counting. A failed batch or channel, or one
  /// whose messages all fail verification, stays unknown, as does one whose
  /// latest post was deleted while the read was in flight, including the
  /// read's own head when it is among [deletedIds]. [authenticIds] is the
  /// verified [authenticHeadIds] of [events].
  void noteRead(
    int read,
    Iterable<String> requested,
    Set<String> failedIds,
    List<NostrEvent>? events,
    Set<String> authenticIds, {
    Set<String> deletedIds = const {},
  }) {
    if (read < _firstRead) return;
    final arrivals = _arrivalsByRead.remove(read);
    final sentAt = _readSentAt.remove(read);
    final forgotten = _forgottenByRead.remove(read) ?? const {};
    if (events == null) return;
    final byChannel = <String, List<NostrEvent>>{};
    for (final event in events) {
      if (event.channelId case final channelId?) {
        byChannel.putIfAbsent(channelId, () => []).add(event);
      }
    }
    final newest = <String, ({int at, String id})>{};
    final invalid = <String>{};
    final deletedHeads = <String>{};
    for (final MapEntry(key: channelId, value: posts) in byChannel.entries) {
      posts.sort((a, b) => b.createdAt.compareTo(a.createdAt));
      final head = posts
          .where((post) => authenticIds.contains(post.id))
          .firstOrNull;
      if (head == null) {
        invalid.add(channelId);
      } else if (deletedIds.contains(head.id)) {
        deletedHeads.add(channelId);
      } else {
        newest[channelId] = (at: _clamp(head.createdAt), id: head.id);
      }
    }
    final covered = <String>{};
    for (final channelId in requested) {
      if (failedIds.contains(channelId) ||
          invalid.contains(channelId) ||
          forgotten.contains(channelId)) {
        continue;
      }
      // Its older posts went unchecked, so it is unknown unless a post
      // arrived since the read began.
      if (deletedHeads.contains(channelId)) {
        if (arrivals?[channelId] == null) {
          _latest.remove(channelId);
          _known.remove(channelId);
        }
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
      if (forgotten.contains(channelId)) continue;
      if (arrivals == null || !covered.contains(channelId)) {
        _raise(channelId, head);
      }
    }
  }

  void noteLive(NostrEvent event) {
    final channelId = event.channelId;
    if (channelId == null) return;
    if (note(channelId, event, _isAuthentic)) _known.add(channelId);
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
  /// advanced the channel's activity. Only a message [isAuthentic] accepts
  /// counts, and it is asked only about one that would change the activity.
  bool note(
    String channelId,
    NostrEvent event,
    bool Function(NostrEvent) isAuthentic,
  ) {
    final at = _clamp(event.createdAt);
    final current = _latest[channelId]?.at;
    final raises = current == null || at > current;
    final arrives = _arrivalsByRead.values.any(
      (arrivals) => (arrivals[channelId]?.at ?? -1) < at,
    );
    if (!raises && !arrives) return false;
    if (!isAuthentic(event)) return false;
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

  /// Checks each channel's newest message among [events], ignoring
  /// [skippedIds], off the UI isolate. Gives the ids that verified and the
  /// channels whose newest message did not, which stay unknown rather than
  /// fall back to an older, unchecked message.
  static Future<({Set<String> authentic, Set<String> rejected})>
  authenticHeadIds(
    List<NostrEvent> events, [
    Set<String> skippedIds = const {},
  ]) async {
    final heads = <String, NostrEvent>{};
    for (final event in events) {
      if (skippedIds.contains(event.id) ||
          !EventKind.channelMessageEventKinds.contains(event.kind)) {
        continue;
      }
      if (event.channelId case final channelId?) {
        final head = heads[channelId];
        if (head == null || event.createdAt > head.createdAt) {
          heads[channelId] = event;
        }
      }
    }
    if (heads.isEmpty) {
      return (authentic: const <String>{}, rejected: const <String>{});
    }
    final authentic = await _checkOffIsolate(heads.values.toList());
    return (
      authentic: authentic,
      rejected: {
        for (final MapEntry(key: channelId, value: head) in heads.entries)
          if (!authentic.contains(head.id)) channelId,
      },
    );
  }

  // Its own function, so the isolate's closure captures only [heads].
  static Future<Set<String>> _checkOffIsolate(List<NostrEvent> heads) =>
      debugRunActivityChecks(
        () => {
          for (final head in heads)
            if (_isAuthentic(head)) head.id,
        },
      );

  // The activity time decides which rows the filter shows, so it comes only
  // from messages with a valid id and signature, as the old per-channel lookup
  // required. A live message is checked here only when it would change the
  // activity; reads check their candidates with [authenticHeadIds].
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
    _forgottenByRead.clear();
    _coveredAt.clear();
    _firstRead = _nextRead;
  }
}

extension _ChannelLatestMessages on ChannelsNotifier {
  Set<String> _deletionTargets(NostrEvent deletion) => {
    for (final tag in deletion.tags)
      if (tag.length >= 2 && tag[0] == 'e') tag[1],
  };

  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request, with [_ChannelActivity.authenticHeadIds] of the result.
  /// The relay preserves NIP-01 per-filter limits while executing the filters
  /// with bounded concurrency, avoiding an unbounded burst of websocket REQs
  /// on communities with many channels.
  Future<({List<NostrEvent> events, Set<String> authentic})?>
  _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<ChannelData> channels, {
    Set<String>? failedChannelIds,
  }) async {
    if (channels.isEmpty) {
      return (events: const <NostrEvent>[], authentic: const <String>{});
    }

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

    final events = await _fetchChannelHistoryBatch(
      session,
      filters,
      operation: 'latest-message query',
      failedChannelIds: failedChannelIds,
    );
    if (events == null) return null;
    return (
      events: events,
      authentic: (await _ChannelActivity.authenticHeadIds(events)).authentic,
    );
  }
}
