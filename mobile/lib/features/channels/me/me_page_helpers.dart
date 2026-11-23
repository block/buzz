part of 'me_page.dart';

class _MeRetry extends StatelessWidget {
  final VoidCallback onRetry;
  const _MeRetry({required this.onRetry});
  @override
  Widget build(BuildContext context) => Center(
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Text('Couldn’t load your chats and projects.'),
        TextButton(onPressed: onRetry, child: const Text('Try again')),
      ],
    ),
  );
}

/// Opens the list-row action popover and clears the matching pane when requested.
Future<bool?> openConversationListDetails(
  BuildContext context,
  Channel channel,
  String? viewer, {
  required bool personal,
  bool isUnread = false,
  String? sectionId,
  ConversationMenuSession? nativeSession,
}) async {
  final pane = ConversationPaneScope.maybeOf(context);
  final result = await showConversationPopover(
    anchorContext: context,
    channel: channel,
    viewer: viewer,
    personal: personal,
    isUnread: isUnread,
    sectionId: sectionId,
    nativeSession: nativeSession,
  );
  if (result == true && context.mounted && pane?.selectedId == channel.id) {
    pane!.clear();
  }
  return result;
}

/// A Solo chat row. It opens as a personal chat in whichever sidebar
/// section the chat is filed.
class SoloChatTile extends ConsumerWidget {
  const SoloChatTile({super.key, required this.channel, this.onTap});
  final VoidCallback? onTap;
  final Channel channel;

  static void showActions(
    BuildContext context,
    Channel channel,
    String? viewer,
  ) => showChannelDetailsSheet(
    context: context,
    channel: channel,
    currentPubkey: viewer,
    personalChat: true,
  );

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final viewer = ref.watch(myPubkeyProvider);
    final legacyChannels = ref.watch(
      meGroupsProvider.select((g) => g.value?.channels ?? const <String>[]),
    );
    final isUnread = ref.watch(
      meUnreadChannelIdsProvider.select((ids) => ids.contains(channel.id)),
    );
    final pane = ConversationPaneScope.maybeOf(context);
    final selected = pane?.selectedId == channel.id;
    return IosConversationMenuRow(
      onMenu: (session) => openConversationListDetails(
        context,
        channel,
        viewer,
        personal: channel.isPersonalChat(viewer, legacyChannels),
        isUnread: isUnread,
        nativeSession: session,
      ),
      child: markMessagesRow(
        context,
        Stack(
          children: [
            if (selected)
              Positioned.fill(
                left: MessagesSectionOptions.leadingInset(context) - Grid.xxs,
                right: Grid.gutter - Grid.xxs,
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: context.colors.primaryContainer,
                    borderRadius: BorderRadius.circular(Radii.container),
                  ),
                ),
              ),
            Semantics(
              selected: selected,
              child: _MeChatRow(
                channelId: channel.id,
                name: channel.name,
                isUnread: isUnread,
                onLongPress: () => openConversationListDetails(
                  context,
                  channel,
                  viewer,
                  personal: channel.isPersonalChat(viewer, legacyChannels),
                  isUnread: isUnread,
                ),
                onTap:
                    onTap ??
                    () => openConversationPane(
                      context,
                      channel.id,
                      (_) => ChannelDetailPage(
                        channel: channel,
                        personalChat: channel.isPersonalChat(
                          viewer,
                          legacyChannels,
                        ),
                      ),
                    ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _MeChatRow extends ConsumerWidget {
  const _MeChatRow({
    required this.channelId,
    required this.name,
    required this.onTap,
    required this.onLongPress,
    required this.isUnread,
  });
  final String channelId;
  final String name;
  final bool isUnread;
  final VoidCallback onTap;
  final VoidCallback onLongPress;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final color = channelNavigationForeground(context, unread: isUnread);
    final working = ref.watch(meWorkingAgentsProvider(channelId));
    final status = working.isEmpty ? '' : ', ${working.join(', ')} working';
    return Semantics(
      label: '${isUnread ? '$name, unread' : name}$status',
      onTap: onTap,
      onLongPress: onLongPress,
      excludeSemantics: true,
      button: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.container),
        onTap: onTap,
        onLongPress: usesIosConversationMenu ? null : onLongPress,
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            MessagesSectionOptions.leadingInset(context),
            Grid.inventoryRowInset,
            Grid.gutter + Grid.half,
            Grid.inventoryRowInset,
          ),
          child: Row(
            children: [
              SizedBox(
                width: Grid.inventoryIconSize,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(
                    BuzzIcons.messageCircle,
                    size: Grid.inventoryIconSize,
                    color: navigationRowIconForeground(context),
                  ),
                ),
              ),
              const SizedBox(width: Grid.inventoryLabelGap),
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: navigationChatTitleTextStyle.copyWith(
                    color: color,
                    fontWeight: isUnread ? FontWeight.w600 : FontWeight.w500,
                  ),
                ),
              ),
              SizedBox(
                width: Grid.inventoryAccessoryWidth,
                child: Align(
                  alignment: Alignment.centerRight,
                  child: SizedBox(
                    width: 20,
                    child: Center(
                      child: working.isEmpty
                          ? null
                          : SoloWorkingDots(color: color),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// Reconnects replace the transport service while an editor may still be open.
// Retain the reviewed account/community boundary but use its current connection.
MeSync _currentMeSync(WidgetRef ref, MeSync reviewed) {
  final current = ref.read(meSyncProvider);
  if (current.community != reviewed.community ||
      current.pubkey != reviewed.pubkey) {
    throw StateError('Your account or community changed. Open Solo again.');
  }
  return current;
}

class _MeSectionEmpty extends StatelessWidget {
  const _MeSectionEmpty(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(
      horizontal: Grid.xxs * 2,
      vertical: Grid.half,
    ),
    child: Align(
      alignment: Alignment.centerLeft,
      child: Text(
        label,
        style: contentListBodyTextStyle.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    ),
  );
}
