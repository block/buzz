part of '../channels_page.dart';

SectionChatContent _watchSectionChats(
  WidgetRef ref,
  String key,
  String title,
  bool messages,
) {
  final all = ref.watch(channelsProvider).value ?? const <Channel>[];
  final legacy =
      ref.watch(meGroupsProvider).value?.channels ?? const <String>[];
  final viewer = ref.watch(myPubkeyProvider);
  final layout = ref.watch(sidebarLayoutProvider);
  final stars = ref.watch(channelStarsProvider).store.channels;
  final mutes = ref.watch(channelMutesProvider).store.channels;
  bool solo(Channel c) => c.isSoloChat(legacy);
  final members = all.where((c) => c.isMember && !c.isArchived);
  final rows = members.where((c) {
    if ((!c.isStream && !c.isDm) || (!messages && solo(c))) return false;
    return layout.sectionKeyFor(
          c.id,
          starred: stars[c.id]?.starred == true,
          home: solo(c)
              ? 'solo'
              : c.isDm
              ? 'dms'
              : 'channels',
        ) ==
        key;
  }).toList();
  final sort =
      (messages ? ref.watch(messagesDisplayProvider).sort : null) ??
      ref.watch(channelSortProvider).sortModeFor(sidebarGroupKey(key));
  final ordered = key == 'dms' && sort == ChannelSortMode.alpha
      ? sortDmChannelsByDisplayLabel(
          rows,
          currentPubkey: viewer,
          names: ref.watch(identityNameSourcesProvider),
        )
      : sortChannelsForList(rows, sort);
  final read = ref.watch(readStateProvider);
  final highlights = computeChannelListHighlights(
    channels: rows.where((c) => !solo(c)).toList(),
    readState: read,
    channelsNotifier: ref.watch(channelsProvider.notifier),
  );
  final section = key.startsWith('group:')
      ? layout.section(key.substring(6))
      : null;
  return (
    title: section?.name ?? title,
    channels: ordered,
    buildRow: (c, [onTap]) => solo(c)
        ? SoloChatTile(key: ValueKey(c.id), channel: c, onTap: onTap)
        : _ChannelTile(
            key: ValueKey(c.id),
            channel: c,
            isUnread: highlights.unread.contains(c.id),
            boldUnread: highlights.rowUnread.contains(c.id),
            hasAttention: highlights.attention.contains(c.id),
            isMuted: mutes[c.id]?.muted == true,
            currentPubkey: viewer,
            sectionId: section?.teamId,
            onTap: onTap!,
          ),
    buildDetail: (c) => solo(c)
        ? ChannelDetailPage(
            channel: c,
            personalChat: c.isPersonalChat(viewer, legacy),
          )
        : ChannelDetailPage(
            channel: c,
            placement: ChannelRoutePlacement.ordinary,
          ),
  );
}
