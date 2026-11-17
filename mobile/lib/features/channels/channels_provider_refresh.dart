part of 'channels_provider.dart';

/// Explicit list reads and the shared list installation path.
extension ChannelsNotifierRefresh on ChannelsNotifier {
  Future<List<Channel>> _fetch({
    bool subscribeLive = false,
    bool fetchLastMessage = true,
    bool fetchDirectory = false,
    bool strong = false,
  }) async {
    final channels = await _fetchChannels(
      subscribeLive: subscribeLive,
      fetchLastMessage: fetchLastMessage,
      fetchDirectory: fetchDirectory,
      strong: strong,
    );
    _hasLoaded = true;
    return channels;
  }
}

Future<List<NostrEvent>> _fetchMemberMetadata(
  RelaySessionNotifier session,
  List<String> ids, {
  required bool strong,
}) => strong
    ? _fetchPaginatedChannelEvents(
        session,
        kind: 39000,
        tags: {'#d': ids},
        operation: 'Writer channel metadata',
        strong: true,
      )
    : session.fetchHistory(NostrFilters.channelMetadata(ids));
