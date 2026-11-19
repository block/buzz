part of 'channels_provider.dart';

/// The channel with this id from the list, else as it was opened ahead of the
/// list (see [ChannelsNotifier.rememberOpened]).
final channelByIdProvider = Provider.autoDispose.family<Channel?, String>(
  (ref, channelId) =>
      ref.watch(
        channelsProvider.select(
          (channels) => channels.value
              ?.where((channel) => channel.id == channelId)
              .firstOrNull,
        ),
      ) ??
      ref.watch(_openedAheadProvider.select((opened) => opened[channelId])),
);

/// Whether this channel can start or continue threads. A Solo chat's agents
/// answer at its top level, so it keeps existing threads readable but starts
/// none and takes no thread replies, as desktop does. A channel not loaded yet
/// keeps the default thread behavior.
final channelStartsThreadsProvider = Provider.autoDispose.family<bool, String>(
  (ref, channelId) => !(ref.watch(
    channelByIdProvider(
      channelId,
    ).select((channel) => channel?.isSolo ?? false),
  )),
);

/// Channels opened from their own metadata before the list had them, as a
/// fast DM open does. Scoped to one relay and identity. Each is dropped once
/// the list has it, so a DM later hidden or left is not found through it.
final _openedAheadProvider =
    NotifierProvider<_OpenedAheadNotifier, Map<String, Channel>>(
      _OpenedAheadNotifier.new,
    );

class _OpenedAheadNotifier extends Notifier<Map<String, Channel>> {
  @override
  Map<String, Channel> build() {
    ref.watch(relayConfigProvider.select((config) => config.baseUrl));
    ref.watch(myPubkeyProvider);
    return const {};
  }

  void remember(Channel channel) => state = {...state, channel.id: channel};

  void dropListed(List<Channel> listed) {
    if (state.isEmpty) return;
    final ids = {for (final channel in listed) channel.id};
    if (!state.keys.any(ids.contains)) return;
    state = {
      for (final entry in state.entries)
        if (!ids.contains(entry.key)) entry.key: entry.value,
    };
  }
}
