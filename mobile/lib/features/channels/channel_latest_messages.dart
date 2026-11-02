part of 'channels_provider.dart';

/// Deletions asked for per request alongside late-started latest messages.
const _deletionPageLimit = 500;

/// The latest-message query behind each channel's `lastMessageAt`.
///
/// Lives in this part file because `channels_provider.dart` sits against the
/// repository-wide 1200-line file ceiling enforced by `just file-size-check`.
extension _LatestMessages on ChannelsNotifier {
  /// Fetches each channel's independent latest-message window in one HTTP
  /// bridge request. The relay preserves NIP-01 per-filter limits while
  /// executing the filters with bounded concurrency, avoiding an unbounded
  /// burst of websocket REQs on communities with many channels.
  ///
  /// A channel asked only for newer messages can't see its latest one being
  /// deleted, so the request asks for deletions too. A channel with one is
  /// asked again in full and removed from [sinceById].
  Future<List<NostrEvent>?> _fetchLastMessageEvents(
    RelaySessionNotifier session,
    List<ChannelData> channels,
    Map<String, int> sinceById,
    Set<String> failedChannelIds,
  ) async {
    if (channels.isEmpty) return const [];

    NostrFilter latest(ChannelData channel, {int? since}) => NostrFilter(
      kinds: EventKind.channelMessageEventKinds,
      tags: {
        '#h': [channel.id],
      },
      since: since,
      limit: channel.channelType == 'dm' ? 1 : 20,
    );
    final windowed = [
      for (final channel in channels)
        if (sinceById.containsKey(channel.id)) channel.id,
    ];
    final events = await _fetchChannelHistoryBatch(
      session,
      [
        for (final channel in channels)
          latest(channel, since: sinceById[channel.id]),
        for (var i = 0; i < windowed.length; i += kMaxExplicitChannelValues)
          NostrFilter(
            kinds: const [EventKind.deletion, EventKind.nip29DeleteEvent],
            tags: {
              '#h': windowed.sublist(
                i,
                min(i + kMaxExplicitChannelValues, windowed.length),
              ),
            },
            since: windowed.map((id) => sinceById[id]!).reduce(min),
            limit: _deletionPageLimit,
          ),
      ],
      operation: 'latest-message query',
      failedChannelIds: failedChannelIds,
    );
    if (events == null) return null;
    bool isDeletion(NostrEvent event) =>
        event.kind == EventKind.deletion ||
        event.kind == EventKind.nip29DeleteEvent;
    // A deletion matters only when it removes a channel's known latest
    // message. Reaction removals (kind 5 without `h`) and deletions of older
    // messages leave it in place.
    final channelByLatestId = {
      for (final channelId in windowed)
        ?_latestMessageById[channelId]?.id: channelId,
    };
    // A full page of deletions may have left out older ones, so every window
    // starts over rather than risk keeping a deleted latest message.
    final deletedIn = events.where(isDeletion).length >= _deletionPageLimit
        ? windowed.toSet()
        : {
            for (final event in events)
              if (isDeletion(event))
                for (final tag in event.tags)
                  if (tag.length >= 2 && tag[0] == 'e')
                    ?channelByLatestId[tag[1]],
          };
    final messages = [
      for (final event in events)
        if (!isDeletion(event) && !deletedIn.contains(event.channelId)) event,
    ];
    if (deletedIn.isEmpty) return messages;
    sinceById.removeWhere((id, _) => deletedIn.contains(id));
    _latestMessageById.removeWhere((id, _) => deletedIn.contains(id));
    final full = await _fetchChannelHistoryBatch(
      session,
      [
        for (final channel in channels)
          if (deletedIn.contains(channel.id)) latest(channel),
      ],
      operation: 'latest-message query',
      failedChannelIds: failedChannelIds,
    );
    return full == null ? null : [...messages, ...full];
  }

  /// Remembers [event] as [channelId]'s latest counted message if it is
  /// newer than the one known.
  void _noteLatestMessage(String channelId, NostrEvent event) {
    final known = _latestMessageById[channelId];
    if (known == null || event.createdAt >= known.createdAt) {
      _latestMessageById[channelId] = (
        id: event.id,
        createdAt: event.createdAt,
      );
    }
  }
}
